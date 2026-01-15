//! 日志系统
//!
//! 提供彩色日志输出和级别控制
//! 支持 MY_DEBUG 调试模式（与 C++ 版本保持一致）

use std::fmt;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::Mutex;

/// 全局退出状态标记（与 C++ 版本保持一致）
///
/// 当日志级别为 FATAL 时设置此标记，用于优雅退出
static ABOUT_TO_EXIT: AtomicUsize = AtomicUsize::new(0);

/// 检查是否即将退出
pub fn is_about_to_exit() -> bool {
    ABOUT_TO_EXIT.load(Ordering::Relaxed) != 0
}

/// 设置退出标记
pub fn set_about_to_exit() {
    ABOUT_TO_EXIT.store(1, Ordering::Relaxed);
}

/// MY_DEBUG 调试模式开关（与 C++ 版本保持一致）
/// 在 Cargo.toml 中通过 [features] 配置
#[cfg(feature = "my_debug")]
pub const MY_DEBUG_MODE: bool = true;

#[cfg(not(feature = "my_debug"))]
pub const MY_DEBUG_MODE: bool = false;

/// 日志级别 (与 C++ 版本保持一致: NEVER=0, FATAL=1, ERROR=2, WARN=3, INFO=4, DEBUG=5, TRACE=6)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// 从不输出
    Never = 0,
    /// 致命错误
    Fatal = 1,
    /// 错误
    Error = 2,
    /// 警告
    Warn = 3,
    /// 信息
    Info = 4,
    /// 调试
    Debug = 5,
    /// 追踪
    Trace = 6,
}

impl From<u8> for LogLevel {
    fn from(val: u8) -> Self {
        match val {
            0 => LogLevel::Never,
            1 => LogLevel::Fatal,
            2 => LogLevel::Error,
            3 => LogLevel::Warn,
            4 => LogLevel::Info,
            5 => LogLevel::Debug,
            6 => LogLevel::Trace,
            _ => LogLevel::Trace, // 超过6的按Trace处理，但会在验证时警告
        }
    }
}

impl LogLevel {
    /// 验证日志级别是否有效 (0-6)
    pub fn is_valid(val: u8) -> bool {
        val <= 6
    }

    /// 从 u8 创建日志级别，无效值返回错误
    pub fn from_u8(val: u8) -> Result<Self, &'static str> {
        if val > 6 {
            Err("invalid log_level, must be between 0 and 6")
        } else {
            Ok(Self::from(val))
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogLevel::Never => write!(f, "NEVER"),
            LogLevel::Fatal => write!(f, "FATAL"),
            LogLevel::Error => write!(f, "ERROR"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Debug => write!(f, "DEBUG"),
            LogLevel::Trace => write!(f, "TRACE"),
        }
    }
}

impl Default for Logger {
    fn default() -> Self {
        Self::new()
    }
}

/// 日志器单例
#[derive(Debug)]
pub struct Logger {
    /// 当前日志级别
    log_level: AtomicU8,
    /// 是否启用颜色
    enable_color: AtomicBool,
    /// 是否显示位置信息
    enable_position: AtomicBool,
    /// 日志文件 (Mutex 保护)
    log_file: Mutex<Option<std::fs::File>>,
}

impl Logger {
    /// 创建新的日志器
    pub const fn new() -> Self {
        Self {
            log_level: AtomicU8::new(LogLevel::Info as u8),
            enable_color: AtomicBool::new(true),
            enable_position: AtomicBool::new(true),
            log_file: Mutex::new(None),
        }
    }

    /// 检查是否启用颜色
    pub fn is_color_enabled(&self) -> bool {
        self.enable_color.load(Ordering::Relaxed)
    }

    /// 设置日志级别
    pub fn set_level(&self, level: LogLevel) {
        self.log_level.store(level as u8, Ordering::Relaxed);
    }

    /// 获取日志级别
    pub fn get_level(&self) -> LogLevel {
        LogLevel::from(self.log_level.load(Ordering::Relaxed))
    }

    /// 启用/禁用颜色
    pub fn set_color(&self, enable: bool) {
        self.enable_color.store(enable, Ordering::Relaxed);
    }

    /// 启用/禁用位置信息
    pub fn set_position(&self, enable: bool) {
        self.enable_position.store(enable, Ordering::Relaxed);
    }

    /// 检查是否启用位置信息
    pub fn is_position_enabled(&self) -> bool {
        self.enable_position.load(Ordering::Relaxed)
    }

    /// 检查级别是否启用
    pub fn is_enabled(&self, level: LogLevel) -> bool {
        level as u8 <= self.log_level.load(Ordering::Relaxed)
    }

    /// 打开日志文件
    pub fn open_log_file(&self, path: &str) -> Result<(), std::io::Error> {
        let mut file = std::fs::File::create(path)?;
        // 写入 BOM 以支持中文
        file.write_all(b"\xef\xbb\xbf")?;
        let mut guard = self.log_file.lock().expect("Mutex poisoned");
        *guard = Some(file);
        Ok(())
    }

    /// 写入日志到文件
    pub fn write_to_file(&self, msg: &str) {
        if let Ok(mut guard) = self.log_file.lock() {
            if let Some(ref mut file) = *guard {
                let _ = file.write_all(msg.as_bytes());
                let _ = file.write_all(b"\n");
            }
        }
    }

    /// 获取全局日志器实例
    pub fn global() -> &'static Logger {
        static INSTANCE: Logger = Logger::new();
        &INSTANCE
    }

    /// 检查全局日志器是否启用位置信息
    pub fn global_position_enabled() -> bool {
        Self::global().is_position_enabled()
    }
}

/// 获取当前时间戳（毫秒）- 与 C++ 版本保持一致
///
/// 使用时间修正逻辑，确保时间戳单调递增，处理系统时间回跳
pub fn get_current_time() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::SystemTime;

    static VALUE_FIX: AtomicU64 = AtomicU64::new(0);
    static LARGEST_VALUE: AtomicU64 = AtomicU64::new(0);

    let raw_value = (SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1_000_000.0) as u64;

    let mut value_fix = VALUE_FIX.load(Ordering::Relaxed);
    let mut largest_value = LARGEST_VALUE.load(Ordering::Relaxed);

    let fixed_value = raw_value + value_fix;

    if fixed_value < largest_value {
        value_fix += largest_value - fixed_value;
    } else {
        largest_value = fixed_value;
    }

    VALUE_FIX.store(value_fix, Ordering::Relaxed);
    LARGEST_VALUE.store(largest_value, Ordering::Relaxed);

    (raw_value + value_fix) / 1000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_ordering() {
        // 与 C++ 版本保持一致: NEVER=0, FATAL=1, ERROR=2, WARN=3, INFO=4, DEBUG=5, TRACE=6
        assert!(LogLevel::Debug > LogLevel::Info);
        assert!(LogLevel::Error < LogLevel::Warn); // ERROR(2) < WARN(3)
        assert!(LogLevel::Fatal < LogLevel::Error); // FATAL(1) < ERROR(2)
        assert!(LogLevel::Warn < LogLevel::Info); // WARN(3) < INFO(4)
    }

    #[test]
    fn test_get_current_time() {
        let t1 = get_current_time();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let t2 = get_current_time();
        assert!(t2 >= t1);
    }
}
