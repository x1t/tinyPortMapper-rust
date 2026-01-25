//! 文件描述符管理器
//!
//! 管理 RawFd 和 Fd64 之间的映射关系

use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::io::RawFd;
#[cfg(windows)]
use std::os::windows::io::RawSocket as RawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// 抽象的文件描述符类型（u64 包装）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fd64(pub u64);

impl Fd64 {
    /// 获取内部值
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

/// FD 信息结构体
#[derive(Debug, Clone)]
pub struct FdInfo {
    /// 创建时间戳
    pub create_time: u64,
    /// 最后活跃时间戳
    pub last_active_time: Arc<AtomicU64>,
}

impl FdInfo {
    /// 创建新的 FD 信息
    pub fn new(create_time: u64) -> Self {
        Self {
            create_time,
            last_active_time: Arc::new(AtomicU64::new(create_time)),
        }
    }

    /// 更新活跃时间
    pub fn update_active(&self) {
        self.last_active_time
            .store(crate::log::get_current_time(), Ordering::Relaxed);
    }
}

/// 文件描述符管理器
///
/// 管理 RawFd 和 Fd64 之间的双向映射，以及 FD 附加信息
#[derive(Debug)]
pub struct FdManager {
    /// RawFd -> Fd64 映射
    fd_to_fd64: RwLock<HashMap<RawFd, Fd64>>,
    /// Fd64 -> RawFd 映射
    fd64_to_fd: RwLock<HashMap<Fd64, RawFd>>,
    /// Fd64 -> FdInfo 映射
    fd_info: RwLock<HashMap<Fd64, FdInfo>>,
    /// Fd64 计数器
    counter: AtomicU64,
}

impl FdManager {
    /// 创建新的 FD 管理器
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            fd_to_fd64: RwLock::new(HashMap::new()),
            fd64_to_fd: RwLock::new(HashMap::new()),
            fd_info: RwLock::new(HashMap::new()),
            counter: AtomicU64::new(1),
        })
    }

    /// 预分配容量
    pub fn reserve(&self, capacity: usize) {
        self.fd_to_fd64
            .write()
            .expect("RwLock poisoned")
            .reserve(capacity);
        self.fd64_to_fd
            .write()
            .expect("RwLock poisoned")
            .reserve(capacity);
        self.fd_info
            .write()
            .expect("RwLock poisoned")
            .reserve(capacity);
    }

    /// 从 RawFd 创建 Fd64
    pub fn create(&self, raw_fd: RawFd, create_time: u64) -> Fd64 {
        let fd64 = Fd64(self.counter.fetch_add(1, Ordering::Relaxed));

        let mut fd_to_fd64 = self.fd_to_fd64.write().expect("RwLock poisoned");
        let mut fd64_to_fd = self.fd64_to_fd.write().expect("RwLock poisoned");
        let mut fd_info = self.fd_info.write().expect("RwLock poisoned");

        fd_to_fd64.insert(raw_fd, fd64);
        fd64_to_fd.insert(fd64, raw_fd);
        fd_info.insert(fd64, FdInfo::new(create_time));

        fd64
    }

    /// 获取现有的 Fd64 或创建新的
    /// 如果 raw_fd 已存在映射，返回现有的 Fd64；否则创建新的
    pub fn get_or_create(&self, raw_fd: RawFd, create_time: u64) -> Fd64 {
        // 首先检查是否已存在
        {
            let fd_to_fd64 = self.fd_to_fd64.read().expect("RwLock poisoned");
            if let Some(fd64) = fd_to_fd64.get(&raw_fd) {
                return *fd64;
            }
        }

        // 不存在，创建新的
        let fd64 = Fd64(self.counter.fetch_add(1, Ordering::Relaxed));

        let mut fd_to_fd64 = self.fd_to_fd64.write().expect("RwLock poisoned");
        let mut fd64_to_fd = self.fd64_to_fd.write().expect("RwLock poisoned");
        let mut fd_info = self.fd_info.write().expect("RwLock poisoned");

        // 双重检查，避免并发创建
        if let Some(existing) = fd_to_fd64.get(&raw_fd) {
            return *existing;
        }

        fd_to_fd64.insert(raw_fd, fd64);
        fd64_to_fd.insert(fd64, raw_fd);
        fd_info.insert(fd64, FdInfo::new(create_time));

        fd64
    }

    /// 将 Fd64 转换为 RawFd
    pub fn to_fd(&self, fd64: Fd64) -> Option<RawFd> {
        self.fd64_to_fd
            .read()
            .expect("RwLock poisoned")
            .get(&fd64)
            .copied()
    }

    /// 检查 Fd64 是否存在
    pub fn exist(&self, fd64: Fd64) -> bool {
        self.fd64_to_fd
            .read()
            .expect("RwLock poisoned")
            .contains_key(&fd64)
    }

    /// 获取 FD 信息
    pub fn get_info(&self, fd64: &Fd64) -> Option<FdInfo> {
        self.fd_info
            .read()
            .expect("RwLock poisoned")
            .get(fd64)
            .cloned()
    }

    /// 检查 FD 信息是否存在
    pub fn exist_info(&self, fd64: &Fd64) -> bool {
        self.fd_info
            .read()
            .expect("RwLock poisoned")
            .contains_key(fd64)
    }

    /// 关闭并清理 Fd64
    pub fn close(&self, fd64: Fd64) -> Option<RawFd> {
        let raw_fd = {
            let mut fd64_to_fd = self.fd64_to_fd.write().expect("RwLock poisoned");
            fd64_to_fd.remove(&fd64)
        };

        if let Some(_raw_fd) = raw_fd {
            let mut fd_to_fd64 = self.fd_to_fd64.write().expect("RwLock poisoned");
            fd_to_fd64.retain(|_, v| *v != fd64);

            let mut fd_info = self.fd_info.write().expect("RwLock poisoned");
            fd_info.remove(&fd64);
        }

        raw_fd
    }

    /// 更新活跃时间
    pub fn update_active(&self, fd64: &Fd64) {
        if let Some(info) = self.fd_info.read().expect("RwLock poisoned").get(fd64) {
            info.update_active();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_and_lookup() {
        let manager: Arc<FdManager> = FdManager::new();
        let raw_fd = 42;
        let fd64 = manager.create(raw_fd, 1000);

        assert_eq!(manager.to_fd(fd64), Some(raw_fd));
        assert!(manager.exist(fd64));
    }

    #[test]
    fn test_close() {
        let manager: Arc<FdManager> = FdManager::new();
        let raw_fd = 42;
        let fd64 = manager.create(raw_fd, 1000);

        assert_eq!(manager.close(fd64), Some(raw_fd));
        assert!(!manager.exist(fd64));
        assert_eq!(manager.to_fd(fd64), None);
    }

    #[test]
    fn test_reserve() {
        let manager: Arc<FdManager> = FdManager::new();
        manager.reserve(100);
    }

    #[test]
    fn test_multiple_fds() {
        let manager: Arc<FdManager> = FdManager::new();
        let fd1 = manager.create(10, 1000);
        let fd2 = manager.create(20, 1000);
        let fd3 = manager.create(30, 1000);

        assert_ne!(fd1, fd2);
        assert_ne!(fd2, fd3);
        assert_ne!(fd1, fd3);

        assert_eq!(manager.to_fd(fd1), Some(10));
        assert_eq!(manager.to_fd(fd2), Some(20));
        assert_eq!(manager.to_fd(fd3), Some(30));
    }

    #[test]
    fn test_fd_info() {
        let manager: Arc<FdManager> = FdManager::new();
        let fd64 = manager.create(42, 1000);

        assert!(manager.exist_info(&fd64));
        assert!(!manager.exist_info(&Fd64(99999)));
    }

    #[test]
    fn test_close_nonexistent() {
        let manager: Arc<FdManager> = FdManager::new();
        let result = manager.close(Fd64(99999));
        assert_eq!(result, None);
    }
}
