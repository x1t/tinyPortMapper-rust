//! 定时器模块
//!
//! 提供定时任务功能

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::vec::Vec;

/// 定时器回调类型
pub type TimerCallback = Box<dyn Fn() + Send + Sync>;

/// 定时器
pub struct Timer {
    /// 定时器条目
    entries: Arc<Mutex<BTreeMap<Instant, Vec<TimerEntry>>>>,
}

struct TimerEntry {
    /// 回调函数 (使用 Option 以便取出)
    callback: Option<TimerCallback>,
    /// 间隔
    interval: Duration,
    /// 是否已标记删除
    deleted: Arc<AtomicBool>,
}

impl Timer {
    /// 创建新的定时器
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// 注册定时任务
    pub fn register<F>(&self, interval: Duration, callback: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        let mut entries = self.entries.lock().expect("Mutex poisoned");
        let now = Instant::now();
        let next_time = now + interval;

        let entry = TimerEntry {
            callback: Some(Box::new(callback)),
            interval,
            deleted: Arc::new(AtomicBool::new(false)),
        };

        entries.entry(next_time).or_default().push(entry);
    }

    /// 运行定时器 - 执行所有到期的回调
    pub fn run(&self) {
        let now = Instant::now();
        let mut to_remove: Vec<Instant> = Vec::new();
        let mut to_reschedule: Vec<(Duration, TimerCallback, Arc<AtomicBool>)> = Vec::new();

        // 收集到期的回调
        {
            let mut entries = self.entries.lock().expect("Mutex poisoned");

            for (time, vec) in entries.iter_mut() {
                if *time <= now {
                    for entry in vec.iter_mut() {
                        if !entry.deleted.load(Ordering::Relaxed) {
                            // 标记为删除
                            entry.deleted.store(true, Ordering::Relaxed);
                            // 取出回调用于执行，然后重新调度
                            if let Some(callback) = entry.callback.take() {
                                to_reschedule.push((
                                    entry.interval,
                                    callback,
                                    Arc::clone(&entry.deleted),
                                ));
                            }
                        }
                    }
                    to_remove.push(*time);
                }
            }

            // 清理已删除的条目
            for time in &to_remove {
                entries
                    .entry(*time)
                    .and_modify(|vec| vec.retain(|e| !e.deleted.load(Ordering::Relaxed)));
            }

            // 清理空条目
            entries.retain(|_, vec| !vec.is_empty());
        }

        // 执行回调并重新调度
        for (interval, callback, deleted) in to_reschedule {
            // 执行回调
            callback();

            // 重新调度 - 只有未标记删除时才重新调度
            if !deleted.load(Ordering::Relaxed) {
                let mut entries = self.entries.lock().expect("Mutex poisoned");
                let new_time = Instant::now() + interval;
                let new_entry = TimerEntry {
                    callback: Some(callback),
                    interval,
                    deleted,
                };
                entries.entry(new_time).or_default().push(new_entry);
            }
        }
    }

    /// 获取下一个定时器到期时间
    pub fn next_timeout(&self) -> Option<Duration> {
        let entries = self.entries.lock().expect("Mutex poisoned");
        entries.keys().next().map(|time| {
            let now = Instant::now();
            if *time > now {
                time.duration_since(now)
            } else {
                Duration::ZERO
            }
        })
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}
