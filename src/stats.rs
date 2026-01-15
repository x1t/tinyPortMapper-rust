//! 数据统计模块
//!
//! 跟踪流量统计信息

use std::sync::atomic::{AtomicU64, Ordering};

/// 全局流量统计
#[derive(Debug, Default)]
pub struct TrafficStats {
    /// TCP 接收字节数
    pub tcp_bytes_received: AtomicU64,
    /// TCP 发送字节数
    pub tcp_bytes_sent: AtomicU64,
    /// UDP 接收字节数
    pub udp_bytes_received: AtomicU64,
    /// UDP 发送字节数
    pub udp_bytes_sent: AtomicU64,
    /// TCP 连接数
    pub tcp_connections: AtomicU64,
    /// UDP 会话数
    pub udp_sessions: AtomicU64,
}

impl TrafficStats {
    /// 获取单例实例
    pub fn global() -> &'static Self {
        use std::sync::OnceLock;
        static INSTANCE: OnceLock<TrafficStats> = OnceLock::new();
        INSTANCE.get_or_init(TrafficStats::default)
    }

    /// 增加 TCP 接收字节数
    #[inline]
    pub fn add_tcp_received(&self, bytes: usize) {
        self.tcp_bytes_received
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// 增加 TCP 发送字节数
    #[inline]
    pub fn add_tcp_sent(&self, bytes: usize) {
        self.tcp_bytes_sent
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// 增加 UDP 接收字节数
    #[inline]
    pub fn add_udp_received(&self, bytes: usize) {
        self.udp_bytes_received
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// 增加 UDP 发送字节数
    #[inline]
    pub fn add_udp_sent(&self, bytes: usize) {
        self.udp_bytes_sent
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// 增加 TCP 连接数
    #[inline]
    pub fn inc_tcp_connections(&self) {
        self.tcp_connections.fetch_add(1, Ordering::Relaxed);
    }

    /// 减少 TCP 连接数
    #[inline]
    pub fn dec_tcp_connections(&self) {
        self.tcp_connections.fetch_sub(1, Ordering::Relaxed);
    }

    /// 增加 UDP 会话数
    #[inline]
    pub fn inc_udp_sessions(&self) {
        self.udp_sessions.fetch_add(1, Ordering::Relaxed);
    }

    /// 减少 UDP 会话数
    #[inline]
    pub fn dec_udp_sessions(&self) {
        self.udp_sessions.fetch_sub(1, Ordering::Relaxed);
    }

    /// 获取格式化的统计信息
    pub fn get_stats_string(&self) -> String {
        format!(
            "TCP: {}/{}, UDP: {}/{}",
            format_bytes(self.tcp_bytes_received.load(Ordering::Relaxed)),
            format_bytes(self.tcp_bytes_sent.load(Ordering::Relaxed)),
            format_bytes(self.udp_bytes_received.load(Ordering::Relaxed)),
            format_bytes(self.udp_bytes_sent.load(Ordering::Relaxed))
        )
    }
}

/// 格式化字节数
pub fn format_bytes(bytes: u64) -> String {
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
