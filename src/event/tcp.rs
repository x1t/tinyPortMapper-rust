//! TCP 处理器模块 - 使用 splice() 零拷贝转发 (Linux)

use crate::config::FwdType;
use crate::event::EventLoop;
use crate::fd_manager::Fd64;
use crate::stats::TrafficStats;
use crate::types::Address;
use crate::{debug, info, warn};
use mio::net::{TcpListener, TcpStream};
use mio::{Interest, Token};
use std::io;

#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};

#[cfg(unix)]
type RawFd = std::os::unix::io::RawFd;

#[cfg(target_os = "linux")]
const SPLICE_F_MOVE: libc::c_uint = 1;
#[cfg(target_os = "linux")]
const SPLICE_F_NONBLOCK: libc::c_uint = 2;

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
                        interface.as_ptr(),
                        ifreq.ifr_name.as_mut_ptr() as *mut u8,
                        len,
                    );
                }
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
        }
        Ok(())
    }

    fn get_remote_addr_for_connect(&self) -> Address {
        match self.fwd_type {
            FwdType::FwdType4to6 => self
                .remote_addr
                .to_ipv4_mapped_ipv6()
                .unwrap_or_else(|| self.remote_addr.clone()),
            FwdType::FwdType6to4 => self
                .remote_addr
                .from_ipv4_mapped_ipv6()
                .unwrap_or_else(|| self.remote_addr.clone()),
            _ => self.remote_addr.clone(),
        }
    }

    fn get_remote_addr_family(&self) -> libc::c_int {
        match self.fwd_type {
            FwdType::FwdType4to6 => libc::AF_INET6,
            FwdType::FwdType6to4 => libc::AF_INET,
            _ => {
                if self.remote_addr.get_type() == 4 {
                    libc::AF_INET
                } else {
                    libc::AF_INET6
                }
            }
        }
    }

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
        if tcp_manager.len() >= event_loop.config.max_connections {
            warn!("[tcp] max connections reached, closing {}", client_addr);
            return Ok(());
        }

        let fd = stream.as_raw_fd();
        unsafe {
            libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);
            let bufsize = self.socket_buf_size as libc::c_int;
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &bufsize as *const _ as *const libc::c_void,
                4,
            );
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &bufsize as *const _ as *const libc::c_void,
                4,
            );
        }

        let remote_addr_for_connect = self.get_remote_addr_for_connect();
        let remote_fd = unsafe {
            let fd = libc::socket(self.get_remote_addr_family(), libc::SOCK_STREAM, 0);
            if fd < 0 {
                warn!("[tcp] create remote socket failed");
                drop(stream);
                return Ok(());
            }
            let _ = self.set_bind_to_device(fd);
            let bufsize = self.socket_buf_size as libc::c_int;
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &bufsize as *const _ as *const libc::c_void,
                4,
            );
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &bufsize as *const _ as *const libc::c_void,
                4,
            );
            libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);
            fd
        };

        let sockaddr = remote_addr_for_connect.to_sockaddr_storage();
        let ret = unsafe {
            libc::connect(
                remote_fd,
                &sockaddr as *const _ as *const libc::sockaddr,
                remote_addr_for_connect.get_len() as libc::socklen_t,
            )
        };
        let remote_connecting =
            ret != 0 && unsafe { *libc::__errno_location() } == libc::EINPROGRESS;

        let now = crate::log::get_current_time();
        let fd_manager = &event_loop.fd_manager;
        let local_fd64 = fd_manager.create(fd, now);
        let remote_fd64 = fd_manager.create(remote_fd, now);

        let mut tm = token_manager.write().expect("poisoned");
        let local_token = tm.generate_token(local_fd64);
        poll.registry()
            .register(&mut stream, local_token, Interest::READABLE)?;
        let _ = stream.into_raw_fd();

        let remote_token = tm.generate_token(remote_fd64);
        let mut remote_stream = unsafe { TcpStream::from_raw_fd(remote_fd) };
        poll.registry().register(
            &mut remote_stream,
            remote_token,
            if remote_connecting {
                Interest::READABLE | Interest::WRITABLE
            } else {
                Interest::READABLE
            },
        )?;
        let _ = remote_stream.into_raw_fd();

        tcp_manager.new_connection(
            local_fd64,
            remote_fd64,
            client_addr.clone(),
            now,
            self.socket_buf_size,
            remote_connecting,
        );
        TrafficStats::global().inc_tcp_connections();
        info!(
            "[tcp] new connection from {}, fd1={}, fd2={}, tcp connections={}",
            client_addr,
            fd,
            remote_fd,
            tcp_manager.len()
        );
        Ok(())
    }

    /// 使用 splice 进行零拷贝转发
    #[cfg(target_os = "linux")]
    fn do_splice(
        &self,
        src_fd: RawFd,
        dst_fd: RawFd,
        pipe: &mut crate::connection::SplicePipe,
    ) -> Result<isize, i32> {
        let mut total: isize = 0;
        let chunk_size: usize = 256 * 1024; // 256KB per splice call, 提升吞吐量

        loop {
            // 先发送 pipe 中的 pending 数据
            while pipe.pending > 0 {
                let n = unsafe {
                    libc::splice(
                        pipe.read_fd,
                        std::ptr::null_mut(),
                        dst_fd,
                        std::ptr::null_mut(),
                        pipe.pending,
                        SPLICE_F_MOVE | SPLICE_F_NONBLOCK,
                    )
                };
                if n > 0 {
                    pipe.pending -= n as usize;
                    total += n;
                    TrafficStats::global().add_tcp_sent(n as usize);
                } else if n == 0 {
                    return Ok(total);
                } else {
                    let e = unsafe { *libc::__errno_location() };
                    if e == libc::EAGAIN || e == libc::EWOULDBLOCK {
                        return Ok(if total > 0 { total } else { -1 });
                    }
                    return Err(e);
                }
            }

            // 从 src 读取到 pipe
            let n_in = unsafe {
                libc::splice(
                    src_fd,
                    std::ptr::null_mut(),
                    pipe.write_fd,
                    std::ptr::null_mut(),
                    chunk_size,
                    SPLICE_F_MOVE | SPLICE_F_NONBLOCK,
                )
            };
            if n_in > 0 {
                TrafficStats::global().add_tcp_received(n_in as usize);
                // 立即发送到 dst
                let n_out = unsafe {
                    libc::splice(
                        pipe.read_fd,
                        std::ptr::null_mut(),
                        dst_fd,
                        std::ptr::null_mut(),
                        n_in as usize,
                        SPLICE_F_MOVE | SPLICE_F_NONBLOCK,
                    )
                };
                if n_out > 0 {
                    total += n_out;
                    pipe.pending = (n_in - n_out) as usize;
                    TrafficStats::global().add_tcp_sent(n_out as usize);
                    if pipe.pending > 0 {
                        return Ok(total);
                    }
                } else if n_out == 0 {
                    pipe.pending = n_in as usize;
                    return Ok(total);
                } else {
                    let e = unsafe { *libc::__errno_location() };
                    if e == libc::EAGAIN || e == libc::EWOULDBLOCK {
                        pipe.pending = n_in as usize;
                        return Ok(if total > 0 { total } else { -1 });
                    }
                    return Err(e);
                }
            } else if n_in == 0 {
                return Ok(0); // EOF
            } else {
                let e = unsafe { *libc::__errno_location() };
                if e == libc::EAGAIN || e == libc::EWOULDBLOCK {
                    return Ok(if total > 0 { total } else { -1 });
                }
                return Err(e);
            }
        }
    }

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
        let conn_arc = match tcp_manager.get_connection_by_any_fd(&fd64) {
            Some(c) => c,
            None => return Ok(()),
        };

        // 处理连接中状态
        {
            let cg = conn_arc.read().expect("poisoned");
            if fd64 == cg.remote.fd64 && cg.remote_connecting {
                let fd = match fd_manager.to_fd(fd64) {
                    Some(f) => f,
                    None => return Ok(()),
                };
                let mut err: libc::c_int = 0;
                let mut len = 4u32;
                unsafe {
                    libc::getsockopt(
                        fd,
                        libc::SOL_SOCKET,
                        libc::SO_ERROR,
                        &mut err as *mut _ as *mut libc::c_void,
                        &mut len,
                    );
                }
                drop(cg);
                let mut cg = conn_arc.write().expect("poisoned");
                if err == 0 {
                    cg.remote_connecting = false;
                    return Ok(());
                }
                let addr_s = cg.addr_s.clone();
                let other_fd64 = cg.local.fd64;
                let other_fd = fd_manager.to_fd(other_fd64).unwrap_or(-1);
                #[cfg(target_os = "linux")]
                cg.close_pipes();
                drop(cg);
                self.close_connection(event_loop, fd64, other_fd64, fd, other_fd, &addr_s);
                tcp_manager.erase(&fd64);
                return Ok(());
            }
        }

        let mut cg = conn_arc.write().expect("poisoned");
        let is_local = fd64 == cg.local.fd64;
        if cg.remote_connecting && is_local {
            return Ok(());
        }

        let (my_fd64, other_fd64) = if is_local {
            (cg.local.fd64, cg.remote.fd64)
        } else {
            (cg.remote.fd64, cg.local.fd64)
        };
        let my_fd = match fd_manager.to_fd(my_fd64) {
            Some(f) => f,
            None => return Ok(()),
        };
        let other_fd = match fd_manager.to_fd(other_fd64) {
            Some(f) => f,
            None => return Ok(()),
        };
        let addr_s = cg.addr_s.clone();

        // 使用 splice
        #[cfg(target_os = "linux")]
        {
            let pipe = if is_local {
                &mut cg.pipe_l2r
            } else {
                &mut cg.pipe_r2l
            };
            if let Some(ref mut p) = pipe {
                match self.do_splice(my_fd, other_fd, p) {
                    Ok(0) => {
                        info!("[tcp] connection {} closed bc of EOF", addr_s);
                        cg.close_pipes();
                        drop(cg);
                        self.close_connection(
                            event_loop, fd64, other_fd64, my_fd, other_fd, &addr_s,
                        );
                        tcp_manager.erase(&fd64);
                        return Ok(());
                    }
                    Ok(_) => {
                        if p.pending > 0 {
                            if let Some(tok) = token_manager
                                .read()
                                .expect("poisoned")
                                .get_token(&other_fd64)
                            {
                                let mut s = unsafe { TcpStream::from_raw_fd(other_fd) };
                                poll.registry()
                                    .reregister(
                                        &mut s,
                                        tok,
                                        Interest::READABLE | Interest::WRITABLE,
                                    )
                                    .ok();
                                let _ = s.into_raw_fd();
                            }
                        }
                        tcp_manager.update_lru(&fd64);
                        return Ok(());
                    }
                    Err(e) => {
                        info!("[tcp] splice error {}, connection {} closed", e, addr_s);
                        cg.close_pipes();
                        drop(cg);
                        self.close_connection(
                            event_loop, fd64, other_fd64, my_fd, other_fd, &addr_s,
                        );
                        tcp_manager.erase(&fd64);
                        return Ok(());
                    }
                }
            }
        }

        // Fallback: recv/send
        let data_len = if is_local {
            cg.local.data_len
        } else {
            cg.remote.data_len
        };
        if data_len != 0 {
            return Ok(());
        }

        let (buf_ptr, buf_len) = if is_local {
            (cg.local.data.as_mut_ptr(), cg.local.data.len())
        } else {
            (cg.remote.data.as_mut_ptr(), cg.remote.data.len())
        };
        let recv_len = unsafe { libc::recv(my_fd, buf_ptr as *mut libc::c_void, buf_len, 0) };
        if recv_len > 0 {
            TrafficStats::global().add_tcp_received(recv_len as usize);
        }

        if recv_len == 0 {
            info!("[tcp] connection {} closed bc of EOF", addr_s);
            #[cfg(target_os = "linux")]
            cg.close_pipes();
            drop(cg);
            self.close_connection(event_loop, fd64, other_fd64, my_fd, other_fd, &addr_s);
            tcp_manager.erase(&fd64);
            return Ok(());
        }
        if recv_len < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::WouldBlock {
                return Ok(());
            }
            #[cfg(target_os = "linux")]
            cg.close_pipes();
            drop(cg);
            self.close_connection(event_loop, fd64, other_fd64, my_fd, other_fd, &addr_s);
            tcp_manager.erase(&fd64);
            return Ok(());
        }

        if is_local {
            cg.local.data_len = recv_len as usize;
            cg.local.begin = 0;
        } else {
            cg.remote.data_len = recv_len as usize;
            cg.remote.begin = 0;
        }

        let (send_ptr, send_len) = if is_local {
            (cg.local.data.as_ptr(), cg.local.data_len)
        } else {
            (cg.remote.data.as_ptr(), cg.remote.data_len)
        };
        let sent = unsafe { libc::send(other_fd, send_ptr as *const libc::c_void, send_len, 0) };
        if sent > 0 {
            TrafficStats::global().add_tcp_sent(sent as usize);
            if is_local {
                cg.local.data_len -= sent as usize;
                cg.local.begin += sent as usize;
            } else {
                cg.remote.data_len -= sent as usize;
                cg.remote.begin += sent as usize;
            }
        } else if sent < 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock
        {
            #[cfg(target_os = "linux")]
            cg.close_pipes();
            drop(cg);
            self.close_connection(event_loop, fd64, other_fd64, my_fd, other_fd, &addr_s);
            tcp_manager.erase(&fd64);
            return Ok(());
        }

        let pending = if is_local {
            cg.local.data_len
        } else {
            cg.remote.data_len
        };
        if pending > 0 {
            if let Some(tok) = token_manager.read().expect("poisoned").get_token(&fd64) {
                let f = fd_manager.to_fd(fd64).unwrap_or(-1);
                let mut s = unsafe { TcpStream::from_raw_fd(f) };
                poll.registry()
                    .reregister(&mut s, tok, Interest::READABLE | Interest::WRITABLE)
                    .ok();
                let _ = s.into_raw_fd();
            }
        }
        tcp_manager.update_lru(&fd64);
        Ok(())
    }

    pub fn on_write(
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
        let conn_arc = match tcp_manager.get_connection_by_any_fd(&fd64) {
            Some(c) => c,
            None => return Ok(()),
        };
        let mut cg = conn_arc.write().expect("poisoned");

        // 处理连接完成
        if fd64 == cg.remote.fd64 && cg.remote_connecting {
            let fd = match fd_manager.to_fd(fd64) {
                Some(f) => f,
                None => return Ok(()),
            };
            let mut err: libc::c_int = 0;
            let mut len = 4u32;
            unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_ERROR,
                    &mut err as *mut _ as *mut libc::c_void,
                    &mut len,
                );
            }
            if err == 0 {
                cg.remote_connecting = false;
                if let Some(tok) = token_manager.read().expect("poisoned").get_token(&fd64) {
                    let mut s = unsafe { TcpStream::from_raw_fd(fd) };
                    poll.registry()
                        .reregister(&mut s, tok, Interest::READABLE)
                        .ok();
                    let _ = s.into_raw_fd();
                }
                let local_fd64 = cg.local.fd64;
                drop(cg);
                return self.on_read(event_loop, _token, local_fd64);
            }
            let addr_s = cg.addr_s.clone();
            let other_fd64 = cg.local.fd64;
            let other_fd = fd_manager.to_fd(other_fd64).unwrap_or(-1);
            #[cfg(target_os = "linux")]
            cg.close_pipes();
            drop(cg);
            self.close_connection(event_loop, fd64, other_fd64, fd, other_fd, &addr_s);
            tcp_manager.erase(&fd64);
            return Ok(());
        }

        let is_local = fd64 == cg.local.fd64;
        let my_fd = match fd_manager.to_fd(fd64) {
            Some(f) => f,
            None => return Ok(()),
        };
        let other_fd64 = if is_local {
            cg.remote.fd64
        } else {
            cg.local.fd64
        };
        let other_fd = match fd_manager.to_fd(other_fd64) {
            Some(f) => f,
            None => return Ok(()),
        };
        let addr_s = cg.addr_s.clone();

        // 发送 splice pending 数据
        #[cfg(target_os = "linux")]
        {
            // on_write 表示 my_fd 可写，需要发送对端方向的 pending 数据到 my_fd
            let pipe = if is_local {
                &mut cg.pipe_r2l
            } else {
                &mut cg.pipe_l2r
            };
            if let Some(ref mut p) = pipe {
                if p.pending > 0 {
                    let n = unsafe {
                        libc::splice(
                            p.read_fd,
                            std::ptr::null_mut(),
                            my_fd,
                            std::ptr::null_mut(),
                            p.pending,
                            SPLICE_F_MOVE | SPLICE_F_NONBLOCK,
                        )
                    };
                    if n > 0 {
                        p.pending -= n as usize;
                        TrafficStats::global().add_tcp_sent(n as usize);
                    } else if n < 0 {
                        let e = unsafe { *libc::__errno_location() };
                        if e != libc::EAGAIN && e != libc::EWOULDBLOCK {
                            cg.close_pipes();
                            drop(cg);
                            self.close_connection(
                                event_loop, fd64, other_fd64, my_fd, other_fd, &addr_s,
                            );
                            tcp_manager.erase(&fd64);
                            return Ok(());
                        }
                    }
                    if p.pending == 0 {
                        if let Some(tok) = token_manager.read().expect("poisoned").get_token(&fd64)
                        {
                            let mut s = unsafe { TcpStream::from_raw_fd(my_fd) };
                            poll.registry()
                                .reregister(&mut s, tok, Interest::READABLE)
                                .ok();
                            let _ = s.into_raw_fd();
                        }
                    }
                    tcp_manager.update_lru(&fd64);
                    return Ok(());
                }
            }
        }

        // Fallback: 发送 pending 数据
        let (data_len, data_ptr, data_begin) = if is_local {
            (cg.local.data_len, cg.local.data.as_ptr(), cg.local.begin)
        } else {
            (cg.remote.data_len, cg.remote.data.as_ptr(), cg.remote.begin)
        };
        if data_len == 0 {
            return Ok(());
        }

        let sent = unsafe {
            libc::send(
                other_fd,
                data_ptr.add(data_begin) as *const libc::c_void,
                data_len,
                0,
            )
        };
        if sent > 0 {
            TrafficStats::global().add_tcp_sent(sent as usize);
            if is_local {
                cg.local.data_len -= sent as usize;
                cg.local.begin += sent as usize;
            } else {
                cg.remote.data_len -= sent as usize;
                cg.remote.begin += sent as usize;
            }
        } else if sent < 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock
        {
            #[cfg(target_os = "linux")]
            cg.close_pipes();
            drop(cg);
            self.close_connection(event_loop, fd64, other_fd64, my_fd, other_fd, &addr_s);
            tcp_manager.erase(&fd64);
            return Ok(());
        }

        let pending = if is_local {
            cg.local.data_len
        } else {
            cg.remote.data_len
        };
        if pending == 0 {
            if let Some(tok) = token_manager.read().expect("poisoned").get_token(&fd64) {
                let f = fd_manager.to_fd(fd64).unwrap_or(-1);
                let mut s = unsafe { TcpStream::from_raw_fd(f) };
                poll.registry()
                    .reregister(&mut s, tok, Interest::READABLE)
                    .ok();
                let _ = s.into_raw_fd();
            }
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
        addr_s: &str,
    ) {
        let fd_manager = &event_loop.fd_manager;
        let poll = &event_loop.poll;
        let token_manager = &event_loop.token_manager;
        let tcp_manager = &event_loop.tcp_manager;

        if let Some(f) = fd_manager.close(fd64) {
            unsafe {
                libc::close(f);
            }
        }
        if let Some(f) = fd_manager.close(other_fd64) {
            unsafe {
                libc::close(f);
            }
        }

        let mut s1 = unsafe { TcpStream::from_raw_fd(my_fd) };
        poll.registry().deregister(&mut s1).ok();
        let _ = s1.into_raw_fd();

        let mut s2 = unsafe { TcpStream::from_raw_fd(other_fd) };
        poll.registry().deregister(&mut s2).ok();
        let _ = s2.into_raw_fd();

        info!(
            "[tcp]closed connection {} cleared, tcp connections={}",
            addr_s,
            tcp_manager.len()
        );
        TrafficStats::global().dec_tcp_connections();

        let mut tm = token_manager.write().expect("poisoned");
        tm.remove(&fd64);
        tm.remove(&other_fd64);
    }
}

impl Default for TcpHandler {
    fn default() -> Self {
        Self::new()
    }
}
