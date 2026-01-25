//! TCP 处理器模块 - 使用简单 recv/send 转发 (高性能可靠方案)

use crate::config::FwdType;
use crate::event::EventLoop;
use crate::fd_manager::Fd64;
use crate::manager::TcpConnectionManager;
use crate::stats::TrafficStats;
use crate::types::Address;
use crate::{info, warn, debug};
use mio::net::{TcpListener, TcpStream};
use mio::{Interest, Token};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

/// TCP 处理器
#[derive(Debug)]
pub struct TcpHandler {
    remote_addr: Address,
    socket_buf_size: usize,
    fwd_type: FwdType,
    bind_interface: Option<String>,
}

impl TcpHandler {
    pub fn new() -> Self {
        Self {
            remote_addr: Address::from_ipv4(std::net::Ipv4Addr::UNSPECIFIED, 0),
            socket_buf_size: 16 * 1024,
            fwd_type: FwdType::Normal,
            bind_interface: None,
        }
    }

    pub fn set_remote_addr(&mut self, addr: Address) {
        self.remote_addr = addr;
    }

    pub fn set_buf_size(&mut self, size: usize) {
        self.socket_buf_size = size;
    }

    pub fn set_fwd_type(&mut self, fwd_type: FwdType) {
        self.fwd_type = fwd_type;
    }

    pub fn set_bind_interface(&mut self, interface: Option<String>) {
        self.bind_interface = interface;
    }

    fn set_bind_to_device(&self, fd: libc::c_int) -> Result<(), std::io::Error> {
        #[cfg(target_os = "linux")]
        if let Some(ref interface) = self.bind_interface {
            if !interface.is_empty() {
                let mut ifreq: libc::ifreq = unsafe { std::mem::zeroed() };
                let len = std::cmp::min(interface.len(), libc::IFNAMSIZ - 1);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        interface.as_ptr() as *const libc::c_char,
                        ifreq.ifr_name.as_mut_ptr() as *mut libc::c_char,
                        len,
                    );
                }
                let ret = unsafe {
                    libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_BINDTODEVICE, &ifreq as *const _ as *const libc::c_void, std::mem::size_of::<libc::ifreq>() as libc::socklen_t)
                };
                if ret < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
        }
        Ok(())
    }

    fn get_remote_addr_for_connect(&self) -> Address {
        match self.fwd_type {
            FwdType::FwdType4to6 => self.remote_addr.to_ipv4_mapped_ipv6().unwrap_or_else(|| self.remote_addr.clone()),
            FwdType::FwdType6to4 => self.remote_addr.from_ipv4_mapped_ipv6().unwrap_or_else(|| self.remote_addr.clone()),
            _ => self.remote_addr.clone(),
        }
    }

    fn get_remote_addr_family(&self) -> libc::c_int {
        match self.fwd_type {
            FwdType::FwdType4to6 => libc::AF_INET6,
            FwdType::FwdType6to4 => libc::AF_INET,
            _ => if self.remote_addr.get_type() == 4 { libc::AF_INET } else { libc::AF_INET6 },
        }
    }

    #[inline]
    fn configure_socket(&self, fd: RawFd) -> Result<(), std::io::Error> {
        unsafe {
            libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);
            let bufsize = self.socket_buf_size as libc::c_int;
            let buflen = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_SNDBUF, &bufsize as *const _ as *const libc::c_void, buflen);
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_RCVBUF, &bufsize as *const _ as *const libc::c_void, buflen);
            let nodelay: libc::c_int = 1;
            libc::setsockopt(fd, libc::IPPROTO_TCP, libc::TCP_NODELAY, &nodelay as *const _ as *const libc::c_void, std::mem::size_of::<libc::c_int>() as libc::socklen_t);
        }
        Ok(())
    }

    pub fn on_accept(&self, event_loop: &EventLoop, _token: Token, listener: &mut TcpListener) -> Result<(), std::io::Error> {
        let tcp_manager = &event_loop.tcp_manager;
        let poll = &event_loop.poll;
        let token_manager = &event_loop.token_manager;

        let (mut stream, addr) = match listener.accept() {
            Ok(result) => result,
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e),
        };

        let client_addr = format!("{}", addr);

        if tcp_manager.len() >= event_loop.config.max_connections {
            warn!("[tcp] max connections reached, closing {}", client_addr);
            return Ok(());
        }

        let fd = stream.as_raw_fd();
        self.configure_socket(fd)?;

        let remote_addr_for_connect = self.get_remote_addr_for_connect();
        let remote_fd = unsafe {
            let fd = libc::socket(self.get_remote_addr_family(), libc::SOCK_STREAM, 0);
            if fd < 0 {
                warn!("[tcp] create remote socket failed");
                drop(stream);
                return Ok(());
            }
            let _ = self.set_bind_to_device(fd);
            self.configure_socket(fd).ok();
            fd
        };

        let sockaddr = remote_addr_for_connect.to_sockaddr_storage();
        let ret = unsafe { libc::connect(remote_fd, &sockaddr as *const _ as *const libc::sockaddr, remote_addr_for_connect.get_len() as libc::socklen_t) };
        let remote_connecting = ret != 0 && unsafe { *libc::__errno_location() } == libc::EINPROGRESS;

        let now = crate::log::get_current_time();
        let fd_manager = &event_loop.fd_manager;
        let local_fd64 = fd_manager.create(fd, now);
        let remote_fd64 = fd_manager.create(remote_fd, now);

        let mut tm = token_manager.write().expect("poisoned");
        let local_token = tm.generate_token(local_fd64);
        poll.registry().register(&mut stream, local_token, Interest::READABLE)?;
        let _ = stream.into_raw_fd();

        let remote_token = tm.generate_token(remote_fd64);
        let mut remote_stream = unsafe { TcpStream::from_raw_fd(remote_fd) };
        poll.registry().register(&mut remote_stream, remote_token, if remote_connecting { Interest::READABLE | Interest::WRITABLE } else { Interest::READABLE })?;
        let _ = remote_stream.into_raw_fd();

        tcp_manager.new_connection(local_fd64, remote_fd64, client_addr.clone(), now, self.socket_buf_size, remote_connecting);
        TrafficStats::global().inc_tcp_connections();

        info!("[tcp] new connection from {}, fd1={}, fd2={}, tcp connections={}", client_addr, fd, remote_fd, tcp_manager.len());
        Ok(())
    }

    pub fn on_read(&self, event_loop: &EventLoop, _token: Token, fd64: Fd64) -> Result<(), std::io::Error> {
        debug!("[tcp] on_read ENTRY fd64={:?}", fd64);
        let fd_manager = &event_loop.fd_manager;
        let tcp_manager = &event_loop.tcp_manager;

        if !fd_manager.exist(fd64) {
            debug!("[tcp] on_read: fd64 does not exist");
            return Ok(());
        }

        let conn_arc = match tcp_manager.get_connection_by_any_fd(&fd64) {
            Some(c) => c,
            None => {
                debug!("[tcp] on_read: connection not found");
                return Ok(());
            }
        };

        debug!("[tcp] on_read: got connection arc");
        let conn = conn_arc.read().expect("poisoned");
        debug!("[tcp] on_read: got read lock, remote_connecting={}", conn.remote_connecting);

        if fd64 == conn.remote.fd64 && conn.remote_connecting {
            drop(conn);
            debug!("[tcp] on_read: calling handle_connect_finish");
            return self.handle_connect_finish(event_loop, fd64, fd_manager, tcp_manager);
        }

        let (my_fd64, other_fd64, is_local) = if fd64 == conn.local.fd64 {
            (conn.local.fd64, conn.remote.fd64, true)
        } else {
            (conn.remote.fd64, conn.local.fd64, false)
        };

        let my_fd = match fd_manager.to_fd(my_fd64) {
            Some(f) => f,
            None => return Ok(()),
        };
        let other_fd = match fd_manager.to_fd(other_fd64) {
            Some(f) => f,
            None => return Ok(()),
        };

        let addr_s = conn.addr_s.clone();
        let remote_still_connecting = conn.remote_connecting;

        drop(conn);

        // 即使远程连接尚未完成，也应该尝试读取数据
        // 只是不能将数据发送到尚未建立连接的远程 socket
        let mut conn = conn_arc.write().expect("poisoned");
        let poll = &event_loop.poll;
        let token_manager = &event_loop.token_manager;

        debug!("[tcp] on_read: is_local={}, remote_connecting={}", is_local, remote_still_connecting);

        if is_local {
            // local -> remote
            // 循环读取并发送数据，直到没有更多数据
            debug!("[tcp] local: pending data_len={}", conn.remote.data_len);
            loop {
                // 1. 发送 pending 数据
                if conn.remote.data_len > 0 && !remote_still_connecting {
                    debug!("[tcp] local: sending {} pending bytes", conn.remote.data_len);
                    let sent = unsafe {
                        libc::send(
                            other_fd,
                            conn.remote.data.as_ptr().add(conn.remote.begin) as *const libc::c_void,
                            conn.remote.data_len,
                            0,
                        )
                    };
                    debug!("[tcp] local: sent {}", sent);
                    if sent > 0 {
                        TrafficStats::global().add_tcp_sent(sent as usize);
                        conn.remote.data_len -= sent as usize;
                        conn.remote.begin += sent as usize;
                    } else if sent < 0 {
                        let e = std::io::Error::last_os_error();
                        debug!("[tcp] local: send error {:?}", e.kind());
                        if e.kind() != io::ErrorKind::WouldBlock {
                            Self::close_conn(
                                poll,
                                token_manager,
                                fd_manager,
                                my_fd64,
                                other_fd64,
                                my_fd,
                                other_fd,
                                &addr_s,
                                tcp_manager,
                            );
                            tcp_manager.erase(&fd64);
                            return Ok(());
                        }
                        // WouldBlock，停止发送
                        break;
                    }
                }

                // 2. 从 local 接收数据
                let recv_len = Self::do_recv(my_fd, &mut conn.remote.data);
                debug!("[tcp] local: do_recv returned {}", recv_len);

                if recv_len < 0 {
                    let e = std::io::Error::last_os_error();
                    if e.kind() == io::ErrorKind::WouldBlock {
                        // 没有更多数据，停止
                        break;
                    }
                    // EOF 或错误
                    info!("[tcp] connection {} closed (EOF)", addr_s);
                    Self::close_conn(
                        poll,
                        token_manager,
                        fd_manager,
                        my_fd64,
                        other_fd64,
                        my_fd,
                        other_fd,
                        &addr_s,
                        tcp_manager,
                    );
                    tcp_manager.erase(&fd64);
                    return Ok(());
                }

                if recv_len == 0 {
                    // WouldBlock，停止
                    break;
                }

                // 3. 发送到 remote
                if remote_still_connecting {
                    // 连接尚未建立，缓冲数据
                    debug!(
                        "[tcp] local: buffering {} bytes (connecting)",
                        recv_len
                    );
                    conn.remote.data_len = recv_len as usize;
                    conn.remote.begin = 0;
                    // 不能发送，等待连接建立
                    break;
                } else {
                    let sent = unsafe {
                        libc::send(
                            other_fd,
                            conn.remote.data.as_ptr() as *const libc::c_void,
                            recv_len as usize,
                            0,
                        )
                    };
                    debug!("[tcp] local: sent to remote {}", sent);
                    if sent > 0 {
                        TrafficStats::global().add_tcp_sent(sent as usize);
                        conn.remote.data_len = 0;
                        conn.remote.begin = 0;
                    } else if sent < 0 {
                        let e = std::io::Error::last_os_error();
                        if e.kind() != io::ErrorKind::WouldBlock {
                            Self::close_conn(
                                poll,
                                token_manager,
                                fd_manager,
                                my_fd64,
                                other_fd64,
                                my_fd,
                                other_fd,
                                &addr_s,
                                tcp_manager,
                            );
                            tcp_manager.erase(&fd64);
                            return Ok(());
                        }
                        // WouldBlock，部分发送
                        conn.remote.data_len -= sent as usize;
                        conn.remote.begin += sent as usize;
                        break;
                    } else {
                        conn.remote.data_len = 0;
                        conn.remote.begin = 0;
                    }
                }
            }

            debug!("[tcp] local: exiting loop, pending={}", conn.remote.data_len);

            // 如果有待发送数据，注册 WRITE 事件
            if conn.remote.data_len > 0 && !remote_still_connecting {
                if let Some(tok) = token_manager.read().expect("poisoned").get_token(&fd64) {
                    let mut s = unsafe { TcpStream::from_raw_fd(my_fd) };
                    poll.registry()
                        .reregister(&mut s, tok, Interest::READABLE | Interest::WRITABLE)
                        .ok();
                    let _ = s.into_raw_fd();
                }
            }
        } else {
            // remote -> local
            // 循环读取并发送数据
            loop {
                // 1. 发送 pending 数据到 local
                if conn.remote.data_len > 0 {
                    let sent = unsafe {
                        libc::send(
                            other_fd,
                            conn.remote.data.as_ptr().add(conn.remote.begin) as *const libc::c_void,
                            conn.remote.data_len,
                            0,
                        )
                    };
                    if sent > 0 {
                        TrafficStats::global().add_tcp_sent(sent as usize);
                        conn.remote.data_len -= sent as usize;
                        conn.remote.begin += sent as usize;
                    } else if sent < 0 {
                        let e = std::io::Error::last_os_error();
                        if e.kind() != io::ErrorKind::WouldBlock {
                            Self::close_conn(
                                poll,
                                token_manager,
                                fd_manager,
                                my_fd64,
                                other_fd64,
                                my_fd,
                                other_fd,
                                &addr_s,
                                tcp_manager,
                            );
                            tcp_manager.erase(&fd64);
                            return Ok(());
                        }
                        break;
                    }
                }

                // 2. 从 remote 接收数据
                let recv_len = Self::do_recv(my_fd, &mut conn.remote.data);

                if recv_len < 0 {
                    let e = std::io::Error::last_os_error();
                    if e.kind() == io::ErrorKind::WouldBlock {
                        break;
                    }
                    info!("[tcp] connection {} closed (EOF)", addr_s);
                    Self::close_conn(
                        poll,
                        token_manager,
                        fd_manager,
                        my_fd64,
                        other_fd64,
                        my_fd,
                        other_fd,
                        &addr_s,
                        tcp_manager,
                    );
                    tcp_manager.erase(&fd64);
                    return Ok(());
                }

                if recv_len == 0 {
                    break;
                }

                // 3. 发送到 local
                let sent = unsafe {
                    libc::send(
                        other_fd,
                        conn.remote.data.as_ptr() as *const libc::c_void,
                        recv_len as usize,
                        0,
                    )
                };
                if sent > 0 {
                    TrafficStats::global().add_tcp_sent(sent as usize);
                    conn.remote.data_len = 0;
                    conn.remote.begin = 0;
                } else if sent < 0 {
                    let e = std::io::Error::last_os_error();
                    if e.kind() != io::ErrorKind::WouldBlock {
                        Self::close_conn(
                            poll,
                            token_manager,
                            fd_manager,
                            my_fd64,
                            other_fd64,
                            my_fd,
                            other_fd,
                            &addr_s,
                            tcp_manager,
                        );
                        tcp_manager.erase(&fd64);
                        return Ok(());
                    }
                    conn.remote.data_len -= sent as usize;
                    conn.remote.begin += sent as usize;
                    break;
                } else {
                    conn.remote.data_len = 0;
                    conn.remote.begin = 0;
                }
            }

            // 如果有待发送数据，注册 WRITE 事件
            if conn.remote.data_len > 0 {
                if let Some(tok) = token_manager.read().expect("poisoned").get_token(&fd64) {
                    let mut s = unsafe { TcpStream::from_raw_fd(my_fd) };
                    poll.registry()
                        .reregister(&mut s, tok, Interest::READABLE | Interest::WRITABLE)
                        .ok();
                    let _ = s.into_raw_fd();
                }
            }
        }

        tcp_manager.update_lru(&fd64);
        Ok(())
    }

    #[inline]
    fn do_recv(fd: RawFd, data: &mut [u8]) -> isize {
        // 直接尝试读取数据
        let real_recv = unsafe { libc::recv(fd, data.as_mut_ptr() as *mut libc::c_void, data.len(), 0) };
        
        if real_recv < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == io::ErrorKind::WouldBlock {
                return 0; // 没有数据
            }
            // 其他错误
            return -1;
        }
        
        if real_recv == 0 {
            return -2; // EOF - 对端关闭连接
        }
        
        real_recv
    }

    fn close_conn(
        poll: &mio::Poll,
        token_manager: &std::sync::Arc<std::sync::RwLock<super::TokenManager>>,
        fd_manager: &crate::fd_manager::FdManager,
        fd64: Fd64,
        other_fd64: Fd64,
        my_fd: RawFd,
        other_fd: RawFd,
        addr_s: &str,
        tcp_manager: &TcpConnectionManager,
    ) {
        if let Some(f) = fd_manager.close(fd64) { unsafe { libc::close(f); } }
        if let Some(f) = fd_manager.close(other_fd64) { unsafe { libc::close(f); } }

        let mut s1 = unsafe { TcpStream::from_raw_fd(my_fd) };
        poll.registry().deregister(&mut s1).ok();
        let _ = s1.into_raw_fd();

        let mut s2 = unsafe { TcpStream::from_raw_fd(other_fd) };
        poll.registry().deregister(&mut s2).ok();
        let _ = s2.into_raw_fd();

        info!("[tcp] closed connection {} cleared, tcp connections={}", addr_s, tcp_manager.len());
        TrafficStats::global().dec_tcp_connections();

        let mut tm = token_manager.write().expect("poisoned");
        tm.remove(&fd64);
        tm.remove(&other_fd64);
    }

    fn handle_connect_finish(&self, event_loop: &EventLoop, fd64: Fd64, fd_manager: &crate::fd_manager::FdManager, tcp_manager: &TcpConnectionManager) -> Result<(), std::io::Error> {
        let fd = match fd_manager.to_fd(fd64) {
            Some(f) => f,
            None => {
                debug!("[tcp] handle_connect_finish: fd not found for fd64={:?}", fd64);
                return Ok(());
            }
        };

        let mut err: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_ERROR, &mut err as *mut _ as *mut libc::c_void, &mut len); }

        debug!("[tcp] handle_connect_finish: fd64={:?}, fd={}, SO_ERROR={}", fd64, fd, err);

        let conn_arc = match tcp_manager.get_connection_by_any_fd(&fd64) {
            Some(c) => c,
            None => {
                debug!("[tcp] handle_connect_finish: connection not found");
                return Ok(());
            }
        };

        if err == 0 {
            {
                let mut conn = conn_arc.write().expect("poisoned");
                conn.remote_connecting = false;
                debug!("[tcp] handle_connect_finish: connection established, remote_connecting=false");

                // 如果有缓冲的数据，立即尝试发送
                if conn.local.data_len > 0 {
                    debug!("[tcp] handle_connect_finish: {} buffered bytes ready to send", conn.local.data_len);
                }
            }

            let token_manager = &event_loop.token_manager;

            // 获取 local socket 的 token
            let conn_arc = match tcp_manager.get_connection_by_any_fd(&fd64) {
                Some(c) => c,
                None => return Ok(()),
            };
            let conn = conn_arc.read().expect("poisoned");
            let local_fd64 = conn.local.fd64;
            let local_token = token_manager.read().expect("poisoned").get_token(&local_fd64);
            drop(conn);

            // reregister remote socket
            if let Some(tok) = token_manager.read().expect("poisoned").get_token(&fd64) {
                let mut s = unsafe { TcpStream::from_raw_fd(fd) };
                debug!("[tcp] handle_connect_finish: reregistering remote fd64={:?} with READABLE", fd64);
                event_loop.poll.registry().reregister(&mut s, tok, Interest::READABLE).ok();
                let _ = s.into_raw_fd();
            }

            // 优先调用 local socket 的 on_read 来发送缓冲的数据
            if let Some(tok) = local_token {
                debug!("[tcp] handle_connect_finish: calling on_read for local fd64={:?}", local_fd64);
                return self.on_read(event_loop, tok, local_fd64);
            }

            debug!("[tcp] handle_connect_finish: calling on_read for remote fd64={:?}", fd64);
            return self.on_read(event_loop, token_manager.read().expect("poisoned").get_token(&fd64).unwrap(), fd64);
        }

        debug!("[tcp] handle_connect_finish: connection failed, err={}", err);
        let conn = conn_arc.read().expect("poisoned");
        let addr_s = conn.addr_s.clone();
        let other_fd64 = conn.local.fd64;
        let other_fd = fd_manager.to_fd(other_fd64).unwrap_or(-1);
        drop(conn);

        Self::close_conn(&event_loop.poll, &event_loop.token_manager, fd_manager, fd64, other_fd64, fd, other_fd, &addr_s, tcp_manager);
        tcp_manager.erase(&fd64);
        Ok(())
    }

    pub fn on_write(&self, event_loop: &EventLoop, _token: Token, fd64: Fd64) -> Result<(), std::io::Error> {
        let fd_manager = &event_loop.fd_manager;
        let tcp_manager = &event_loop.tcp_manager;

        if !fd_manager.exist(fd64) {
            return Ok(());
        }

        let conn_arc = match tcp_manager.get_connection_by_any_fd(&fd64) {
            Some(c) => c,
            None => return Ok(()),
        };

        {
            let conn = conn_arc.read().expect("poisoned");
            if fd64 == conn.remote.fd64 && conn.remote_connecting {
                drop(conn);
                return self.handle_connect_finish(event_loop, fd64, fd_manager, tcp_manager);
            }
        }

        let conn = conn_arc.read().expect("poisoned");

        let (my_fd64, other_fd64, is_local) = if fd64 == conn.local.fd64 {
            (conn.local.fd64, conn.remote.fd64, true)
        } else {
            (conn.remote.fd64, conn.local.fd64, false)
        };

        let my_fd = match fd_manager.to_fd(my_fd64) {
            Some(f) => f,
            None => return Ok(()),
        };
        let other_fd = match fd_manager.to_fd(other_fd64) {
            Some(f) => f,
            None => return Ok(()),
        };

        let addr_s = conn.addr_s.clone();
        let pending_data_len = if is_local { conn.local.data_len } else { conn.remote.data_len };

        drop(conn);

        if pending_data_len > 0 {
            let mut conn = conn_arc.write().expect("poisoned");
            let (data_len, data_begin, data_ptr, fd_to_send) = if is_local {
                (conn.local.data_len, conn.local.begin, conn.local.data.as_ptr(), my_fd)
            } else {
                (conn.remote.data_len, conn.remote.begin, conn.remote.data.as_ptr(), my_fd)
            };

            if data_len > 0 {
                let sent = unsafe { libc::send(fd_to_send, data_ptr.add(data_begin) as *const libc::c_void, data_len, 0) };
                if sent > 0 {
                    TrafficStats::global().add_tcp_sent(sent as usize);
                    if is_local {
                        conn.local.data_len -= sent as usize;
                        conn.local.begin += sent as usize;
                    } else {
                        conn.remote.data_len -= sent as usize;
                        conn.remote.begin += sent as usize;
                    }
                } else if sent < 0 {
                    let e = std::io::Error::last_os_error();
                    if e.kind() != io::ErrorKind::WouldBlock {
                        Self::close_conn(&event_loop.poll, &event_loop.token_manager, fd_manager, my_fd64, other_fd64, my_fd, other_fd, &addr_s, tcp_manager);
                        tcp_manager.erase(&fd64);
                        return Ok(());
                    }
                }
            }
        }

        let conn = conn_arc.read().expect("poisoned");
        let pending = if is_local { conn.local.data_len } else { conn.remote.data_len };
        drop(conn);

        if pending == 0 {
            let poll = &event_loop.poll;
            let token_manager = &event_loop.token_manager;
            if let Some(tok) = token_manager.read().expect("poisoned").get_token(&fd64) {
                let mut s = unsafe { TcpStream::from_raw_fd(my_fd) };
                poll.registry().reregister(&mut s, tok, Interest::READABLE).ok();
                let _ = s.into_raw_fd();
            }
        }

        tcp_manager.update_lru(&fd64);
        Ok(())
    }
}

impl Default for TcpHandler {
    fn default() -> Self {
        Self::new()
    }
}
