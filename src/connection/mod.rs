//! 连接数据结构模块
//!
//! TCP 连接和 UDP 会话的数据结构定义

use crate::fd_manager::Fd64;
use crate::types::Address;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// TCP 端点
#[derive(Debug, Clone)]
pub struct TcpEndpoint {
    /// 文件描述符
    pub fd64: Fd64,
    /// 数据缓冲区
    pub data: Vec<u8>,
    /// 缓冲区起始位置
    pub begin: usize,
    /// 有效数据长度
    pub data_len: usize,
}

impl TcpEndpoint {
    /// 创建新的 TCP 端点
    pub fn new(fd64: Fd64, buf_size: usize) -> Self {
        Self {
            fd64,
            data: vec![0u8; buf_size],
            begin: 0,
            data_len: 0,
        }
    }

    /// 清空缓冲区
    pub fn clear(&mut self) {
        self.begin = 0;
        self.data_len = 0;
    }

    /// 获取可用空间
    pub fn available_space(&self) -> usize {
        self.data.len() - (self.begin + self.data_len)
    }

    /// 获取读取切片
    pub fn read_slice(&self) -> &[u8] {
        &self.data[self.begin..self.begin + self.data_len]
    }

    /// 获取写入位置
    pub fn write_pos(&mut self) -> &mut [u8] {
        let start = self.begin + self.data_len;
        &mut self.data[start..]
    }
}

/// TCP 连接对
#[derive(Debug, Clone)]
pub struct TcpConnection {
    /// 本地端
    pub local: TcpEndpoint,
    /// 远程端
    pub remote: TcpEndpoint,
    /// 源地址字符串
    pub addr_s: String,
    /// 创建时间戳
    pub create_time: u64,
    /// 最后活跃时间
    pub last_active_time: Arc<AtomicU64>,
    /// 远程端是否仍在连接中（非阻塞连接尚未完成）
    pub remote_connecting: bool,
}

impl TcpConnection {
    /// 创建新的 TCP 连接
    pub fn new(
        local_fd: Fd64,
        remote_fd: Fd64,
        addr_s: String,
        create_time: u64,
        buf_size: usize,
        remote_connecting: bool,
    ) -> Self {
        Self {
            local: TcpEndpoint::new(local_fd, buf_size),
            remote: TcpEndpoint::new(remote_fd, buf_size),
            addr_s,
            create_time,
            last_active_time: Arc::new(AtomicU64::new(create_time)),
            remote_connecting,
        }
    }

    /// 更新活跃时间
    pub fn update_active(&self) {
        let now = crate::log::get_current_time();
        self.last_active_time.store(now, Ordering::Relaxed);
    }

    /// 获取空闲时间（毫秒）
    pub fn idle_duration(&self) -> Duration {
        let now = crate::log::get_current_time();
        let last = self.last_active_time.load(Ordering::Relaxed);
        Duration::from_millis(now - last)
    }
}

/// UDP 会话
#[derive(Debug, Clone)]
pub struct UdpSession {
    /// 客户端地址
    pub address: Address,
    /// 远程 FD
    pub fd64: Fd64,
    /// 本地监听 FD
    pub local_listen_fd: Fd64,
    /// 地址字符串
    pub addr_s: String,
    /// 创建时间戳
    pub create_time: u64,
    /// 最后活跃时间
    pub last_active_time: Arc<AtomicU64>,
}

impl UdpSession {
    /// 创建新的 UDP 会话
    pub fn new(
        address: Address,
        fd64: Fd64,
        local_listen_fd: Fd64,
        addr_s: String,
        create_time: u64,
    ) -> Self {
        Self {
            address,
            fd64,
            local_listen_fd,
            addr_s,
            create_time,
            last_active_time: Arc::new(AtomicU64::new(create_time)),
        }
    }

    /// 更新活跃时间
    pub fn update_active(&self) {
        let now = crate::log::get_current_time();
        self.last_active_time.store(now, Ordering::Relaxed);
    }

    /// 获取空闲时间（毫秒）
    pub fn idle_duration(&self) -> Duration {
        let now = crate::log::get_current_time();
        let last = self.last_active_time.load(Ordering::Relaxed);
        Duration::from_millis(now - last)
    }
}

/// FD 信息枚举
#[derive(Debug, Clone)]
pub enum FdInfo {
    /// TCP 连接
    Tcp(Arc<TcpConnection>),
    /// UDP 会话
    Udp(Arc<UdpSession>),
}
