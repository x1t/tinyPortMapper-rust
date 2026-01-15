//! 信号处理模块
//!
//! 处理 SIGPIPE、SIGTERM、SIGINT 等信号
//! 使用原始 libc 调用，避免 signal_hook 库的兼容性问题

use crate::info;
use libc::{SIGINT, SIGPIPE, SIGTERM, SIG_DFL};
use std::io::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// 信号处理器
#[derive(Debug, Clone)]
pub struct SignalHandler {
    /// 运行标志
    running: Arc<AtomicBool>,
}

impl SignalHandler {
    /// 创建新的信号处理器
    pub fn new() -> Result<Self, Error> {
        let running = Arc::new(AtomicBool::new(true));

        // Spawn signal handling thread
        {
            let running = Arc::clone(&running);
            std::thread::spawn(move || {
                // 只处理 SIGTERM 和 SIGINT（与 C++ 版本保持一致）
                info!("[signal] signal handler started");

                // 设置信号处理函数
                unsafe {
                    libc::signal(SIGPIPE, SIG_DFL);
                }

                // 使用简单的信号等待机制
                let mut sigset: libc::sigset_t = unsafe { std::mem::zeroed() };
                unsafe {
                    libc::sigemptyset(&mut sigset);
                    libc::sigaddset(&mut sigset, SIGTERM);
                    libc::sigaddset(&mut sigset, SIGINT);
                    libc::pthread_sigmask(libc::SIG_BLOCK, &sigset, std::ptr::null_mut());
                }

                loop {
                    let mut sig: libc::c_int = 0;
                    let ret = unsafe { libc::sigwait(&sigset, &mut sig) };

                    if ret != 0 {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        continue;
                    }

                    match sig {
                        SIGPIPE => {
                            // 忽略 SIGPIPE
                            info!("[signal] got sigpipe, ignored");
                        }
                        SIGTERM | SIGINT => {
                            let sig_name = if sig == SIGTERM { "sigterm" } else { "sigint" };
                            info!("[signal] got {}, exit", sig_name);
                            running.store(false, Ordering::Relaxed);
                            break;
                        }
                        _ => {
                            info!("[signal] got unknown signal: {}", sig);
                        }
                    }
                }

                info!("[signal] signal handler thread exiting");
            });
        }

        Ok(Self { running })
    }

    /// 注册信号处理
    pub fn register(&self) -> Result<(), Error> {
        Ok(())
    }

    /// 检查是否仍在运行
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// 停止运行
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }
}
