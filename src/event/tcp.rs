//! TCP 处理器模块
//!
//! 处理 TCP 连接的所有事件

use crate::{debug, info, warn};

use crate::config::FwdType;
use crate::event::EventLoop;
use crate::fd_manager::Fd64;
use crate::stats::TrafficStats;
use crate::types::Address;
use mio::net::TcpStream;
use mio::{Interest, Token};
use std::io;

#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};

#[cfg(windows)]
use std::os::windows::io::{AsRawFd, FromRawFd, IntoRawFd};

// 跨平台 RawFd 类型别名
#[cfg(unix)]
type RawFd = std::os::unix::io::RawFd;

#[cfg(windows)]
type RawFd = std::os::windows::io::RawSocket;

/// TCP 处理器
#[derive(Debug)]
pub struct TcpHandler {
    /// 远程地址
    remote_addr: Address,
    /// Socket 缓冲区大小
    socket_buf_size: usize,
    /// 转发类型
    fwd_type: FwdType,
    /// 绑定的网络接口名称
    bind_interface: Option<String>,
}

impl TcpHandler {
    /// 创建新的 TCP 处理器
    pub fn new() -> Self {
        Self {
            remote_addr: Address::from_ipv4(std::net::Ipv4Addr::UNSPECIFIED, 0),
            socket_buf_size: 16 * 1024,
            fwd_type: FwdType::Normal,
            bind_interface: None,
        }
    }

    /// 设置远程地址
    pub fn set_remote_addr(&mut self, addr: Address) {
        self.remote_addr = addr;
    }

    /// 设置缓冲区大小
    pub fn set_buf_size(&mut self, size: usize) {
        self.socket_buf_size = size;
    }

    /// 设置转发类型
    pub fn set_fwd_type(&mut self, fwd_type: FwdType) {
        self.fwd_type = fwd_type;
    }

    /// 设置绑定的网络接口
    pub fn set_bind_interface(&mut self, interface: Option<String>) {
        self.bind_interface = interface;
    }

    /// 设置 socket 到指定网络接口 (SO_BINDTODEVICE)
    fn set_bind_to_device(&self, fd: libc::c_int) -> Result<(), std::io::Error> {
        if let Some(ref interface) = self.bind_interface {
            if interface.is_empty() {
                return Ok(());
            }
            #[cfg(target_os = "linux")]
            {
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
                    return Err(std::io::Error::last_os_error());
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                // 非 Linux 平台不支持 SO_BINDTODEVICE
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "SO_BINDTODEVICE is not supported on this platform",
                ));
            }
        }
        Ok(())
    }

    /// 根据转发类型获取远程地址
    fn get_remote_addr_for_connect(&self) -> Address {
        match self.fwd_type {
            FwdType::FwdType4to6 => {
                // 4to6: 将 IPv4 地址转换为 IPv4 映射的 IPv6 地址
                if let Some(ipv6_addr) = self.remote_addr.to_ipv4_mapped_ipv6() {
                    ipv6_addr
                } else {
                    self.remote_addr.clone()
                }
            }
            FwdType::FwdType6to4 => {
                // 6to4: 将 IPv6 地址转换为 IPv4
                if let Some(ipv4_addr) = self.remote_addr.from_ipv4_mapped_ipv6() {
                    ipv4_addr
                } else {
                    self.remote_addr.clone()
                }
            }
            _ => self.remote_addr.clone(),
        }
    }

    /// 获取远程地址类型（用于创建 socket）
    fn get_remote_addr_family(&self) -> libc::c_int {
        match self.fwd_type {
            FwdType::FwdType4to6 => libc::AF_INET6, // 4to6 需要创建 IPv6 socket
            FwdType::FwdType6to4 => libc::AF_INET,  // 6to4 需要创建 IPv4 socket
            _ => {
                // 将 ADDR_TYPE_IPV4/IPV6 转换为正确的地址族常量
                // ADDR_TYPE_IPV4 = 4, ADDR_TYPE_IPV6 = 6
                // 但 AF_INET = 2, AF_INET6 = 10 (在大多数系统上)
                if self.remote_addr.get_type() == 4 {
                    libc::AF_INET
                } else {
                    libc::AF_INET6
                }
            }
        }
    }

    /// 处理新连接（accept）
    pub fn on_accept(
        &self,
        event_loop: &EventLoop,
        _token: Token,
        listener: &mut TcpListener,
    ) -> Result<(), std::io::Error> {
        let tcp_manager = &event_loop.tcp_manager;
        let poll = &event_loop.poll;
        let token_manager = &event_loop.token_manager;

        let (mut stream, addr) = match listener.accept() {
            Ok(result) => result,
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e),
        };

        let client_addr = format!("{}", addr);
        debug!("[tcp] accept from {}", client_addr);

        // 记录原始fd
        let raw_client_fd = stream.as_raw_fd();
        debug!("[tcp] client socket fd={}", raw_client_fd);

        if tcp_manager.len() >= event_loop.config.max_connections {
            warn!(
                "[tcp] max connections reached, closing new connection from {}",
                client_addr
            );
            return Ok(());
        }

        let fd = stream.as_raw_fd();
        unsafe {
            libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);
            let bufsize = self.socket_buf_size as libc::socklen_t;
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
        }

        // 创建远程 socket（使用翻译模式的地址类型）
        let remote_addr_for_connect = self.get_remote_addr_for_connect();
        let remote_addr_family = self.get_remote_addr_family();
        let remote_fd = unsafe {
            let fd = libc::socket(remote_addr_family, libc::SOCK_STREAM, 0);
            if fd < 0 {
                warn!(
                    "[tcp] create remote socket failed, errno={}",
                    crate::get_sock_error()
                );
                // 与 C++ 版本保持一致：关闭客户端 socket
                drop(stream);
                return Ok(());
            }

            // 设置接口绑定 (SO_BINDTODEVICE)
            if let Err(e) = self.set_bind_to_device(fd) {
                warn!("[tcp] failed to bind to interface: {}", e);
            }

            let bufsize = self.socket_buf_size as libc::socklen_t;
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
            libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);

            fd
        };

        // 连接到远程地址（需要在 unsafe 块外部执行以正确获取 errno）
        let sockaddr = remote_addr_for_connect.to_sockaddr_storage();
        let sockaddr_len = remote_addr_for_connect.get_len() as libc::socklen_t;
        let ret = unsafe {
            libc::connect(
                remote_fd,
                &sockaddr as *const _ as *const libc::sockaddr,
                sockaddr_len,
            )
        };

        // 检查连接状态
        let connect_errno = unsafe { *libc::__errno_location() };
        let remote_connecting = ret != 0 && connect_errno == libc::EINPROGRESS;

        debug!(
            "[tcp] connect returned {}, errno={} (EINPROGRESS: {}), remote_connecting={}",
            ret,
            crate::get_sock_error(),
            remote_connecting,
            remote_connecting
        );

        let now = crate::log::get_current_time();
        let fd_manager = &event_loop.fd_manager;

        let local_fd64 = fd_manager.create(fd, now);
        let remote_fd64 = fd_manager.create(remote_fd, now);

        let mut token_manager_guard = token_manager.write().expect("token_manager poisoned");
        let local_token = token_manager_guard.generate_token(local_fd64);
        poll.registry()
            .register(&mut stream, local_token, Interest::READABLE)?;
        #[cfg(unix)]
        let _ = stream.into_raw_fd(); // 防止 drop 时关闭
        #[cfg(windows)]
        let _ = stream.into_raw_socket();

        let remote_token = token_manager_guard.generate_token(remote_fd64);

        // 创建 TcpStream 用于注册（不获取所有权）
        #[cfg(unix)]
        let mut remote_stream = unsafe { TcpStream::from_raw_fd(remote_fd) };
        #[cfg(windows)]
        let mut remote_stream =
            unsafe { TcpStream::from_raw_socket(remote_fd as std::os::windows::io::RawSocket) };

        // 与 C++ 版本保持一致：始终注册 READABLE 事件
        // 但对于正在连接中的 socket，需要同时监听 READABLE | WRITABLE
        // 因为在 Linux 上，连接完成时会触发 WRITABLE 事件
        let remote_interest = if remote_connecting {
            Interest::READABLE | Interest::WRITABLE
        } else {
            Interest::READABLE
        };
        poll.registry()
            .register(&mut remote_stream, remote_token, remote_interest)?;
        #[cfg(unix)]
        let _ = remote_stream.into_raw_fd(); // 防止 drop 时关闭
        #[cfg(windows)]
        let _ = remote_stream.into_raw_socket(); // 防止 drop 时关闭

        tcp_manager.new_connection(
            local_fd64,
            remote_fd64,
            client_addr.clone(),
            now,
            self.socket_buf_size,
            remote_connecting,
        );

        // 更新统计
        TrafficStats::global().inc_tcp_connections();

        // 与 C++ 版本保持一致：打印 fd1, fd2, connections
        info!(
            "[tcp] new connection from {}, fd1={}, fd2={}, tcp connections={}",
            client_addr,
            fd,
            remote_fd,
            tcp_manager.len()
        );

        debug!("[tcp] connection registered, local_token={:?}, remote_token={:?}, local_fd64={:?}, remote_fd64={:?}",
               local_token, remote_token, local_fd64, remote_fd64);

        Ok(())
    }

    /// 处理读事件
    pub fn on_read(
        &self,
        event_loop: &EventLoop,
        _token: Token,
        fd64: Fd64,
    ) -> Result<(), std::io::Error> {
        let fd_manager = &event_loop.fd_manager;
        let tcp_manager = &event_loop.tcp_manager;
        let poll = &event_loop.poll;
        let token_manager = &event_loop.token_manager;

        if !fd_manager.exist(fd64) {
            return Ok(());
        }

        // 使用 get_connection_by_any_fd 可以通过 local 或 remote fd64 查找连接
        let connection_arc = match tcp_manager.get_connection_by_any_fd(&fd64) {
            Some(conn) => conn,
            None => return Ok(()),
        };

        let conn_guard = connection_arc.read().expect("connection poisoned");
        debug!(
            "[tcp] on_read: fd64={:?}, local={:?}, remote={:?}, is_remote={}, remote_connecting={}",
            fd64,
            conn_guard.local.fd64,
            conn_guard.remote.fd64,
            fd64 == conn_guard.remote.fd64,
            conn_guard.remote_connecting
        );

        // 检查是否是远程端且仍在连接中（连接完成事件）
        if fd64 == conn_guard.remote.fd64 && conn_guard.remote_connecting {
            // 检查连接状态
            let fd = match fd_manager.to_fd(fd64) {
                Some(fd) => fd,
                None => return Ok(()),
            };

            let mut error: libc::c_int = 0;
            let mut error_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
            let ret = unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_ERROR,
                    &mut error as *mut _ as *mut libc::c_void,
                    &mut error_len,
                )
            };

            // 释放读锁后再获取写锁，避免死锁
            drop(conn_guard);

            let mut conn_guard = connection_arc.write().expect("connection poisoned");
            if ret == 0 && error == 0 {
                // 连接成功完成
                conn_guard.remote_connecting = false;
                debug!("[tcp] connection established, fd={}", fd);
                // 连接完成，等待下一次事件触发时再读取数据
                return Ok(());
            } else {
                // 连接失败
                let conn_addr_s = conn_guard.addr_s.clone();
                let other_fd64 = conn_guard.local.fd64;
                // 使用 ? 操作符安全获取 FD
                let my_fd = match fd_manager.to_fd(fd64) {
                    Some(fd) => fd,
                    None => return Ok(()),
                };
                let other_fd = match fd_manager.to_fd(other_fd64) {
                    Some(fd) => fd,
                    None => return Ok(()),
                };
                warn!(
                    "[tcp] connection failed, error={}, closing {}",
                    error, conn_addr_s
                );
                drop(conn_guard);
                self.close_connection(event_loop, fd64, other_fd64, my_fd, other_fd, &conn_addr_s);
                tcp_manager.erase(&fd64);
                return Ok(());
            }
        }

        // 释放读锁，避免死锁
        drop(conn_guard);

        // 获取连接的读锁来获取 fd64 信息
        let (my_fd64, other_fd64, conn_addr_s): (Fd64, Fd64, String) = {
            let conn_guard = connection_arc.read().expect("connection poisoned");
            let my_fd64 = if fd64 == conn_guard.local.fd64 {
                conn_guard.local.fd64
            } else {
                conn_guard.remote.fd64
            };
            let other_fd64 = if fd64 == conn_guard.local.fd64 {
                conn_guard.remote.fd64
            } else {
                conn_guard.local.fd64
            };
            (my_fd64, other_fd64, conn_guard.addr_s.clone())
        };

        let my_fd = match fd_manager.to_fd(my_fd64) {
            Some(fd) => fd,
            None => return Ok(()),
        };

        let other_fd = match fd_manager.to_fd(other_fd64) {
            Some(fd) => fd,
            None => return Ok(()),
        };

        // 获取写锁来修改数据
        let mut conn_guard = connection_arc.write().expect("connection poisoned");

        // 先获取 other_endpoint_fd64，再获取可变引用
        let other_endpoint_fd64 = if fd64 == conn_guard.local.fd64 {
            conn_guard.remote.fd64
        } else {
            conn_guard.local.fd64
        };

        let remote_connecting = conn_guard.remote_connecting;
        let is_local = fd64 == conn_guard.local.fd64;

        // 如果远程连接还在进行中，且这是本地端，则跳过读取
        if remote_connecting && is_local {
            debug!("[tcp] remote connecting, skipping recv for local fd");
            return Ok(());
        }

        let my_endpoint = if fd64 == conn_guard.local.fd64 {
            &mut conn_guard.local
        } else {
            &mut conn_guard.remote
        };

        if my_endpoint.data_len != 0 {
            debug!(
                "[tcp] data_len={} != 0, skipping recv",
                my_endpoint.data_len
            );
            return Ok(());
        }

        let recv_len = unsafe {
            libc::recv(
                my_fd,
                my_endpoint.data.as_mut_ptr() as *mut libc::c_void,
                my_endpoint.data.len(),
                0,
            )
        };

        // 更新接收统计
        TrafficStats::global().add_tcp_received(recv_len as usize);

        debug!(
            "[tcp] recv from {}, recv_len={}, remote_connecting={}",
            conn_addr_s, recv_len, remote_connecting
        );

        if recv_len == 0 {
            // 与 C++ 版本保持一致：打印 recv_len 和 closed bc of EOF
            info!(
                "[tcp] recv_len={}, connection {} closed bc of EOF",
                recv_len, conn_addr_s
            );
            drop(conn_guard); // 释放锁
            self.close_connection(event_loop, fd64, other_fd64, my_fd, other_fd, &conn_addr_s);
            tcp_manager.erase(&fd64);
            return Ok(());
        }

        if recv_len < 0 {
            let err = std::io::Error::last_os_error();
            // 检查是否是 EAGAIN/EWOULDBLOCK（正常情况，非阻塞 socket 没有数据时）
            if err.kind() == std::io::ErrorKind::WouldBlock {
                debug!("[tcp] recv would block, connection {}", conn_addr_s);
                return Ok(());
            }
            // 与 C++ 版本保持一致：打印 recv_len 和错误信息
            info!(
                "[tcp] recv_len={}, connection {} closed bc of {}, fd={}",
                recv_len, conn_addr_s, err, my_fd
            );
            drop(conn_guard); // 释放锁
            self.close_connection(event_loop, fd64, other_fd64, my_fd, other_fd, &conn_addr_s);
            tcp_manager.erase(&fd64);
            return Ok(());
        }

        // 更新缓冲区
        my_endpoint.data_len = recv_len as usize;
        my_endpoint.begin = 0;

        // 发送数据到对端
        let other_fd_send = match fd_manager.to_fd(other_endpoint_fd64) {
            Some(fd) => fd,
            None => return Ok(()),
        };

        let send_len = unsafe {
            libc::send(
                other_fd_send,
                my_endpoint.data.as_ptr() as *const libc::c_void,
                my_endpoint.data_len,
                0,
            )
        };

        debug!(
            "[tcp] send to {}, send_len={}, data_len={}",
            conn_addr_s, send_len, my_endpoint.data_len
        );

        // 更新发送统计
        TrafficStats::global().add_tcp_sent(send_len as usize);

        if send_len > 0 {
            // 成功发送部分数据
            my_endpoint.data_len = my_endpoint.data_len.saturating_sub(send_len as usize);
            my_endpoint.begin += send_len as usize;
        } else if send_len == 0 {
            // send_len == 0 表示对端关闭了连接
            // 关闭连接并清理资源
            info!(
                "[tcp] send_len==0, peer closed connection {}, closing",
                conn_addr_s
            );
            drop(conn_guard);
            self.close_connection(
                event_loop,
                fd64,
                other_endpoint_fd64,
                my_fd,
                other_fd_send,
                &conn_addr_s,
            );
            tcp_manager.erase(&fd64);
            return Ok(());
        } else {
            // send_len < 0 表示发送错误
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::WouldBlock {
                // 发送缓冲区满，注册 WRITABLE 事件后继续
                debug!(
                    "[tcp] send would block in on_read, connection {}",
                    conn_addr_s
                );
            } else {
                // 其他错误，关闭连接
                info!(
                    "[tcp] send error in on_read: {}, connection {} closed",
                    err, conn_addr_s
                );
                drop(conn_guard);
                self.close_connection(
                    event_loop,
                    fd64,
                    other_endpoint_fd64,
                    my_fd,
                    other_fd_send,
                    &conn_addr_s,
                );
                tcp_manager.erase(&fd64);
                return Ok(());
            }
        }

        if my_endpoint.data_len > 0 {
            let token = token_manager
                .read()
                .expect("token_manager poisoned")
                .get_token(&fd64);
            if let Some(tok) = token {
                let fd = match fd_manager.to_fd(fd64) {
                    Some(f) => f,
                    None => return Ok(()),
                };
                #[cfg(unix)]
                let mut stream = unsafe { TcpStream::from_raw_fd(fd) };
                #[cfg(windows)]
                let mut stream =
                    unsafe { TcpStream::from_raw_socket(fd as std::os::windows::io::RawSocket) };
                poll.registry()
                    .reregister(&mut stream, tok, Interest::WRITABLE)
                    .ok();
                #[cfg(unix)]
                let _ = stream.into_raw_fd();
                #[cfg(windows)]
                let _ = stream.into_raw_socket();
            }
        }

        tcp_manager.update_lru(&fd64);
        Ok(())
    }

    /// 处理写事件
    pub fn on_write(
        &self,
        event_loop: &EventLoop,
        _token: Token,
        fd64: Fd64,
    ) -> Result<(), std::io::Error> {
        debug!("[tcp] on_write called, fd64={:?}", fd64);

        let fd_manager = &event_loop.fd_manager;
        let tcp_manager = &event_loop.tcp_manager;
        let poll = &event_loop.poll;
        let token_manager = &event_loop.token_manager;

        if !fd_manager.exist(fd64) {
            return Ok(());
        }

        // 使用 get_connection_by_any_fd 可以通过 local 或 remote fd64 查找连接
        let connection_arc = match tcp_manager.get_connection_by_any_fd(&fd64) {
            Some(conn) => conn,
            None => return Ok(()),
        };

        let mut conn_guard = connection_arc.write().expect("connection poisoned");

        // 检查是否是远程端且仍在连接中（连接完成事件）
        if fd64 == conn_guard.remote.fd64 && conn_guard.remote_connecting {
            // 检查连接状态
            let fd = match fd_manager.to_fd(fd64) {
                Some(fd) => fd,
                None => return Ok(()),
            };

            let mut error: libc::c_int = 0;
            let mut error_len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
            let ret = unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_ERROR,
                    &mut error as *mut _ as *mut libc::c_void,
                    &mut error_len,
                )
            };

            if ret == 0 && error == 0 {
                // 连接成功完成
                conn_guard.remote_connecting = false;
                debug!("[tcp] connection established (writable), fd={}", fd);

                // 切换为仅 READABLE 事件
                let token = token_manager
                    .read()
                    .expect("token_manager poisoned")
                    .get_token(&fd64);
                if let Some(tok) = token {
                    let fd = match fd_manager.to_fd(fd64) {
                        Some(f) => f,
                        None => return Ok(()),
                    };
                    #[cfg(unix)]
                    let mut stream = unsafe { TcpStream::from_raw_fd(fd) };
                    #[cfg(windows)]
                    let mut stream = unsafe {
                        TcpStream::from_raw_socket(fd as std::os::windows::io::RawSocket)
                    };
                    poll.registry()
                        .reregister(&mut stream, tok, Interest::READABLE)
                        .ok();
                    #[cfg(unix)]
                    let _ = stream.into_raw_fd();
                    #[cfg(windows)]
                    let _ = stream.into_raw_socket();
                }

                // 连接完成，现在尝试读取本地端的数据
                // 因为在连接过程中可能已经有数据到达
                let local_fd64 = conn_guard.local.fd64;
                debug!(
                    "[tcp] connection established, trying to read local data, local_fd64={:?}",
                    local_fd64
                );
                drop(conn_guard); // 释放锁
                let _ = self.on_read(event_loop, _token, local_fd64);

                // 连接完成，返回让事件循环继续处理
                return Ok(());
            } else {
                // 连接失败
                let conn_addr_s = conn_guard.addr_s.clone();
                let my_fd = fd;
                let other_fd64 = conn_guard.local.fd64;
                let other_fd = match fd_manager.to_fd(other_fd64) {
                    Some(fd) => fd,
                    None => return Ok(()),
                };
                warn!(
                    "[tcp] connection failed (writable), error={}, closing {}",
                    error, conn_addr_s
                );
                drop(conn_guard);
                self.close_connection(event_loop, fd64, other_fd64, my_fd, other_fd, &conn_addr_s);
                tcp_manager.erase(&fd64);
                return Ok(());
            }
        }

        // 确定当前端和对端
        // on_write 事件表示 my_fd 可写，应该发送 my_endpoint 中 pending 的数据
        let (my_fd64, other_fd64, is_local) = if fd64 == conn_guard.local.fd64 {
            (conn_guard.local.fd64, conn_guard.remote.fd64, true)
        } else {
            (conn_guard.remote.fd64, conn_guard.local.fd64, false)
        };

        let my_fd = match fd_manager.to_fd(my_fd64) {
            Some(fd) => fd,
            None => return Ok(()),
        };

        let other_fd = match fd_manager.to_fd(other_fd64) {
            Some(fd) => fd,
            None => return Ok(()),
        };

        // 检查当前端是否有待发送数据
        let (my_endpoint_data_len, my_endpoint_data_ptr, my_endpoint_begin) = if is_local {
            (
                conn_guard.local.data_len,
                conn_guard.local.data.as_ptr(),
                conn_guard.local.begin,
            )
        } else {
            (
                conn_guard.remote.data_len,
                conn_guard.remote.data.as_ptr(),
                conn_guard.remote.begin,
            )
        };

        // on_write 事件表示 my_fd 可写，应该把 pending 的数据发送到对端
        if my_endpoint_data_len == 0 {
            return Ok(());
        }

        // 发送 pending 的数据到对端
        let send_len = unsafe {
            libc::send(
                other_fd,
                my_endpoint_data_ptr.add(my_endpoint_begin) as *const libc::c_void,
                my_endpoint_data_len,
                0,
            )
        };

        // 更新发送统计
        TrafficStats::global().add_tcp_sent(send_len as usize);

        let conn_addr_s = conn_guard.addr_s.clone();

        if send_len == 0 {
            // send_len == 0 表示对端关闭了连接，或者缓冲区暂时不可用
            // 检查是否还有 pending 的数据需要发送
            let pending_len = if is_local {
                conn_guard.local.data_len
            } else {
                conn_guard.remote.data_len
            };

            if pending_len == 0 {
                // 没有 pending 数据，连接可能被对端关闭
                info!(
                    "[tcp] send_len={}, connection {} closed bc of EOF",
                    send_len, conn_addr_s
                );
                drop(conn_guard);
                self.close_connection(event_loop, fd64, other_fd64, my_fd, other_fd, &conn_addr_s);
                tcp_manager.erase(&fd64);
                return Ok(());
            } else {
                // 有 pending 数据，但 send 返回 0 (可能是缓冲区满)
                // 重新注册 WRITABLE 事件
                let token = token_manager
                    .read()
                    .expect("token_manager poisoned")
                    .get_token(&fd64);
                if let Some(tok) = token {
                    let fd = match fd_manager.to_fd(fd64) {
                        Some(f) => f,
                        None => {
                            tcp_manager.update_lru(&fd64);
                            return Ok(());
                        }
                    };
                    #[cfg(unix)]
                    let mut stream = unsafe { TcpStream::from_raw_fd(fd) };
                    #[cfg(windows)]
                    let mut stream = unsafe {
                        TcpStream::from_raw_socket(fd as std::os::windows::io::RawSocket)
                    };
                    poll.registry()
                        .reregister(&mut stream, tok, Interest::READABLE | Interest::WRITABLE)
                        .ok();
                    #[cfg(unix)]
                    let _ = stream.into_raw_fd();
                    #[cfg(windows)]
                    let _ = stream.into_raw_socket();
                }
                tcp_manager.update_lru(&fd64);
                return Ok(());
            }
        }

        if send_len < 0 {
            let err = std::io::Error::last_os_error();
            // 检查是否是 EAGAIN/EWOULDBLOCK（正常情况，非阻塞 socket 缓冲区满时）
            if err.kind() == std::io::ErrorKind::WouldBlock {
                debug!(
                    "[tcp] send would block, connection {}, re-registering writable",
                    conn_addr_s
                );
                // 重新注册 WRITABLE 事件，以便在 socket 可写时继续发送
                let token = token_manager
                    .read()
                    .expect("token_manager poisoned")
                    .get_token(&fd64);
                if let Some(tok) = token {
                    let fd = match fd_manager.to_fd(fd64) {
                        Some(f) => f,
                        None => {
                            tcp_manager.update_lru(&fd64);
                            return Ok(());
                        }
                    };
                    #[cfg(unix)]
                    let mut stream = unsafe { TcpStream::from_raw_fd(fd) };
                    #[cfg(windows)]
                    let mut stream = unsafe {
                        TcpStream::from_raw_socket(fd as std::os::windows::io::RawSocket)
                    };
                    poll.registry()
                        .reregister(&mut stream, tok, Interest::READABLE | Interest::WRITABLE)
                        .ok();
                    #[cfg(unix)]
                    let _ = stream.into_raw_fd();
                    #[cfg(windows)]
                    let _ = stream.into_raw_socket();
                }
                tcp_manager.update_lru(&fd64);
                return Ok(());
            }
            // 与 C++ 版本保持一致
            info!(
                "[tcp] send_len={}, connection {} closed bc of {}",
                send_len, conn_addr_s, err
            );
            drop(conn_guard);
            self.close_connection(event_loop, fd64, other_fd64, my_fd, other_fd, &conn_addr_s);
            tcp_manager.erase(&fd64);
            return Ok(());
        }

        // 更新当前端的状态
        if send_len > 0 {
            if is_local {
                conn_guard.local.data_len =
                    conn_guard.local.data_len.saturating_sub(send_len as usize);
                conn_guard.local.begin += send_len as usize;
            } else {
                conn_guard.remote.data_len =
                    conn_guard.remote.data_len.saturating_sub(send_len as usize);
                conn_guard.remote.begin += send_len as usize;
            }
        }

        let pending_len = if is_local {
            conn_guard.local.data_len
        } else {
            conn_guard.remote.data_len
        };

        if pending_len == 0 {
            let token = token_manager
                .read()
                .expect("token_manager poisoned")
                .get_token(&fd64);
            if let Some(tok) = token {
                let fd = match fd_manager.to_fd(fd64) {
                    Some(f) => f,
                    None => return Ok(()),
                };
                #[cfg(unix)]
                let mut stream = unsafe { TcpStream::from_raw_fd(fd) };
                #[cfg(windows)]
                let mut stream =
                    unsafe { TcpStream::from_raw_socket(fd as std::os::windows::io::RawSocket) };
                poll.registry()
                    .reregister(&mut stream, tok, Interest::READABLE)
                    .ok();
                #[cfg(unix)]
                let _ = stream.into_raw_fd();
                #[cfg(windows)]
                let _ = stream.into_raw_socket();
            }
        } else {
            // 继续保持 WRITABLE 事件，等待更多可写机会
        }

        tcp_manager.update_lru(&fd64);
        Ok(())
    }

    fn close_connection(
        &self,
        event_loop: &EventLoop,
        fd64: Fd64,
        other_fd64: Fd64,
        my_fd: RawFd,
        other_fd: RawFd,
        conn_addr_s: &str,
    ) {
        let fd_manager = &event_loop.fd_manager;
        let poll = &event_loop.poll;
        let token_manager = &event_loop.token_manager;
        let tcp_manager = &event_loop.tcp_manager;

        if let Some(raw_fd) = fd_manager.close(fd64) {
            #[cfg(unix)]
            unsafe {
                libc::close(raw_fd);
            }
            #[cfg(windows)]
            unsafe {
                libc::closesocket(raw_fd as std::os::windows::io::RawSocket);
            }
        }
        if let Some(raw_fd) = fd_manager.close(other_fd64) {
            #[cfg(unix)]
            unsafe {
                libc::close(raw_fd);
            }
            #[cfg(windows)]
            unsafe {
                libc::closesocket(raw_fd as std::os::windows::io::RawSocket);
            }
        }

        #[cfg(unix)]
        let mut stream = unsafe { TcpStream::from_raw_fd(my_fd) };
        #[cfg(windows)]
        let mut stream =
            unsafe { TcpStream::from_raw_socket(my_fd as std::os::windows::io::RawSocket) };
        poll.registry().deregister(&mut stream).ok();
        #[cfg(unix)]
        let _ = stream.into_raw_fd();
        #[cfg(windows)]
        let _ = stream.into_raw_socket();

        #[cfg(unix)]
        let mut stream2 = unsafe { TcpStream::from_raw_fd(other_fd) };
        #[cfg(windows)]
        let mut stream2 =
            unsafe { TcpStream::from_raw_socket(other_fd as std::os::windows::io::RawSocket) };
        poll.registry().deregister(&mut stream2).ok();
        #[cfg(unix)]
        let _ = stream2.into_raw_fd();
        #[cfg(windows)]
        let _ = stream2.into_raw_socket();

        // 与 C++ 版本保持一致：打印 closed connection 日志
        info!(
            "[tcp]closed connection {} cleared, tcp connections={}",
            conn_addr_s,
            tcp_manager.len()
        );

        // 更新统计
        TrafficStats::global().dec_tcp_connections();

        let mut token_manager_guard = token_manager.write().expect("token_manager poisoned");
        token_manager_guard.remove(&fd64);
        token_manager_guard.remove(&other_fd64);
    }
}

impl Default for TcpHandler {
    fn default() -> Self {
        Self::new()
    }
}

use mio::net::TcpListener;
