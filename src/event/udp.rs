//! UDP 处理器模块
//!
//! 处理 UDP 数据包的所有事件

use crate::info;
use crate::warn;
use crate::trace;

use crate::config::FwdType;
use crate::event::EventLoop;
use crate::fd_manager::Fd64;
use crate::stats::TrafficStats;
use crate::types::Address;
use mio::net::UdpSocket;
use mio::Token;
use std::io;

#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};

#[cfg(windows)]
use std::os::windows::io::{AsRawFd, FromRawFd, IntoRawFd};

/// UDP 处理器
#[derive(Debug)]
pub struct UdpHandler {
    /// 远程地址
    remote_addr: Address,
    /// Socket 缓冲区大小
    socket_buf_size: usize,
    /// 转发类型
    fwd_type: FwdType,
    /// 启用 UDP 分片转发 (启用 IP_MTU_DISCOVER)
    enable_fragment: bool,
    /// 绑定的网络接口名称
    bind_interface: Option<String>,
}

impl UdpHandler {
    /// 创建新的 UDP 处理器
    pub fn new() -> Self {
        Self {
            remote_addr: Address::from_ipv4(std::net::Ipv4Addr::UNSPECIFIED, 0),
            socket_buf_size: 16 * 1024,
            fwd_type: FwdType::Normal,
            enable_fragment: false,
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

    /// 启用/禁用 UDP 分片转发
    pub fn set_enable_fragment(&mut self, enable: bool) {
        self.enable_fragment = enable;
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

    /// 设置分片转发的 socket 选项
    fn setup_fragment_socket_options(&self, fd: libc::c_int) -> Result<(), std::io::Error> {
        if !self.enable_fragment {
            return Ok(());
        }

        // 启用路径 MTU 发现 (IP_MTU_DISCOVER)
        // IP_PMTUDISC_DO: 总是进行路径 MTU 发现
        #[cfg(target_os = "linux")]
        {
            let val: libc::c_int = libc::IP_PMTUDISC_DO;
            unsafe {
                if libc::setsockopt(
                    fd,
                    libc::IPPROTO_IP,
                    libc::IP_MTU_DISCOVER,
                    &val as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                ) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }
            }
        }

        // IPv6 的路径 MTU 发现
        #[cfg(target_os = "linux")]
        {
            let val: libc::c_int = libc::IP_PMTUDISC_DO;
            unsafe {
                if libc::setsockopt(
                    fd,
                    libc::IPPROTO_IPV6,
                    libc::IPV6_MTU_DISCOVER,
                    &val as *const _ as *const libc::c_void,
                    std::mem::size_of::<libc::c_int>() as libc::socklen_t,
                ) != 0
                {
                    // IPv6 可能不可用，忽略错误
                }
            }
        }

        Ok(())
    }

    /// 根据转发类型获取远程地址
    fn get_remote_addr_for_connect(&self) -> Address {
        match self.fwd_type {
            FwdType::FwdType4to6 => {
                if let Some(ipv6_addr) = self.remote_addr.to_ipv4_mapped_ipv6() {
                    ipv6_addr
                } else {
                    self.remote_addr.clone()
                }
            }
            FwdType::FwdType6to4 => {
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
            FwdType::FwdType4to6 => libc::AF_INET6,
            FwdType::FwdType6to4 => libc::AF_INET,
            _ => self.remote_addr.get_type() as libc::c_int,
        }
    }

    /// 处理 UDP 数据包
    pub fn on_datagram(
        &self,
        event_loop: &EventLoop,
        _token: Token,
        listen_socket: &UdpSocket,
    ) -> Result<(), std::io::Error> {
        let fd_manager = &event_loop.fd_manager;
        let udp_manager = &event_loop.udp_manager;

        let mut buf = vec![0u8; 65535];
        let (recv_len, src_addr) = match listen_socket.recv_from(&mut buf) {
            Ok(result) => result,
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e),
        };

        // 创建源地址 (支持 IPv4 和 IPv6)
        let src_address = Address::from_sockaddr(src_addr);
        let src_addr_s = src_address.to_string();

        if recv_len > 65535 - 1 {
            warn!("[udp] huge packet from {}, dropped", src_addr_s);
            return Ok(());
        }

        // 与 C++ 版本保持一致: data[data_len] = 0; (便于调试)
        // 注意：这里添加 null 字节便于日志打印，但发送时仍使用原始 recv_len
        if recv_len < buf.len() {
            buf[recv_len] = 0;
        } else {
            // 缓冲区已满，需要扩展
            buf.push(0);
        }

        let session_arc = if let Some(existing) = udp_manager.get_session(&src_address) {
            trace!("[udp] found existing session for {}", src_addr_s);
            existing
        } else {
            if udp_manager.len() >= event_loop.config.max_connections {
                info!(
                    "[udp] max connections reached, dropping packet from {}",
                    src_addr_s
                );
                return Ok(());
            }

            // 与 Go 版本保持一致：使用 Address::new_connected_udp_fd 创建已连接的 UDP socket
            // 这样可以正确处理 IPv4/IPv6 地址转换
            let remote_addr_for_connect = self.get_remote_addr_for_connect();
            let udp_fd = match remote_addr_for_connect.new_connected_udp_fd(self.socket_buf_size) {
                Ok(fd) => fd,
                Err(e) => {
                    info!(
                        "[udp] create connected udp socket failed for {} -> {}: {}",
                        src_addr_s,
                        remote_addr_for_connect,
                        e
                    );
                    return Ok(());
                }
            };

            let now = crate::log::get_current_time();

            // 添加 remote socket 的 fd 到 fd_manager
            let remote_fd64 = fd_manager.create(udp_fd, now);

            // 添加 listen socket 的 fd 到 fd_manager（如果尚未添加）
            let listen_raw_fd = listen_socket.as_raw_fd();
            let listen_fd64 = fd_manager.get_or_create(listen_raw_fd, now);
            trace!(
                "[udp] session for {}, listen_fd={}, listen_fd64={:?}",
                src_addr_s,
                listen_raw_fd,
                listen_fd64
            );

            let poll = &event_loop.poll;
            let token_manager = &event_loop.token_manager;
            let mut token_manager_guard = token_manager.write().expect("token_manager poisoned");
            let tok = token_manager_guard.generate_token(remote_fd64);

            // 创建 UdpSocket 用于注册（不获取所有权）
            #[cfg(unix)]
            let mut remote_socket = unsafe { UdpSocket::from_raw_fd(udp_fd) };
            #[cfg(windows)]
            let mut remote_socket =
                unsafe { UdpSocket::from_raw_socket(udp_fd as std::os::windows::io::RawSocket) };
            if let Err(e) = poll.registry().register(&mut remote_socket, tok, mio::Interest::READABLE) {
                warn!("[udp] failed to register remote socket: {}", e);
                unsafe { libc::close(udp_fd) };
                return Ok(());
            }
            trace!("[udp] registered remote socket with token {:?}", tok);
            #[cfg(unix)]
            let _ = remote_socket.into_raw_fd(); // 防止 drop 时关闭
            #[cfg(windows)]
            let _ = remote_socket.into_raw_socket(); // 防止 drop 时关闭

            // 使用 get_or_create 返回的 listen_fd64
            let session = udp_manager.new_session(
                src_address.clone(),
                remote_fd64,
                listen_fd64,
                src_addr_s.clone(),
                now,
            );

            // 更新统计
            TrafficStats::global().inc_udp_sessions();

            // 与 C++ 版本保持一致：打印 udp fd 和 sessions
            info!(
                "[udp] new connection from {}, udp fd={}, udp connections={}",
                src_addr_s,
                udp_fd,
                udp_manager.len()
            );

            session
        };

        // 获取会话信息并发送
        let session_fd64 = {
            let guard = session_arc.read().expect("session poisoned");
            guard.fd64
        };

        // 直接使用 raw fd 发送，避免 UdpSocket drop 时关闭 fd
        let remote_fd = match fd_manager.to_fd(session_fd64) {
            Some(fd) => fd,
            None => return Ok(()),
        };
        // 与 C++ 版本保持一致：使用 recv_len 而非 buf.len()
        let send_len =
            unsafe { libc::send(remote_fd, buf.as_ptr() as *const libc::c_void, recv_len, 0) };

        // 更新发送统计
        TrafficStats::global().add_udp_sent(send_len as usize);

        if send_len < 0 {
            let err = std::io::Error::last_os_error();
            warn!("[udp] send failed to remote: {}", err);
        } else {
            udp_manager.update_lru(&src_address);
        }

        Ok(())
    }

    /// 处理远程响应
    pub fn on_response(
        &self,
        event_loop: &EventLoop,
        _token: Token,
        fd64: Fd64,
    ) -> Result<(), std::io::Error> {
        const MAX_DATA_LEN_UDP: usize = 65536;

        let fd_manager = &event_loop.fd_manager;
        let udp_manager = &event_loop.udp_manager;

        if !fd_manager.exist(fd64) {
            trace!("[udp] on_response: fd64 {:?} does not exist", fd64);
            return Ok(());
        }

        let fd = match fd_manager.to_fd(fd64) {
            Some(f) => f,
            None => return Ok(()),
        };

        trace!("[udp] on_response: reading from fd {}", fd);
        let mut buf = vec![0u8; MAX_DATA_LEN_UDP + 1];
        let recv_len =
            unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };

        // 更新接收统计
        TrafficStats::global().add_udp_received(recv_len as usize);

        if recv_len < 0 {
            let err = std::io::Error::last_os_error();
            warn!("[udp] recv from remote failed: {}", err);
            return Ok(());
        }

        if recv_len == 0 {
            trace!("[udp] on_response: recv_len = 0, no data");
            return Ok(());
        }

        trace!("[udp] on_response: received {} bytes from remote", recv_len);

        // 检查是否超大包（类似C++版本的处理）
        if recv_len == (MAX_DATA_LEN_UDP + 1) as isize {
            // 获取会话地址用于日志
            if let Some(session_arc) = udp_manager.get_session_by_fd64(&fd64) {
                let guard = session_arc.read().expect("session poisoned");
                warn!("[udp] huge packet from {}, dropped", guard.address);
            }
            return Ok(());
        }

        buf.truncate(recv_len as usize);

        // 使用 O(1) 查找获取会话
        let session_arc = match udp_manager.get_session_by_fd64(&fd64) {
            Some(s) => s,
            None => {
                warn!("[udp] on_response: no session found for fd64 {:?}", fd64);
                return Ok(());
            }
        };

        let (listen_fd, dest_addr, session_addr) = {
            let guard = session_arc.read().expect("session poisoned");
            let lfd = guard.local_listen_fd;
            let addr = guard.address.clone();
            let addr_clone = guard.address.clone();
            (lfd, addr, addr_clone)
        };

        let listen_raw_fd = match fd_manager.to_fd(listen_fd) {
            Some(fd) => fd,
            None => {
                warn!("[udp] on_response: listen_fd not found");
                return Ok(());
            }
        };

        trace!("[udp] on_response: sending {} bytes to client {} via listen_fd {}",
               recv_len, session_addr, listen_raw_fd);

        let dest_sockaddr = dest_addr.to_sockaddr_storage();
        let sockaddr_len = dest_addr.get_len() as libc::socklen_t;

        let send_len = unsafe {
            libc::sendto(
                listen_raw_fd,
                buf.as_ptr() as *const libc::c_void,
                recv_len as usize,
                0,
                &dest_sockaddr as *const _ as *const libc::sockaddr,
                sockaddr_len,
            )
        };

        // 更新发送到客户端的统计
        TrafficStats::global().add_udp_sent(send_len as usize);

        if send_len < 0 {
            let err = std::io::Error::last_os_error();
            warn!("[udp] sendto to client failed: {}", err);
        } else {
            udp_manager.update_lru(&session_addr);
        }

        Ok(())
    }
}

impl Default for UdpHandler {
    fn default() -> Self {
        Self::new()
    }
}
