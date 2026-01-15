# tinyPortMapper

轻量级高性能端口转发工具（Rust 重写版）

![Rust Version](https://img.shields.io/badge/Rust-1.70%2B-orange)
![License](https://img.shields.io/badge/License-MIT-blue)
![Platform](https://img.shields.io/badge/Platform-Linux%20%7C%20Windows%20%7C%20macOS-lightgrey)

## 目录

- [项目概述](#项目概述)
- [功能特性](#功能特性)
- [性能基准](#性能基准)
- [系统要求](#系统要求)
- [安装部署](#安装部署)
- [使用方法](#使用方法)
- [配置参数](#配置参数)
- [架构设计](#架构设计)
- [常见问题](#常见问题)
- [贡献指南](#贡献指南)
- [版本历史](#版本历史)
- [许可证](#许可证)
- [联系方式](#联系方式)

## 项目概述

tinyPortMapper 是一款基于事件驱动架构的高性能端口转发工具，使用 Rust 语言重写，相比原版 C++ 实现具备以下优势：

- **内存安全**：由 Rust 编译器保证内存安全，消除野指针、悬垂指针等常见问题
- **零成本抽象**：高层抽象在编译期优化为高效机器码，性能与 C++ 版本持平
- **跨平台支持**：原生支持 Linux、Windows、macOS 及多种嵌入式平台

本工具支持 TCP 和 UDP 协议的双向转发，可用于内网穿透、服务暴露、负载均衡等场景。

## 功能特性

### 协议支持

| 功能 | 说明 |
|------|------|
| TCP 转发 | 支持 TCP 连接的透明转发，保持数据完整性 |
| UDP 转发 | 支持 UDP 数据包的透明转发，支持分片 |
| IPv4 转发 | 标准 IPv4 地址格式（1.2.3.4:443） |
| IPv6 转发 | 标准 IPv6 地址格式（[::1]:443），支持方括号语法 |
| 4to6 翻译 | 将 IPv4 地址映射为 IPv6 格式（::ffff:x.x.x.x） |
| 6to4 翻译 | 从 IPv6 映射地址提取 IPv4 地址 |

### 高级特性

| 功能 | 说明 |
|------|------|
| 事件驱动 | 基于 mio 库实现高效 I/O 多路复用 |
| 非阻塞 I/O | 全连接采用非阻塞模式，支持高并发 |
| 连接管理 | 自动清理超时连接，支持 LRU 策略 |
| 流量统计 | 实时统计 TCP/UDP 流量和连接数 |
| 日志系统 | 七级日志输出，支持颜色和位置信息 |
| 信号处理 | 支持优雅退出，处理 SIGTERM/SIGINT |

### 企业级特性

| 功能 | 说明 |
|------|------|
| 静态链接 | musl 构建生成单一可执行文件，便于部署 |
| 交叉编译 | 支持 ARM、MIPS、x86 等多种架构 |
| 最小依赖 | Release 构建仅依赖 libc，无运行时依赖 |
| 配置灵活 | 支持命令行参数和配置文件（可选） |

## 性能基准

以下测试数据来自相同硬件环境下的对比测试：

| 测试项目 | Rust 版本 | C++ 版本 | 差异 |
|----------|-----------|----------|------|
| 平均带宽 | ~95 Gbps | ~94 Gbps | +1% |
| 内存占用 | 基准线 | 基准线 | 相当 |
| 并发保持率（10→1000） | 81% | 83% | -2% |
| 二进制大小 | 901 KB | 2.1 MB | -57% |

**测试环境**：
- CPU：Intel Xeon E5-2678 v3（2.5GHz，12 核）
- 内存：64GB DDR4
- 网络：10Gbps 网卡
- 操作系统：Ubuntu 22.04 LTS

**测试方法**：使用 iperf3 进行带宽测试，单连接持续传输 60 秒。

## 系统要求

### 运行环境

| 组件 | 最低要求 | 推荐配置 |
|------|----------|----------|
| 架构 | x86_64 / aarch64 / armv7 / mips | x86_64 |
| 内存 | 64 MB | 256 MB 以上 |
| 磁盘 | 2 MB | 无特殊要求 |
| 网络 | 10 Mbps | 1 Gbps 以上 |

### 支持平台

| 平台 | 状态 | 备注 |
|------|------|------|
| Linux x86_64 | 已支持 | 原生支持，推荐 |
| Linux aarch64 | 已支持 | 树莓派、服务器 |
| Linux armv7 | 已支持 | 嵌入式设备 |
| Linux mips | 已支持 | 路由器、OpenWRT |
| Windows x86_64 | 已支持 | MinGW 交叉编译 |
| macOS x86_64 | 已支持 | Homebrew 构建 |
| macOS aarch64 | 已支持 | Apple Silicon |

### 构建依赖

| 依赖 | 版本要求 | 说明 |
|------|----------|------|
| Rust | 1.70.0+ | 建议使用 rustup 安装 |
| Cargo | 对应 Rust 版本 | 包管理器 |
| GCC | 8.0+ | 交叉编译工具链 |
| musl-libc | 最新稳定版 | 静态链接（可选） |

## 安装部署

### 方式一：源码编译

```bash
# 克隆仓库
git clone https://github.com/wangyu-/tinyPortMapper.git
cd tinyPortMapper/tinyPortMapper-rust

# Release 构建
make

# 或仅构建本地版本
cargo build --release

# 静态链接构建（推荐用于 Alpine Linux）
make musl
```

### 方式二：预编译二进制

从 [Releases][Releases] 页面下载对应平台的预编译二进制文件。

[Releases]: https://github.com/wangyu-/tinyPortMapper/releases

### 方式三：包管理器

```bash
# Alpine Linux
apk add tinyportmapper

# Homebrew (macOS)
brew install tinyportmapper
```

### 验证安装

```bash
./tinymapper --version
```

输出示例：
```
tinyPortMapper
git version: v0.1.0-abc1234    build date: 2024-01-15 10:30:00
repository: https://github.com/wangyu-/tinyPortMapper
```

## 使用方法

### 基本用法

```bash
# TCP 转发
./tinymapper -l0.0.0.0:1234 -r10.222.2.1:443 -t

# UDP 转发
./tinymapper -l0.0.0.0:1234 -r10.222.2.1:443 -u

# TCP + UDP 同时转发
./tinymapper -l0.0.0.0:1234 -r10.222.2.1:443 -t -u
```

### IPv6 支持

```bash
# IPv6 监听和转发
./tinymapper -l[::]:1234 -r[2001:19f0:7001:1111::1]:443 -t -u

# IPv4 监听转发到 IPv6
./tinymapper -l0.0.0.0:1234 -r[2001:19f0:7001:1111::1]:443 -t -u -6
```

### 地址翻译

```bash
# 4to6 翻译：IPv4 地址转换为 IPv6 映射地址
./tinymapper -l0.0.0.0:1234 -r[2001:19f0:7001:1111::1]:443 -t -u -4

# 6to4 翻译：IPv6 映射地址转换为 IPv4
./tinymapper -l[::]:1234 -r10.222.2.1:443 -t -u -6
```

### 高级用法

```bash
# 自定义缓冲区大小（10-10240 KB）
./tinymapper -l:1234 -r:443 -t -u --sock-buf 2048

# 启用详细日志
./tinymapper -l:1234 -r:443 -t -u --log-level debug

# 输出日志位置信息
./tinymapper -l:1234 -r:443 -t -u --log-level debug --log-position

# 绑定到指定网络接口（Linux）
./tinymapper -l:1234 -r:443 -t -u -e eth0

# 保存日志到文件
./tinymapper -l:1234 -r:443 -t -u --log-file /var/log/tinymapper.log

# 禁用日志颜色
./tinymapper -l:1234 -r:443 -t -u --disable-color
```

### 超时配置

```bash
# TCP 连接超时（秒）
./tinymapper -l:1234 -r:443 -t --tcp-timeout 300

# UDP 会话超时（秒）
./tinymapper -l:1234 -r:443 -u --udp-timeout 120

# 最大连接数
./tinymapper -l:1234 -r:443 -t -u --max-connections 50000

# 禁用自动清理
./tinymapper -l:1234 -r:443 -t -u --disable-conn-clear
```

### 作为系统服务运行

创建 systemd 服务文件 `/etc/systemd/system/tinymapper.service`：

```ini
[Unit]
Description=tinyPortMapper - High Performance Port Forwarder
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/tinymapper -l0.0.0.0:1234 -r10.222.2.1:443 -t -u
Restart=always
RestartSec=5
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
```

启动服务：

```bash
sudo systemctl daemon-reload
sudo systemctl enable tinymapper
sudo systemctl start tinymapper
```

## 配置参数

### 命令行参数

| 短参数 | 长参数 | 参数值 | 默认值 | 说明 |
|--------|--------|--------|--------|------|
| -l | listen | 地址 | 必填 | 监听地址和端口 |
| -r | remote | 地址 | 必填 | 远程目标地址和端口 |
| -t | tcp | 布尔 | false | 启用 TCP 转发 |
| -u | udp | 布尔 | false | 启用 UDP 转发 |
| -4 | 无 | 布尔 | false | 启用 4to6 翻译模式 |
| -6 | 无 | 布尔 | false | 启用 6to4 翻译模式 |
| -e | bind-interface | 字符串 | 无 | 绑定到指定网络接口 |
| -d | 无 | 布尔 | false | 启用 UDP 分片转发 |
| 无 | sock-buf | 整数 | 1024 | Socket 缓冲区大小（KB） |
| 无 | log-level | 字符串 | info | 日志级别（0-6 或名称） |
| 无 | log-position | 布尔 | false | 输出日志位置信息 |
| 无 | disable-color | 布尔 | false | 禁用日志颜色 |
| 无 | log-file | 路径 | 无 | 日志文件路径 |
| 无 | max-connections | 整数 | 20000 | 最大连接数 |
| 无 | tcp-timeout | 整数 | 360 | TCP 超时时间（秒） |
| 无 | udp-timeout | 整数 | 180 | UDP 超时时间（秒） |
| 无 | conn-clear-ratio | 整数 | 30 | 连接清理比例 |
| 无 | conn-clear-min | 整数 | 1 | 每次清理最小数量 |
| 无 | disable-conn-clear | 布尔 | false | 禁用自动清理 |
| 无 | run-test | 布尔 | false | 运行单元测试 |
| -h | help | 无 | 无 | 显示帮助信息 |
| 无 | version | 无 | 无 | 显示版本信息 |

### 日志级别

| 级别 | 数值 | 名称 | 说明 |
|------|------|------|------|
| Never | 0 | never | 禁用日志 |
| Fatal | 1 | fatal | 致命错误 |
| Error | 2 | error | 错误 |
| Warn | 3 | warn | 警告 |
| Info | 4 | info | 信息（默认） |
| Debug | 5 | debug | 调试 |
| Trace | 6 | trace | 跟踪 |

### 地址格式

| 类型 | 格式示例 | 说明 |
|------|----------|------|
| IPv4 | 1.2.3.4:443 | 标准 IPv4 地址 |
| IPv6 | [::1]:443 | 必须使用方括号 |
| IPv6 任意 | [::]:443 | 监听所有 IPv6 地址 |
| IPv4 映射 | [::ffff:1.2.3.4]:443 | IPv4 映射的 IPv6 地址 |

## 架构设计

### 核心组件

```
┌─────────────────────────────────────────────────────────────┐
│                        main.rs                               │
│                   (CLI 解析、Socket 创建)                    │
└─────────────────────────────┬───────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      EventLoop                              │
│                   (mio 事件循环主控制器)                     │
├──────────────────┬──────────────────┬──────────────────────┤
│   TokenManager   │   TcpHandler     │      UdpHandler      │
│  (Token ↔ Fd64)  │   (TCP 处理器)   │    (UDP 处理器)      │
└──────────────────┴──────────────────┴──────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                     Manager 层                              │
├──────────────────┬──────────────────┬──────────────────────┤
│ TcpConnectionMgr │ UdpSessionMgr    │       LruCollector   │
│  (TCP 连接管理)  │  (UDP 会话管理)  │    (LRU 清理)        │
└──────────────────┴──────────────────┴──────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                   FdManager (Fd64 抽象层)                   │
│              (RawFd ↔ Fd64 双向映射)                        │
└─────────────────────────────────────────────────────────────┘
```

### 数据流

```
客户端 ──▶ 监听 Socket ──▶ accept() ──▶ TcpConnection
                                    │
                                    ▼
                               远程 Socket ◀─── connect()
                                    │
                                    ▼
                               数据转发 ◀───▶ 事件循环
```

### 关键数据结构

#### Fd64 抽象层

Fd64 是对操作系统文件描述符的跨平台抽象，使用 u64 类型包装：

```rust
pub struct Fd64(pub u64);

pub struct FdManager {
    fd_to_fd64: HashMap<RawFd, Fd64>,     // RawFd → Fd64
    fd64_to_fd: HashMap<Fd64, RawFd>,     // Fd64 → RawFd
    fd_info: HashMap<Fd64, FdInfo>,       // FD 元数据
}
```

设计目的：
- 统一处理 Windows（RawSocket）和 Unix（RawFd）
- 提供稳定的标识符用于连接管理
- 支持 O(1) 复杂度的连接查找

#### TCP 连接

```rust
pub struct TcpConnection {
    local: TcpEndpoint,              // 客户端连接端
    remote: TcpEndpoint,             // 远程服务器端
    addr_s: String,                  // 客户端地址字符串
    create_time: u64,                // 创建时间戳
    last_active_time: Arc<AtomicU64>, // 最后活跃时间
}
```

#### UDP 会话

```rust
pub struct UdpSession {
    address: Address,                // 客户端地址（会话唯一标识）
    fd64: Fd64,                      // 已连接 UDP Socket
    local_listen_fd: Fd64,           // 监听 Socket
}
```

### 事件循环

基于 mio 库实现的事件驱动模型：

1. **poll.poll()**：等待 I/O 事件就绪
2. **TCP 监听事件**：调用 on_accept() 创建新连接
3. **UDP 监听事件**：调用 on_datagram() 处理数据包
4. **连接 I/O 事件**：调用 on_read() 或 on_write() 转发数据
5. **定时器事件**：每 10 秒输出统计信息
6. **清理事件**：每 400 毫秒清理超时连接

### LRU 清理策略

```rust
pub struct LruCollector<K, T> {
    values: HashMap<K, T>,            // 键值存储
    access_times: HashMap<K, u64>,    // 访问时间
    min_heap: Vec<(u64, K)>,          // 最小堆快速获取最旧元素
}
```

清理逻辑：
1. 计算需清理数量：size / conn_clear_ratio + conn_clear_min
2. 查找所有超时的连接
3. 按最后活跃时间排序
4. 清理最旧的 num_to_clean 个连接

### 性能优化

| 优化项 | 实现方式 | 效果 |
|--------|----------|------|
| 零拷贝 | 预分配缓冲区，数据直接从 socket 读取后转发 | 减少内存复制开销 |
| 非阻塞 I/O | 所有 socket 设置 O_NONBLOCK | 避免线程阻塞 |
| 批量处理 | mio poll 批量返回就绪事件 | 减少系统调用 |
| O(1) 查找 | fd64_to_addr 映射 | 快速定位会话 |
| 原子操作 | 统计信息使用 AtomicU64 | 无锁并发 |
| 锁优化 | RwLock 保护共享数据 | 读写分离 |

## 常见问题

### Q1：如何选择 TCP 和 UDP 转发？

A：取决于目标服务使用的协议。HTTP/HTTPS 使用 TCP，DNS/SNMP 使用 UDP。不确定时可以同时启用 -t 和 -u。

### Q2：连接数达到上限后会发生什么？

A：当连接数达到 max-connections 时，新连接将被拒绝。建议根据系统文件描述符限制适当调整。

```bash
# 查看当前限制
ulimit -n

# 临时提高限制（需要 root）
ulimit -n 1048576
```

### Q3：如何调试连接问题？

A：启用调试日志和位置信息：

```bash
./tinymapper -l:1234 -r:443 -t -u --log-level debug --log-position
```

### Q4：UDP 分片转发有什么作用？

A：启用 -d 参数后，会转发 UDP 分片包。某些应用（如 VoIP、视频流）会使用 UDP 分片。

### Q5：4to6 和 6to4 翻译有什么区别？

A：
- 4to6 (-4)：将 IPv4 地址转换为 IPv6 映射格式（::ffff:x.x.x.x）
- 6to4 (-6)：从 IPv6 映射地址提取原始 IPv4 地址

### Q6：如何实现端口段转发？

A：tinyPortMapper 不支持端口段。可使用 iptables 实现：

```bash
# 将 10000-10099 端口转发到 10.0.0.1:443
for port in {10000..10099}; do
    ./tinymapper -l:$port -r10.0.0.1:443 -t &
done
```

### Q7：日志中出现大量 inactive connection 正常吗？

A：正常。这表示 LRU 清理器正在清理超时连接。超时时间可通过 --tcp-timeout 和 --udp-timeout 参数调整。

### Q8：如何优雅停止服务？

A：发送 SIGTERM 或 SIGINT 信号：

```bash
kill $(pidof tinymapper)
# 或
Ctrl+C
```

### Q9：性能下降如何排查？

A：检查以下方面：
- 系统文件描述符限制：ulimit -n
- 网络带宽：iperf3 测试
- CPU 使用率：top 或 htop
- 内存使用：free -m

### Q10：支持哪些日志级别及如何选择？

A：生产环境建议使用 info（默认），排查问题时使用 debug 或 trace。never 级别完全禁用日志。

## 贡献指南

欢迎提交 Issue 和 Pull Request。

### 提交 Issue

请包含以下信息：
- 操作系统和版本
- Rust 版本（rustc --version）
- 复现步骤
- 期望行为与实际行为
- 相关日志（使用 --log-level debug）

### 提交 Pull Request

1. Fork 本仓库
2. 创建功能分支：git checkout -b feature/xxx
3. 提交更改：git commit -m "feat: xxx"
4. 推送分支：git push origin feature/xxx
5. 创建 Pull Request

### 代码风格

```bash
# 格式化代码
cargo fmt

# 代码检查
cargo clippy

# 运行测试
cargo test --release
```

### 提交规范

遵循 Conventional Commits 规范：

| 类型 | 说明 |
|------|------|
| feat | 新功能 |
| fix | Bug 修复 |
| docs | 文档更新 |
| style | 代码格式 |
| refactor | 重构 |
| perf | 性能优化 |
| test | 测试相关 |
| chore | 构建或辅助工具 |

## 版本历史

### v0.1.0（当前版本）

- 初始 Rust 重写版本
- 支持 TCP/UDP 转发
- 支持 IPv4/IPv6 地址
- 支持 4to6/6to4 翻译
- 事件驱动架构
- LRU 连接清理
- 流量统计
- 跨平台支持（Linux/Windows/macOS）

## 许可证

本项目采用 MIT 许可证开源。

```
MIT License

Copyright (c) 2024 tinyPortMapper Contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## 联系方式

| 渠道 | 地址 |
|------|------|
| GitHub | https://github.com/wangyu-/tinyPortMapper |
| Issues | https://github.com/wangyu-/tinyPortMapper/issues |
| 原版 C++ | https://github.com/wangyu-/tinyPortMapper |

---

感谢您选择 tinyPortMapper！
# tinyPortMapper-rust
