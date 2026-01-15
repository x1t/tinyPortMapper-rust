//! 事件循环模块
//!
//! 基于 mio 的事件驱动框架

use crate::config::Config;
use crate::debug;
use crate::event::signals::SignalHandler;
use crate::event::tcp::TcpHandler;
use crate::event::timer::Timer;
use crate::event::udp::UdpHandler;
use crate::fd_manager::{Fd64, FdManager};
use crate::log::get_current_time;
use crate::log_bare;
use crate::manager::{TcpConnectionManager, UdpSessionManager};
use crate::stats::TrafficStats;

use crate::info;
use mio::net::{TcpListener, UdpSocket};
use mio::{Events, Interest, Poll, Token};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

pub mod signals;
pub mod tcp;
pub mod timer;
pub mod udp;

/// Token 管理器
#[derive(Debug)]
struct TokenManager {
    fd64_to_token: HashMap<Fd64, Token>,
    token_to_fd64: HashMap<Token, Fd64>,
    counter: AtomicUsize,
}

/// 格式化字节数（与 lib.rs 中的 stats 模块保持一致）
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

impl TokenManager {
    fn new() -> Self {
        Self {
            fd64_to_token: HashMap::new(),
            token_to_fd64: HashMap::new(),
            counter: AtomicUsize::new(1),
        }
    }

    fn generate_token(&mut self, fd64: Fd64) -> Token {
        let token = Token(self.counter.fetch_add(1, Ordering::Relaxed));
        self.fd64_to_token.insert(fd64, token);
        self.token_to_fd64.insert(token, fd64);
        token
    }

    fn get_token(&self, fd64: &Fd64) -> Option<Token> {
        self.fd64_to_token.get(fd64).copied()
    }

    fn get_fd64(&self, token: Token) -> Option<Fd64> {
        self.token_to_fd64.get(&token).copied()
    }

    fn remove(&mut self, fd64: &Fd64) -> Option<Token> {
        self.fd64_to_token.remove(fd64).inspect(|token| {
            self.token_to_fd64.remove(token);
        })
    }
}

/// 监听 socket 信息
struct ListenSocket {
    tcp_listener: Option<TcpListener>,
    udp_socket: Option<UdpSocket>,
    tcp_listen_token: Token,
    udp_listen_token: Token,
}

/// 事件循环
pub struct EventLoop {
    poll: Poll,
    token_manager: Arc<RwLock<TokenManager>>,
    fd_manager: Arc<FdManager>,
    tcp_manager: Arc<TcpConnectionManager>,
    udp_manager: Arc<UdpSessionManager>,
    pub config: Arc<Config>,
    tcp_handler: Arc<RwLock<TcpHandler>>,
    udp_handler: Arc<RwLock<UdpHandler>>,
    timer: Timer,
    signal_handler: SignalHandler,
    running: Arc<AtomicBool>,
    listen_socket: RwLock<Option<ListenSocket>>,
}

impl EventLoop {
    pub fn new(
        config: Arc<Config>,
        fd_manager: Arc<FdManager>,
        tcp_manager: Arc<TcpConnectionManager>,
        udp_manager: Arc<UdpSessionManager>,
    ) -> Result<Self, std::io::Error> {
        // 初始化 UdpHandler 并设置分片转发选项
        let mut udp_handler = UdpHandler::new();
        udp_handler.set_enable_fragment(config.enable_udp_fragment);

        Ok(Self {
            poll: Poll::new()?,
            token_manager: Arc::new(RwLock::new(TokenManager::new())),
            fd_manager,
            tcp_manager,
            udp_manager,
            config: Arc::clone(&config),
            tcp_handler: Arc::new(RwLock::new(TcpHandler::new())),
            udp_handler: Arc::new(RwLock::new(udp_handler)),
            timer: Timer::new(),
            signal_handler: SignalHandler::new()?,
            running: Arc::new(AtomicBool::new(false)),
            listen_socket: RwLock::new(None),
        })
    }

    pub fn tcp_handler(&self) -> Arc<RwLock<TcpHandler>> {
        Arc::clone(&self.tcp_handler)
    }

    pub fn udp_handler(&self) -> Arc<RwLock<UdpHandler>> {
        Arc::clone(&self.udp_handler)
    }

    pub fn register_listen_socket(
        &mut self,
        mut tcp_listener: Option<TcpListener>,
        mut udp_socket: Option<UdpSocket>,
    ) -> Result<(), std::io::Error> {
        let mut token_manager = self.token_manager.write().expect("RwLock poisoned");

        let tcp_listen_token = token_manager.generate_token(Fd64(0));
        let udp_listen_token = token_manager.generate_token(Fd64(0));

        if let Some(ref mut listener) = tcp_listener {
            self.poll
                .registry()
                .register(listener, tcp_listen_token, Interest::READABLE)?;
        }

        if let Some(ref mut socket) = udp_socket {
            self.poll
                .registry()
                .register(socket, udp_listen_token, Interest::READABLE)?;
        }

        *self.listen_socket.write().expect("RwLock poisoned") = Some(ListenSocket {
            tcp_listener,
            udp_socket,
            tcp_listen_token,
            udp_listen_token,
        });

        Ok(())
    }

    pub fn run(&mut self) -> Result<(), std::io::Error> {
        self.running.store(true, Ordering::Relaxed);

        self.signal_handler.register()?;

        // 定期统计输出（与 C++ 版本风格一致）
        let stats_interval = Duration::from_secs(10);
        let tcp_manager = Arc::clone(&self.tcp_manager);
        let udp_manager = Arc::clone(&self.udp_manager);
        self.timer.register(stats_interval, move || {
            let tcp_count = tcp_manager.len();
            let udp_count = udp_manager.len();
            let stats = TrafficStats::global();
            let tcp_rx = stats.tcp_bytes_received.load(Ordering::Relaxed);
            let tcp_tx = stats.tcp_bytes_sent.load(Ordering::Relaxed);
            let udp_rx = stats.udp_bytes_received.load(Ordering::Relaxed);
            let udp_tx = stats.udp_bytes_sent.load(Ordering::Relaxed);

            // 格式化输出（与 C++ 版本风格一致）
            log_bare!(
                "[stats] TCP: {}/{}, UDP: {}/{}, conn: TCP={}, UDP={}\n",
                format_bytes(tcp_rx),
                format_bytes(tcp_tx),
                format_bytes(udp_rx),
                format_bytes(udp_tx),
                tcp_count,
                udp_count
            );
        });

        let mut events = Events::with_capacity(1024);
        let mut last_clear_time = 0u64;

        // 检查是否收到终止信号（SIGTERM/SIGINT）
        while self.signal_handler.is_running() {
            self.timer.run();

            // 处理 EINTR 等被信号中断的情况
            let poll_result = self.poll.poll(&mut events, Some(Duration::from_secs(1)));
            // 统计事件数量并打印所有事件
            let event_count = events.iter().count();
            if event_count > 0 {
                debug!("[event] poll returned, events.count={}", event_count);
                for (i, event) in events.iter().enumerate() {
                    debug!(
                        "[event][{}] token={:?}, readable={}, writable={}",
                        i,
                        event.token(),
                        event.is_readable(),
                        event.is_writable()
                    );
                }
            }
            match poll_result {
                Ok(_) => {}
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    // 被信号中断，继续循环
                    continue;
                }
                Err(e) => return Err(e),
            }

            let mut listen_socket_guard = self.listen_socket.write().expect("RwLock poisoned");
            let mut listen_socket = listen_socket_guard.as_mut();

            for event in &events {
                let token = event.token();

                // 调试：打印所有事件（上面已经打印过，这里不再重复）
                // debug!("[event] token={:?}, readable={}, writable={}",
                //        token, event.is_readable(), event.is_writable());

                if let Some(ref mut listen) = listen_socket {
                    if token == listen.tcp_listen_token {
                        if let Some(ref mut listener) = listen.tcp_listener {
                            if event.is_readable() {
                                debug!("[event] TCP listener event, accepting connection");
                                let handler = self.tcp_handler.read().expect("RwLock poisoned");
                                let _ = handler.on_accept(self, token, listener);
                            }
                        }
                        continue;
                    }
                    if token == listen.udp_listen_token {
                        if let Some(ref socket) = listen.udp_socket {
                            if event.is_readable() {
                                let handler = self.udp_handler.read().expect("RwLock poisoned");
                                let _ = handler.on_datagram(self, token, socket);
                            }
                        }
                        continue;
                    }
                }

                let fd64 = {
                    let token_manager = self.token_manager.read().expect("RwLock poisoned");
                    let result = token_manager.get_fd64(token);
                    debug!("[event] token={:?}, fd64={:?}", token, result);
                    result
                };

                if let Some(fd64) = fd64 {
                    debug!("[event] processing token={:?}, fd64={:?}", token, fd64);
                    if !self.fd_manager.exist(fd64) {
                        debug!("[event] fd64 does not exist, skipping");
                        continue;
                    }

                    if event.is_readable() {
                        // 使用 O(1) 查找判断是否是 UDP 会话
                        let is_udp = self.udp_manager.get_session_by_fd64(&fd64).is_some();

                        if is_udp {
                            let handler = self.udp_handler.read().expect("RwLock poisoned");
                            let _ = handler.on_response(self, token, fd64);
                        } else {
                            let handler = self.tcp_handler.read().expect("RwLock poisoned");
                            let _ = handler.on_read(self, token, fd64);
                        }
                    }

                    if event.is_writable() {
                        // 使用 O(1) 查找判断是否是 UDP 会话
                        let is_udp = self.udp_manager.get_session_by_fd64(&fd64).is_some();

                        if !is_udp {
                            let handler = self.tcp_handler.read().expect("RwLock poisoned");
                            let _ = handler.on_write(self, token, fd64);
                        }
                    }
                }
            }

            let now = get_current_time();
            let timer_interval = self.config.timer_interval;
            if now - last_clear_time > timer_interval {
                // 与 C++ 版本 timer_interval 保持一致
                last_clear_time = now;
                self.tcp_manager.clear_inactive();
                self.udp_manager.clear_inactive();
            }
        }

        self.shutdown();
        Ok(())
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    pub fn shutdown(&mut self) {
        info!("[event] shutting down...");

        {
            let connections = self
                .tcp_manager
                .connections
                .write()
                .expect("RwLock poisoned");
            for (fd64, conn) in connections.iter() {
                let conn_guard = conn.read().expect("RwLock poisoned");
                if let Some(raw_fd) = self.fd_manager.to_fd(*fd64) {
                    unsafe {
                        libc::close(raw_fd);
                    }
                }
                if let Some(raw_fd) = self.fd_manager.to_fd(conn_guard.remote.fd64) {
                    unsafe {
                        libc::close(raw_fd);
                    }
                }
            }
        }

        {
            let sessions = self.udp_manager.sessions.read().expect("RwLock poisoned");
            for (_, session) in sessions.iter() {
                let session_guard = session.read().expect("RwLock poisoned");
                if let Some(raw_fd) = self.fd_manager.to_fd(session_guard.fd64) {
                    unsafe {
                        libc::close(raw_fd);
                    }
                }
            }
        }

        info!("[event] shutdown complete");
    }
}
