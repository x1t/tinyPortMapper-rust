//! 连接管理器模块
//!
//! TCP 连接和 UDP 会话的生命周期管理

use crate::connection::{TcpConnection, UdpSession};
use crate::debug;
use crate::fd_manager::Fd64;
use crate::info;
use crate::lru::LruCollector;
use crate::types::Address;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// TCP 连接管理器
#[derive(Debug)]
pub struct TcpConnectionManager {
    /// 连接映射 Fd64 -> TcpConnection (使用 RwLock 保护)
    pub(crate) connections: Arc<RwLock<HashMap<Fd64, Arc<RwLock<TcpConnection>>>>>,
    /// LRU 清理器
    lru: Arc<RwLock<LruCollector<Fd64, Fd64>>>,
    /// 最后清理时间
    last_clear_time: AtomicU64,
    /// 超时时间
    timeout: Duration,
    /// 连接清除比例
    conn_clear_ratio: u32,
    /// 连接清除最小数量
    conn_clear_min: u32,
    /// 是否禁用连接清除
    disable_conn_clear: bool,
}

impl TcpConnectionManager {
    /// 创建新的 TCP 连接管理器
    pub fn new(
        timeout: Duration,
        conn_clear_ratio: u32,
        conn_clear_min: u32,
        disable_conn_clear: bool,
    ) -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
            lru: Arc::new(RwLock::new(LruCollector::<Fd64, Fd64>::new())),
            last_clear_time: AtomicU64::new(0),
            timeout,
            conn_clear_ratio,
            conn_clear_min,
            disable_conn_clear,
        }
    }

    /// 创建新连接
    pub fn new_connection(
        &self,
        local_fd: Fd64,
        remote_fd: Fd64,
        addr_s: String,
        create_time: u64,
        buf_size: usize,
        remote_connecting: bool,
    ) -> Arc<RwLock<TcpConnection>> {
        let connection = Arc::new(RwLock::new(TcpConnection::new(
            local_fd,
            remote_fd,
            addr_s,
            create_time,
            buf_size,
            remote_connecting,
        )));

        let fd64 = local_fd;
        let mut connections = self.connections.write().expect("RwLock poisoned");
        let mut lru = self.lru.write().expect("RwLock poisoned");

        connections.insert(fd64, Arc::clone(&connection));
        lru.new_key(fd64, fd64, create_time);

        connection
    }

    /// 获取连接
    pub fn get_connection(&self, fd64: &Fd64) -> Option<Arc<RwLock<TcpConnection>>> {
        self.connections.read().expect("RwLock poisoned").get(fd64).cloned()
    }

    /// 通过任意 fd64（local 或 remote）获取连接
    pub fn get_connection_by_any_fd(&self, fd64: &Fd64) -> Option<Arc<RwLock<TcpConnection>>> {
        // 首先尝试直接查找
        if let Some(conn) = self.connections.read().expect("RwLock poisoned").get(fd64) {
            return Some(Arc::clone(conn));
        }
        // 如果没找到，遍历查找 remote fd
        let connections = self.connections.read().expect("RwLock poisoned");
        for conn in connections.values() {
            let conn_guard = conn.read().expect("RwLock poisoned");
            if conn_guard.remote.fd64 == *fd64 {
                return Some(Arc::clone(conn));
            }
        }
        None
    }

    /// 清理连接
    pub fn erase(&self, fd64: &Fd64) {
        let mut connections = self.connections.write().expect("RwLock poisoned");
        let mut lru = self.lru.write().expect("RwLock poisoned");

        connections.remove(fd64);
        lru.erase(fd64);
    }

    /// 清理非活跃连接
    pub fn clear_inactive(&self) {
        let now = crate::log::get_current_time();

        // 避免过于频繁清理
        if now - self.last_clear_time.load(Ordering::Relaxed) < 1000 {
            return;
        }

        self.last_clear_time.store(now, Ordering::Relaxed);

        if self.disable_conn_clear {
            return;
        }

        let mut connections = self.connections.write().expect("RwLock poisoned");
        let mut lru = self.lru.write().expect("RwLock poisoned");

        let size = connections.len();
        let num_to_clean = size / self.conn_clear_ratio as usize + self.conn_clear_min as usize;
        let num_to_clean = std::cmp::min(num_to_clean, size);

        // 获取所有超时的连接，按时间排序
        let mut timed_out: Vec<(Fd64, u64, String)> = connections
            .iter()
            .filter_map(|(fd, conn)| {
                let conn_guard = conn.read().expect("RwLock poisoned");
                let last_active = conn_guard.last_active_time.load(Ordering::Relaxed);
                if now - last_active > self.timeout.as_millis() as u64 {
                    Some((*fd, last_active, conn_guard.addr_s.clone()))
                } else {
                    None
                }
            })
            .collect();

        // 按最后活跃时间排序（最旧的在前）
        timed_out.sort_by_key(|(_, ts, _)| *ts);

        // 只清理 num_to_clean 个连接
        let to_remove: Vec<(Fd64, String)> = timed_out
            .into_iter()
            .take(num_to_clean)
            .map(|(fd, _, addr)| (fd, addr))
            .collect();

        for (fd, addr) in &to_remove {
            // 与 C++ 版本保持一致：使用 info 级别打印 inactive connection 日志
            info!(
                "[tcp]inactive connection {} cleared, tcp connections={}",
                addr,
                connections.len().saturating_sub(1)
            );
            debug!("[tcp] lru.size()={}", lru.len().saturating_sub(1));
            connections.remove(fd);
            lru.erase(fd);
        }
    }

    /// 获取连接数量
    pub fn len(&self) -> usize {
        self.connections.read().expect("RwLock poisoned").len()
    }

    /// 检查是否为空
    pub fn is_empty(&self) -> bool {
        self.connections.read().expect("RwLock poisoned").is_empty()
    }

    /// 更新 LRU
    pub fn update_lru(&self, fd64: &Fd64) {
        let now = crate::log::get_current_time();
        let mut lru = self.lru.write().expect("RwLock poisoned");
        lru.update(fd64, now);
    }
}

/// UDP 会话管理器
#[derive(Debug)]
pub struct UdpSessionManager {
    /// 会话映射 Address -> UdpSession (使用 RwLock 保护)
    pub(crate) sessions: Arc<RwLock<HashMap<Address, Arc<RwLock<UdpSession>>>>>,
    /// fd64 到 Address 的映射，用于快速查找
    fd64_to_addr: Arc<RwLock<HashMap<Fd64, Address>>>,
    /// LRU 清理器
    lru: Arc<RwLock<LruCollector<Address, Address>>>,
    /// 最后清理时间
    last_clear_time: AtomicU64,
    /// 超时时间
    timeout: Duration,
    /// 连接清除比例
    conn_clear_ratio: u32,
    /// 连接清除最小数量
    conn_clear_min: u32,
    /// 是否禁用连接清除
    disable_conn_clear: bool,
}

impl UdpSessionManager {
    /// 创建新的 UDP 会话管理器
    pub fn new(
        timeout: Duration,
        conn_clear_ratio: u32,
        conn_clear_min: u32,
        disable_conn_clear: bool,
    ) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            fd64_to_addr: Arc::new(RwLock::new(HashMap::new())),
            lru: Arc::new(RwLock::new(LruCollector::new())),
            last_clear_time: AtomicU64::new(0),
            timeout,
            conn_clear_ratio,
            conn_clear_min,
            disable_conn_clear,
        }
    }

    /// 创建新会话
    pub fn new_session(
        &self,
        address: Address,
        fd64: Fd64,
        local_listen_fd: Fd64,
        addr_s: String,
        create_time: u64,
    ) -> Arc<RwLock<UdpSession>> {
        let address_saved = address.clone();
        let address_lru = address.clone();

        let session = Arc::new(RwLock::new(UdpSession::new(
            address,
            fd64,
            local_listen_fd,
            addr_s,
            create_time,
        )));

        let mut sessions = self.sessions.write().expect("RwLock poisoned");
        let mut fd64_to_addr = self.fd64_to_addr.write().expect("RwLock poisoned");
        let mut lru = self.lru.write().expect("RwLock poisoned");

        sessions.insert(address_saved.clone(), Arc::clone(&session));
        fd64_to_addr.insert(fd64, address_saved.clone());
        lru.new_key(address_lru.clone(), address_lru, create_time);

        session
    }

    /// 获取会话
    pub fn get_session(&self, address: &Address) -> Option<Arc<RwLock<UdpSession>>> {
        self.sessions.read().expect("RwLock poisoned").get(address).cloned()
    }

    /// 通过 fd64 获取会话 (O(1) 查找)
    pub fn get_session_by_fd64(&self, fd64: &Fd64) -> Option<Arc<RwLock<UdpSession>>> {
        let fd64_to_addr = self.fd64_to_addr.read().expect("RwLock poisoned");
        if let Some(addr) = fd64_to_addr.get(fd64) {
            self.sessions.read().expect("RwLock poisoned").get(addr).cloned()
        } else {
            None
        }
    }

    /// 清理会话
    pub fn erase(&self, address: &Address) {
        use crate::stats::TrafficStats;

        let mut sessions = self.sessions.write().expect("RwLock poisoned");
        let mut fd64_to_addr = self.fd64_to_addr.write().expect("RwLock poisoned");
        let mut lru = self.lru.write().expect("RwLock poisoned");

        // 先查找 fd64 再移除
        let fd64_to_remove: Vec<Fd64> = fd64_to_addr
            .iter()
            .filter(|(&_, addr)| **addr == *address)
            .map(|(&fd, _)| fd)
            .collect();

        for fd in &fd64_to_remove {
            fd64_to_addr.remove(fd);
        }

        let addr_s = {
            // 获取地址字符串用于日志
            if let Some(session) = sessions.get(address) {
                let guard = session.read().expect("RwLock poisoned");
                guard.addr_s.clone()
            } else {
                address.to_string()
            }
        };

        // 与 C++ 版本保持一致：打印 inactive connection 日志
        info!(
            "[udp]inactive connection {} cleared, udp connections={}",
            addr_s,
            sessions.len().saturating_sub(1)
        );
        debug!("[udp] lru.size()={}", lru.len().saturating_sub(1));

        sessions.remove(address);
        lru.erase(address);

        // 更新统计
        TrafficStats::global().dec_udp_sessions();
    }

    /// 清理非活跃会话
    pub fn clear_inactive(&self) {
        let now = crate::log::get_current_time();

        if now - self.last_clear_time.load(Ordering::Relaxed) < 1000 {
            return;
        }

        self.last_clear_time.store(now, Ordering::Relaxed);

        if self.disable_conn_clear {
            return;
        }

        let mut sessions = self.sessions.write().expect("RwLock poisoned");
        let mut lru = self.lru.write().expect("RwLock poisoned");

        let size = sessions.len();
        let num_to_clean = size / self.conn_clear_ratio as usize + self.conn_clear_min as usize;
        let num_to_clean = std::cmp::min(num_to_clean, size);

        // 获取所有超时的会话，按时间排序
        let mut timed_out: Vec<(Address, u64)> = sessions
            .iter()
            .filter_map(|(addr, session)| {
                let session_guard = session.read().expect("RwLock poisoned");
                let last_active = session_guard.last_active_time.load(Ordering::Relaxed);
                if now - last_active > self.timeout.as_millis() as u64 {
                    Some((addr.clone(), last_active))
                } else {
                    None
                }
            })
            .collect();

        // 按最后活跃时间排序（最旧的在前）
        timed_out.sort_by_key(|(_, ts)| *ts);

        // 只清理 num_to_clean 个会话
        let to_remove: Vec<Address> = timed_out
            .into_iter()
            .take(num_to_clean)
            .map(|(addr, _)| addr)
            .collect();

        for addr in &to_remove {
            sessions.remove(addr);
            lru.erase(addr);
        }
    }

    /// 获取会话数量
    pub fn len(&self) -> usize {
        self.sessions.read().expect("RwLock poisoned").len()
    }

    /// 检查是否为空
    pub fn is_empty(&self) -> bool {
        self.sessions.read().expect("RwLock poisoned").is_empty()
    }

    /// 更新 LRU
    pub fn update_lru(&self, address: &Address) {
        let now = crate::log::get_current_time();
        let mut lru = self.lru.write().expect("RwLock poisoned");
        lru.update(address, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_tcp_connection_manager() {
        let manager = TcpConnectionManager::new(Duration::from_secs(60), 30, 1, false);

        let _conn =
            manager.new_connection(Fd64(1), Fd64(2), "127.0.0.1:12345".to_string(), 1000, 16384, false);

        assert_eq!(manager.len(), 1);
        assert!(manager.get_connection(&Fd64(1)).is_some());

        manager.erase(&Fd64(1));
        assert!(manager.is_empty());
    }

    #[test]
    fn test_udp_session_manager() {
        let manager = UdpSessionManager::new(Duration::from_secs(30), 30, 1, false);

        let addr = Address::from_str("127.0.0.1:12345").expect("Address parsing failed");
        let addr_clone = addr.clone();
        let _session =
            manager.new_session(addr, Fd64(1), Fd64(2), "127.0.0.1:12345".to_string(), 1000);

        assert_eq!(manager.len(), 1);
        assert!(manager.get_session(&addr_clone).is_some());

        manager.erase(&addr_clone);
        assert!(manager.is_empty());
    }
}
