//! tinyPortMapper - 轻量级高性能端口转发工具
//!
//! Rust 重写版本

use tinyportmapper::{get_sock_error, info, log_bare, myexit};

use mio::net::{TcpListener, UdpSocket};
use std::env;
#[cfg(unix)]
use std::os::unix::io::FromRawFd;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tinyportmapper::config::{Config, FwdType, LISTEN_FD_BUF_SIZE, TIMER_INTERVAL_MS};
use tinyportmapper::event::EventLoop;
use tinyportmapper::fd_manager::FdManager;
use tinyportmapper::log::LogLevel;
use tinyportmapper::manager::{TcpConnectionManager, UdpSessionManager};
use tinyportmapper::types::Address;

use clap::Parser;

/// Windows WSA 初始化
#[cfg(windows)]
fn init_ws() {
    use std::os::windows::io::AsRawSocket;
    use winapi::um::winsock2::{WSACleanup, WSAStartup, MAKEWORD, WSADATA};

    let mut wsa_data: WSADATA = unsafe { std::mem::zeroed() };
    let w_version_requested = MAKEWORD(2, 2);

    let result = unsafe { WSAStartup(w_version_requested, &mut wsa_data) };
    if result != 0 {
        eprintln!("WSAStartup failed with error: {}", result);
        myexit(1);
    }

    // 确认 WinSock DLL 支持 2.2
    if wsa_data.wVersion.lo() != 2 || wsa_data.wVersion.hi() != 2 {
        eprintln!("Could not find a usable version of Winsock.dll");
        unsafe {
            WSACleanup();
        }
        myexit(1);
    }

    println!("The Winsock 2.2 dll was found okay");

    // 设置最大文件描述符数量
    let values = [
        0,
        100,
        200,
        300,
        500,
        800,
        1000,
        2000,
        3000,
        4000,
        usize::MAX,
    ];
    let mut succ = 0;
    for i in 1..values.len() {
        if unsafe { libc::_setmaxstdio(values[i] as libc::c_int) } == -1 {
            break;
        } else {
            succ = i;
        }
    }
    println!(", _setmaxstdio() was set to {}", values[succ]);
}

#[cfg(not(windows))]
fn init_ws() {
    // 非 Windows 平台不需要 WSA 初始化
}

fn print_help() {
    use tinyportmapper::build::{BUILD_DATE, BUILD_TIME, GIT_VERSION};
    use tinyportmapper::config::{
        DEFAULT_CONN_CLEAR_MIN, DEFAULT_CONN_CLEAR_RATIO, DEFAULT_MAX_CONNECTIONS,
        DEFAULT_TCP_TIMEOUT_MS, DEFAULT_UDP_TIMEOUT_MS,
    };

    println!();
    println!("tinyPortMapper - Rust Version");
    println!("==============================");
    println!(
        "git version: {}    build date: {} {}",
        GIT_VERSION, BUILD_DATE, BUILD_TIME
    );
    println!("repository: https://github.com/x1t/tinyPortMapper-rust");
    println!();
    println!("usage:");
    println!(
        "    ./this_program  -l <listen_ip>:<listen_port> -r <remote_ip>:<remote_port>  [options]"
    );
    println!();
    println!("main options:");
    println!("    -t                                    enable TCP forwarding/mapping");
    println!("    -u                                    enable UDP forwarding/mapping");
    println!();
    println!("other options:");
    println!("    --sock-buf            <number>        buf size for socket, >=10 and <=10240, unit: kbyte, default: 1024");
    println!(
        "    --log-level           <number>        0: never    1: fatal   2: error   3: warn "
    );
    println!(
        "                                          4: info (default)      5: debug   6: trace"
    );
    println!(
        "                                          or: fatal, error, warn, info, debug, trace"
    );
    println!("    --log-position                        enable file name, function name, line number in log");
    println!("    --disable-color                       disable log color");
    println!("    --enable-color                        enable log color, log color is enabled by default on most platforms");
    println!("    --log-file            <path>          write log to file");
    println!(
        "    -4                                    enable 4to6 translation mode (IPv4 to IPv6)"
    );
    println!(
        "    -6                                    enable 6to4 translation mode (IPv6 to IPv4)"
    );
    println!("    -e <interface>                        bind to specified interface");
    println!("    -d                                    enable UDP fragment forwarding");
    println!(
        "    --max-connections      <number>       max connections, default: {}",
        DEFAULT_MAX_CONNECTIONS
    );
    println!(
        "    --tcp-timeout          <number>       TCP connection timeout in seconds, default: {}",
        DEFAULT_TCP_TIMEOUT_MS / 1000
    );
    println!(
        "    --udp-timeout          <number>       UDP session timeout in seconds, default: {}",
        DEFAULT_UDP_TIMEOUT_MS / 1000
    );
    println!(
        "    --conn-clear-ratio     <number>       connection clear ratio, default: {}",
        DEFAULT_CONN_CLEAR_RATIO
    );
    println!(
        "    --conn-clear-min       <number>       min connections to clear each time, default: {}",
        DEFAULT_CONN_CLEAR_MIN
    );
    println!("    --disable-conn-clear                   disable automatic connection clearing");
    println!("    --run-test                            run unit tests");
    println!("    -h,--help                             print this help message");
    println!();
}

/// 解析日志级别，支持数字 (0-6) 或字符串
fn parse_log_level(s: &str) -> Result<LogLevel, String> {
    // 先尝试解析为数字
    if let Ok(num) = s.parse::<u8>() {
        LogLevel::from_u8(num).map_err(|e| e.to_string())
    } else {
        // 尝试解析为字符串
        match s.to_lowercase().as_str() {
            "never" => Ok(LogLevel::Never),
            "fatal" => Ok(LogLevel::Fatal),
            "error" => Ok(LogLevel::Error),
            "warn" => Ok(LogLevel::Warn),
            "info" => Ok(LogLevel::Info),
            "debug" => Ok(LogLevel::Debug),
            "trace" => Ok(LogLevel::Trace),
            _ => Err(format!(
                "invalid log_level: {}, must be 0-6 or fatal/error/warn/info/debug/trace",
                s
            )),
        }
    }
}

/// 验证缓冲区大小 (10-10240 KB)
fn validate_buffer_size(s: &str) -> Result<usize, String> {
    let value: usize = s.parse().map_err(|_| "buffer must be a number")?;
    if !(10..=10240).contains(&value) {
        return Err(format!(
            "sock-buf value must be between 10 and 10240 (kbyte), got {}",
            value
        ));
    }
    Ok(value)
}

/// 设置 socket 绑定到指定网络接口 (SO_BINDTODEVICE)
#[cfg(target_os = "linux")]
fn set_bind_to_device(fd: libc::c_int, interface: &str) -> Result<(), String> {
    let ifreq = {
        let mut ifreq: libc::ifreq = unsafe { std::mem::zeroed() };
        let interface_bytes = interface.as_bytes();
        let ifr_name_len = std::mem::size_of::<libc::c_char>() * libc::IFNAMSIZ;
        let len = std::cmp::min(interface_bytes.len(), ifr_name_len - 1);
        unsafe {
            // ifreq.ifr_name 是 *mut i8，需要正确转换
            let dest_ptr = ifreq.ifr_name.as_mut_ptr() as *mut libc::c_char;
            std::ptr::copy_nonoverlapping(
                interface_bytes.as_ptr() as *const libc::c_char,
                dest_ptr,
                len,
            );
        }
        ifreq
    };

    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            &ifreq as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::ifreq>() as libc::socklen_t,
        )
    };

    if ret < 0 {
        Err(format!(
            "failed to bind to interface {}: {}",
            interface,
            get_sock_error()
        ))
    } else {
        Ok(())
    }
}

/// 设置 socket 绑定到指定网络接口 (非 Linux 平台)
#[cfg(not(target_os = "linux"))]
fn set_bind_to_device(_fd: libc::c_int, _interface: &str) -> Result<(), String> {
    Err("SO_BINDTODEVICE is not supported on this platform".to_string())
}

#[derive(Parser, Debug)]
#[command(name = "tinyportmapper")]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    listen: String,

    #[arg(short, long)]
    remote: String,

    #[arg(short)]
    tcp: bool,

    #[arg(short)]
    udp: bool,

    #[arg(long = "sock-buf", default_value = "1024", value_parser = validate_buffer_size, alias = "buffer")]
    buffer: usize,

    #[arg(long, default_value = "info", value_parser = parse_log_level)]
    log_level: LogLevel,

    #[arg(long = "disable-color")]
    disable_color: bool,

    #[arg(long)]
    #[allow(dead_code)]
    enable_color: bool,

    #[arg(long)]
    log_position: bool,

    #[arg(long)]
    log_file: Option<String>,

    #[arg(short = '4')]
    mode_4to6: bool,

    #[arg(short = '6')]
    mode_6to4: bool,

    #[arg(short = 'e')]
    bind_interface: Option<String>,

    #[arg(short = 'd')]
    udp_fragment: bool,

    #[arg(long, default_value_t = tinyportmapper::config::DEFAULT_MAX_CONNECTIONS)]
    max_connections: usize,

    #[arg(long, default_value_t = tinyportmapper::config::DEFAULT_TCP_TIMEOUT_MS / 1000)]
    tcp_timeout: u64,

    #[arg(long, default_value_t = tinyportmapper::config::DEFAULT_UDP_TIMEOUT_MS / 1000)]
    udp_timeout: u64,

    #[arg(long, default_value_t = tinyportmapper::config::DEFAULT_CONN_CLEAR_RATIO)]
    conn_clear_ratio: u32,

    #[arg(long, default_value_t = tinyportmapper::config::DEFAULT_CONN_CLEAR_MIN)]
    conn_clear_min: u32,

    #[arg(long)]
    disable_conn_clear: bool,
}

fn main() {
    // Windows WSA 初始化 (与 C++ 版本 init_ws() 保持一致)
    init_ws();

    // 与 C++ 版本保持一致：将 stderr 重定向到 stdout
    // C++ 版本: dup2(1, 2); // redirect stderr to stdout
    #[cfg(unix)]
    unsafe {
        libc::dup2(1, 2);
    }

    // 收集原始参数用于颜色逻辑
    let raw_args: Vec<String> = std::env::args().collect();

    // 检查 --version 和 --help 参数（C++ 风格的早期检查）
    for arg in &raw_args {
        if arg == "--version" {
            println!("tinyPortMapper");
            println!(
                "git version: {}    build date: {} {}",
                tinyportmapper::build::GIT_VERSION,
                tinyportmapper::build::BUILD_DATE,
                tinyportmapper::build::BUILD_TIME
            );
            println!("repository: https://github.com/x1t/tinyPortMapper-rust");
            myexit(0);
        }
        if arg == "-h" || arg == "--help" {
            print_help();
            myexit(0);
        }
        // 处理单元测试请求（与 C++ 版本 unit_test() 对应）- 提前检查
        if arg == "--run-test" {
            tinyportmapper::unit_test();
            myexit(0);
        }
    }

    // 解析命令行参数
    let args = Args::parse();

    // 与 C++ 版本保持一致的参数处理逻辑：
    // 先遍历所有参数，检查 --enable-color 和 --disable-color
    // 后出现的参数覆盖先出现的
    let mut has_enable_color = false;
    let mut has_disable_color = false;

    for arg in &raw_args {
        if arg == "--enable-color" {
            has_enable_color = true;
            has_disable_color = false; // 重置 disable 状态
        } else if arg == "--disable-color" {
            has_disable_color = true;
            has_enable_color = false; // 重置 enable 状态
        }
    }

    let logger = tinyportmapper::log::Logger::global();
    logger.set_level(args.log_level);
    // 与 C++ 版本对齐的颜色逻辑：
    // --enable-color 强制启用颜色，--disable-color 强制禁用颜色
    // 后出现的参数覆盖先出现的
    let enable_color = if has_enable_color {
        true
    } else if has_disable_color {
        false
    } else {
        // 默认：终端支持颜色时启用
        atty::is(atty::Stream::Stdout)
    };
    logger.set_color(enable_color);
    logger.set_position(args.log_position);

    // 打开日志文件
    if let Some(ref log_file) = args.log_file {
        if let Err(e) = logger.open_log_file(log_file) {
            eprintln!("Warning: failed to open log file '{}': {}", log_file, e);
        } else {
            info!("Log file opened: {}", log_file);
        }
    }

    println!();
    println!("tinyPortMapper - Rust Version");
    println!(
        "version: {} (build: {} {})",
        env!("CARGO_PKG_VERSION"),
        tinyportmapper::build::BUILD_DATE,
        tinyportmapper::build::BUILD_TIME
    );
    println!("git version: {}", tinyportmapper::build::GIT_VERSION);
    println!("repository: https://github.com/x1t/tinyPortMapper-rust");
    println!();
    println!("==============================");
    println!();

    if args.listen.is_empty() || args.remote.is_empty() {
        eprintln!("Error: -l (listen) and -r (remote) are required");
        print_help();
        myexit(1);
    }

    if !args.tcp && !args.udp {
        eprintln!("Error: must specify -t (TCP) or -u (UDP) or both");
        print_help();
        myexit(1);
    }

    // 打印命令行参数（类似C++版本的 log_bare）
    let args_vec: Vec<String> = env::args().collect();
    info!("argc={}", args_vec.len());
    log_bare!("{}", args_vec.join(" "));
    log_bare!("\n");

    let listen_addr: Address = match Address::from_str(&args.listen) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("Error: invalid listen address '{}': {}", args.listen, e);
            myexit(1);
        }
    };

    let remote_addr: Address = match Address::from_str(&args.remote) {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("Error: invalid remote address '{}': {}", args.remote, e);
            myexit(1);
        }
    };

    info!("Starting tinyPortMapper...");
    info!("Listen: {}", listen_addr);
    info!("Remote: {}", remote_addr);
    info!("TCP: {}, UDP: {}", args.tcp, args.udp);
    info!("Buffer: {} KB", args.buffer);
    info!("Max connections: {}", args.max_connections);
    info!(
        "TCP timeout: {}s, UDP timeout: {}s",
        args.tcp_timeout, args.udp_timeout
    );

    // Determine address family for socket creation
    let addr_family = match listen_addr.get_type() {
        4 => libc::AF_INET,
        6 => libc::AF_INET6,
        _ => {
            eprintln!("Error: unsupported address type");
            myexit(1);
        }
    };

    // 确定转发类型
    let fwd_type = if args.mode_4to6 {
        FwdType::FwdType4to6
    } else if args.mode_6to4 {
        FwdType::FwdType6to4
    } else {
        FwdType::Normal
    };

    let config = Arc::new(Config {
        listen_addr: listen_addr.clone(),
        remote_addr: remote_addr.clone(),
        enable_tcp: args.tcp,
        enable_udp: args.udp,
        socket_buf_size: args.buffer * 1024,
        listen_fd_buf_size: LISTEN_FD_BUF_SIZE,
        log_level: args.log_level,
        log_position: args.log_position,
        disable_color: args.disable_color,
        max_connections: args.max_connections,
        tcp_timeout: Duration::from_secs(args.tcp_timeout),
        udp_timeout: Duration::from_secs(args.udp_timeout),
        conn_clear_ratio: args.conn_clear_ratio,
        conn_clear_min: args.conn_clear_min,
        disable_conn_clear: args.disable_conn_clear,
        timer_interval: TIMER_INTERVAL_MS,
        fwd_type,
        bind_interface: args.bind_interface.clone(),
        log_file: args.log_file.clone(),
        enable_udp_fragment: args.udp_fragment,
    });

    let fd_manager: Arc<FdManager> = FdManager::new();
    let tcp_manager: Arc<TcpConnectionManager> = Arc::new(TcpConnectionManager::new(
        config.tcp_timeout,
        config.conn_clear_ratio,
        config.conn_clear_min,
        config.disable_conn_clear,
    ));
    let udp_manager: Arc<UdpSessionManager> = Arc::new(UdpSessionManager::new(
        config.udp_timeout, // 修复：使用正确的 udp_timeout 而非 tcp_timeout
        config.conn_clear_ratio,
        config.conn_clear_min,
        config.disable_conn_clear,
    ));

    let mut event_loop: EventLoop = match EventLoop::new(
        config.clone(),
        Arc::clone(&fd_manager),
        Arc::clone(&tcp_manager),
        Arc::clone(&udp_manager),
    ) {
        Ok(el) => el,
        Err(e) => {
            eprintln!("Error: failed to create event loop: {}", e);
            myexit(1);
        }
    };

    let mut tcp_listener: Option<TcpListener> = None;
    let mut udp_socket: Option<UdpSocket> = None;

    if args.tcp {
        let sockaddr = listen_addr.to_sockaddr_storage();
        let sockaddr_len = listen_addr.get_len() as libc::socklen_t;
        let listener = unsafe {
            let fd = libc::socket(addr_family, libc::SOCK_STREAM, 0);
            if fd < 0 {
                eprintln!("Error: failed to create TCP socket");
                myexit(1);
            }

            let opt: libc::c_int = 1;
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_REUSEADDR,
                &opt as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
            // SO_REUSEPORT 支持多进程绑定同一端口
            #[cfg(target_os = "linux")]
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_REUSEPORT,
                &opt as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );

            let bufsize = (args.buffer * 1024) as libc::socklen_t;
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &bufsize as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::socklen_t>() as libc::socklen_t,
            );
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &bufsize as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::socklen_t>() as libc::socklen_t,
            );

            // 绑定到指定网络接口
            if let Some(ref interface) = args.bind_interface {
                if let Err(e) = set_bind_to_device(fd, interface) {
                    eprintln!("Warning: {}", e);
                }
            }

            libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);

            if libc::bind(
                fd,
                &sockaddr as *const _ as *const libc::sockaddr,
                sockaddr_len,
            ) < 0
            {
                eprintln!("Error: failed to bind TCP socket");
                myexit(1);
            }

            if libc::listen(fd, 512) < 0 {
                eprintln!("Error: failed to listen");
                myexit(1);
            }

            fd
        };

        tcp_listener = Some(unsafe { TcpListener::from_raw_fd(listener) });
        info!("TCP listening on {}", listen_addr);
    }

    if args.udp {
        let sockaddr = listen_addr.to_sockaddr_storage();
        let sockaddr_len = listen_addr.get_len() as libc::socklen_t;
        let socket = unsafe {
            let fd = libc::socket(addr_family, libc::SOCK_DGRAM, libc::IPPROTO_UDP);
            if fd < 0 {
                eprintln!("Error: failed to create UDP socket");
                myexit(1);
            }

            let opt: libc::c_int = 1;
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_REUSEADDR,
                &opt as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );
            // SO_REUSEPORT 支持多进程绑定同一端口
            #[cfg(target_os = "linux")]
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_REUSEPORT,
                &opt as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::c_int>() as libc::socklen_t,
            );

            let bufsize = (args.buffer * 1024) as libc::socklen_t;
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &bufsize as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::socklen_t>() as libc::socklen_t,
            );
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &bufsize as *const _ as *const libc::c_void,
                std::mem::size_of::<libc::socklen_t>() as libc::socklen_t,
            );

            // 绑定到指定网络接口
            if let Some(ref interface) = args.bind_interface {
                if let Err(e) = set_bind_to_device(fd, interface) {
                    eprintln!("Warning: {}", e);
                }
            }

            libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);

            if libc::bind(
                fd,
                &sockaddr as *const _ as *const libc::sockaddr,
                sockaddr_len,
            ) < 0
            {
                eprintln!("Error: failed to bind UDP socket");
                myexit(1);
            }

            fd
        };

        udp_socket = Some(unsafe { UdpSocket::from_raw_fd(socket) });
        info!("UDP listening on {}", listen_addr);
    }

    if let Err(e) = event_loop.register_listen_socket(tcp_listener, udp_socket) {
        eprintln!("Error: failed to register listen socket: {}", e);
        myexit(1);
    }

    let tcp_handler = event_loop.tcp_handler();
    {
        let mut handler = tcp_handler.write().expect("RwLock poisoned");
        handler.set_remote_addr(remote_addr.clone());
        handler.set_buf_size(args.buffer * 1024);
        handler.set_fwd_type(fwd_type);
        handler.set_bind_interface(args.bind_interface.clone());
    }

    let udp_handler = event_loop.udp_handler();
    {
        let mut handler = udp_handler.write().expect("RwLock poisoned");
        handler.set_remote_addr(remote_addr.clone());
        handler.set_buf_size(args.buffer * 1024);
        handler.set_fwd_type(fwd_type);
        handler.set_bind_interface(args.bind_interface.clone());
    }

    info!("tinyPortMapper started successfully");
    info!("Press Ctrl+C to stop");

    if let Err(e) = event_loop.run() {
        eprintln!("Error: event loop failed: {}", e);
        myexit(1);
    }

    info!("tinyPortMapper stopped");
}

/// 单元测试 - 地址解析测试（类似C++版本的unit_test）
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    #[test]
    fn test_address_parsing() {
        // 测试 IPv6 地址解析
        let addr1 = "[2001:19f0:7001:1111:00:ff:11:22]:443"
            .parse::<Address>()
            .expect("Failed to parse IPv6 address");
        assert_eq!(addr1.get_type(), 6);
        assert_eq!(addr1.port(), 443);

        // 测试 IPv4 地址解析
        let addr2 = "44.55.66.77:443"
            .parse::<Address>()
            .expect("Failed to parse IPv4 address");
        assert_eq!(addr2.to_string(), "44.55.66.77:443");
        assert_eq!(addr2.get_type(), 4);
        assert_eq!(addr2.port(), 443);

        // 测试哈希函数
        let hash1 = {
            let mut hasher = DefaultHasher::new();
            addr1.hash(&mut hasher);
            hasher.finish()
        };
        let hash2 = {
            let mut hasher = DefaultHasher::new();
            addr2.hash(&mut hasher);
            hasher.finish()
        };
        assert_ne!(
            hash1, hash2,
            "Different addresses should have different hashes"
        );

        println!("All address parsing tests passed!");
    }

    #[test]
    fn test_buffer_size_validation() {
        assert!(validate_buffer_size("1024").is_ok());
        assert!(validate_buffer_size("10").is_ok());
        assert!(validate_buffer_size("10240").is_ok());
        assert!(validate_buffer_size("9").is_err());
        assert!(validate_buffer_size("10241").is_err());
        assert!(validate_buffer_size("abc").is_err());
    }
}
