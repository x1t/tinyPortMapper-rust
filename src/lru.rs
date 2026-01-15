//! LRU 清理器
//!
//! 基于访问时间排序的最近最少使用清理机制

use std::collections::HashMap;
use std::hash::Hash;
use std::time::Duration;

/// LRU 清理器
///
/// 使用 HashMap 和最小堆实现按访问时间排序的淘汰机制
#[derive(Debug)]
pub struct LruCollector<K, T>
where
    K: Hash + Eq + Clone,
    T: Clone,
{
    /// 值存储 K -> T
    values: HashMap<K, T>,
    /// 访问时间映射 K -> access_time
    access_times: HashMap<K, u64>,
    /// 时间排序的键列表
    time_list: Vec<K>,
    /// 最小堆用于快速找到最旧元素 (time, key)
    min_heap: Vec<(u64, K)>,
}

impl<K, T> LruCollector<K, T>
where
    K: Hash + Eq + Clone + std::fmt::Debug,
    T: Clone,
{
    /// 创建新的 LRU 清理器
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
            access_times: HashMap::new(),
            time_list: Vec::new(),
            min_heap: Vec::new(),
        }
    }

    /// 预分配容量
    pub fn reserve(&mut self, capacity: usize) {
        self.values.reserve(capacity);
        self.access_times.reserve(capacity);
        self.time_list.reserve(capacity);
        self.min_heap.reserve(capacity);
    }

    /// 添加新条目
    pub fn new_key(&mut self, key: K, value: T, access_time: u64) {
        self.values.insert(key.clone(), value);
        self.access_times.insert(key.clone(), access_time);
        self.time_list.push(key.clone());
        self.min_heap.push((access_time, key.clone()));
        self.min_heap.sort_by(|a, b| a.0.cmp(&b.0));
    }

    /// 更新已有条目的访问时间
    pub fn update(&mut self, key: &K, access_time: u64) -> bool {
        if self.access_times.contains_key(key) {
            let new_time = access_time;
            // 更新 min_heap
            for (time, k) in self.min_heap.iter_mut() {
                if k == key {
                    *time = new_time;
                    break;
                }
            }
            self.min_heap.sort_by(|a, b| a.0.cmp(&b.0));
            self.access_times.insert(key.clone(), new_time);
            true
        } else {
            false
        }
    }

    /// 获取最旧的条目
    pub fn peek_back(&mut self) -> Option<(K, T)> {
        // 找到时间戳最小的有效条目
        let min_time = self.min_heap.first().map(|(t, _)| *t)?;

        // 找到对应的键和值
        for key in &self.time_list {
            if self.access_times.get(key) == Some(&min_time) {
                if let Some(value) = self.values.get(key).cloned() {
                    return Some((key.clone(), value));
                }
            }
        }
        None
    }

    /// 删除条目
    pub fn erase(&mut self, key: &K) -> bool {
        let existed = self.values.remove(key).is_some();
        self.access_times.remove(key);
        self.time_list.retain(|k| k != key);
        self.min_heap.retain(|(_, k)| k != key);
        existed
    }

    /// 获取条目数量
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// 检查是否为空
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// 获取指定键的访问时间戳
    ///
    /// 对应 C++ 版本: `my_time_t ts_of(key_t key)`
    pub fn ts_of(&self, key: &K) -> Option<u64> {
        self.access_times.get(key).copied()
    }

    /// 清理超时条目
    pub fn cleanup_timeout(&mut self, timeout: Duration) -> Vec<K> {
        let now = crate::log::get_current_time();
        let timeout_ms = timeout.as_millis() as u64;

        let mut removed = Vec::new();
        self.min_heap.retain(|(time, key)| {
            let is_timeout = now - *time > timeout_ms;
            if is_timeout {
                self.values.remove(key);
                self.access_times.remove(key);
                self.time_list.retain(|k| k != key);
                removed.push(key.clone());
            }
            !is_timeout
        });

        removed
    }
}

impl<K, T> Default for LruCollector<K, T>
where
    K: Hash + Eq + Clone + std::fmt::Debug,
    T: Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_key() {
        let mut lru: LruCollector<&str, &str> = LruCollector::new();
        lru.new_key("key1", "value1", 1000);
        assert_eq!(lru.len(), 1);
    }

    #[test]
    fn test_update() {
        let mut lru: LruCollector<&str, &str> = LruCollector::new();
        lru.new_key("key1", "value1", 1000);
        lru.update(&"key1", 2000);
        assert_eq!(lru.len(), 1);
    }

    #[test]
    fn test_erase() {
        let mut lru: LruCollector<&str, &str> = LruCollector::new();
        lru.new_key("key1", "value1", 1000);
        lru.erase(&"key1");
        assert!(lru.is_empty());
    }

    #[test]
    fn test_cleanup_timeout() {
        let mut lru: LruCollector<&str, &str> = LruCollector::new();
        lru.new_key("key1", "value1", 1000); // Old timestamp
        std::thread::sleep(Duration::from_millis(10));
        lru.new_key("key2", "value2", crate::log::get_current_time());

        let removed = lru.cleanup_timeout(Duration::from_millis(5));
        assert!(removed.contains(&"key1"));
        assert_eq!(lru.len(), 1);
    }

    #[test]
    fn test_peek_back() {
        let mut lru: LruCollector<&str, &str> = LruCollector::new();
        lru.new_key("key1", "value1", 1000);
        lru.new_key("key2", "value2", 2000);

        let (key, value) = lru.peek_back().expect("Lru peek failed");
        assert_eq!(key, "key1"); // Oldest key
        assert_eq!(value, "value1");
    }

    #[test]
    fn test_update_nonexistent() {
        let mut lru: LruCollector<&str, &str> = LruCollector::new();
        // Should not panic
        lru.update(&"nonexistent", 1000);
    }

    #[test]
    fn test_erase_nonexistent() {
        let mut lru: LruCollector<&str, &str> = LruCollector::new();
        lru.new_key("key1", "value1", 1000);
        // Should not panic
        lru.erase(&"nonexistent");
        assert_eq!(lru.len(), 1);
    }

    #[test]
    fn test_cleanup_all() {
        let mut lru: LruCollector<&str, &str> = LruCollector::new();
        lru.new_key("key1", "value1", 1000);
        lru.new_key("key2", "value2", 1001);
        lru.new_key("key3", "value3", 1002);

        let removed = lru.cleanup_timeout(Duration::from_millis(10000));
        assert_eq!(removed.len(), 3);
        assert!(lru.is_empty());
    }
}
