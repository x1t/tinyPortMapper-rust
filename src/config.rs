//! 配置模块
//!
//! 命令行参数解析

use crate::log::LogLevel;
use crate::types::Address;
use std::time::Duration;

/// 监听 socket 缓冲区大小 (与 C++ 版本保持一致: 2MB)
pub const LISTEN_FD_BUF_SIZE: usize = 2 * 1024 * 1024;

/// 定时器间隔 (与 C++ 版本保持一致: 400ms)
pub const TIMER_INTERVAL_MS: u64 = 400;

/// 连接清除间隔 (与 C++ 版本保持一致: 1000ms)
pub const CONN_CLEAR_INTERVAL_MS: u64 = 1000;

/// UDP 数据包最大长度 (与 C++ 版本保持一致: 65536)
pub const MAX_DATA_LEN_UDP: usize = 65536;

/// TCP 数据包最大长度 (与 C++ 版本保持一致: 4096*4 = 16384)
pub const MAX_DATA_LEN_TCP: usize = 4096 * 4;

/// 默认最大连接数 (与 C++ 版本保持一致: 20000)
pub const DEFAULT_MAX_CONNECTIONS: usize = 20000;

/// TCP 默认超时时间 (与 C++ 版本保持一致: 360000ms = 360s)
pub const DEFAULT_TCP_TIMEOUT_MS: u64 = 360 * 1000;

/// UDP 默认超时时间 (与 C++ 版本保持一致: 180000ms = 180s)
pub const DEFAULT_UDP_TIMEOUT_MS: u64 = 180 * 1000;

/// 默认连接清除比例 (与 C++ 版本保持一致: 30)
pub const DEFAULT_CONN_CLEAR_RATIO: u32 = 30;

/// 默认连接清除最小数量 (与 C++ 版本保持一致: 1)
pub const DEFAULT_CONN_CLEAR_MIN: u32 = 1;

/// 地址翻译模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FwdType {
    /// 普通转发 (保持原地址类型)
    Normal,
    /// 4to6 翻译模式：IPv4 转发时转换为 IPv6 (::ffff:x.x.x.x)
    FwdType4to6,
    /// 6to4 翻译模式：IPv6 转发时转换为 IPv4
    FwdType6to4,
}

/// 配置结构体
#[derive(Debug, Clone)]
pub struct Config {
    /// 监听地址
    pub listen_addr: Address,
    /// 远程地址
    pub remote_addr: Address,
    /// 启用 TCP
    pub enable_tcp: bool,
    /// 启用 UDP
    pub enable_udp: bool,
    /// Socket 缓冲区大小
    pub socket_buf_size: usize,
    /// 监听 socket 缓冲区大小
    pub listen_fd_buf_size: usize,
    /// 日志级别
    pub log_level: LogLevel,
    /// 显示位置信息
    pub log_position: bool,
    /// 禁用颜色
    pub disable_color: bool,
    /// 最大连接数
    pub max_connections: usize,
    /// TCP 超时
    pub tcp_timeout: Duration,
    /// UDP 超时 (与 C++ 版本的 conn_timeout_udp=180s 对齐)
    pub udp_timeout: Duration,
    /// 连接清除比例 (每 conn_clear_ratio 个连接清除 1 个)
    pub conn_clear_ratio: u32,
    /// 连接清除最小数量
    pub conn_clear_min: u32,
    /// 是否禁用连接清除
    pub disable_conn_clear: bool,
    /// 定时器间隔 (毫秒)
    pub timer_interval: u64,
    /// 转发类型
    pub fwd_type: FwdType,
    /// 绑定的网络接口名称
    pub bind_interface: Option<String>,
    /// 日志文件路径
    pub log_file: Option<String>,
    /// 启用 UDP 分片转发
    pub enable_udp_fragment: bool,
}

impl Config {
    /// 获取监听 socket 缓冲区大小
    pub fn listen_fd_buf_size(&self) -> usize {
        self.listen_fd_buf_size
    }
}
