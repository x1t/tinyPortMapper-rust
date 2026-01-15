# CLAUDE.md

本文档为 Claude Code (claude.ai/code) 提供代码库操作指导。

## 项目概述

**tinyPortMapper** 是一款轻量级、高性能的端口转发工具，基于 Rust + mio 事件驱动框架重写。性能可达 ~95 Gbps，支持 TCP/UDP 转发、IPv4/IPv6 地址转换及 4-to-6 / 6-to-4 地址翻译，具备内存安全保障。

## 构建命令

```bash
# 本地构建
make                    # Release 构建 (默认)
make debug              # Debug 构建 (含详细日志)
make fast               # 快速构建 (无优化)

# musl 静态链接 (适用于 Alpine Linux)
make musl               # x86_64-unknown-linux-musl

# 交叉编译 (与 C++ 版本对齐)
make arm                # armv7-unknown-linux-gnueabihf
make amd64              # x86_64-unknown-linux-gnu
make mips24kc_be        # mips-unknown-linux-gnu (大端)
make mips24kc_le        # mipsel-unknown-linux-gnu (小端)
make x86                # i686-unknown-linux-gnu

# 其他平台
make mingw              # Windows x86_64-pc-windows-gnu
make macos              # macOS x86_64-apple-darwin

# 代码质量
make test               # cargo test --release
make check              # cargo check (原生 + musl 目标)
make clippy             # cargo clippy
make fmt                # cargo fmt --all
```

## 架构设计

```
src/
├── main.rs              # 入口点、CLI 参数解析、socket 创建
├── lib.rs               # 核心库导出、日志宏、工具函数
├── config.rs            # 配置结构体与常量定义
├── fd_manager.rs        # 文件描述符管理 (Fd64 ↔ RawFd)
├── log.rs               # 七级日志系统
├── lru.rs               # LRU 清理器
├── stats.rs             # 流量统计 (原子计数器)
├── build.rs             # 构建脚本 (git 版本信息)
├── types/
│   └── address.rs       # 地址类型 (IPv4/IPv6)
├── event/
│   ├── mod.rs           # 事件循环主控制器、TokenManager
│   ├── tcp.rs           # TCP 连接处理器
│   ├── udp.rs           # UDP 数据包处理器
│   ├── timer.rs         # 定时器
│   └── signals.rs       # 信号处理 (SIGTERM/SIGINT)
├── connection/
│   └── mod.rs           # 连接数据结构 (TcpConnection, UdpSession)
└── manager/
    └── mod.rs           # 连接管理器 (TcpConnectionManager, UdpSessionManager)
```

### 核心数据流

```
main():
  │
  ├─ init_ws()              # Windows WSA 初始化
  ├─ CLI 参数解析 (clap)    # Args 结构体
  ├─ 创建 Config
  ├─ 创建 FdManager / TcpConnectionManager / UdpSessionManager
  ├─ 创建 TCP/UDP 监听 socket (含 SO_REUSEADDR/SO_REUSEPORT)
  └─ EventLoop::run()       # 主事件循环
        │
        ├─ poll.poll()      # 等待 I/O 事件
        ├─ TCP 监听 → TcpHandler::on_accept() → 创建 TcpConnection
        ├─ UDP 监听 → UdpHandler::on_datagram() → 创建/查找 UdpSession
        ├─ 连接 I/O → TcpHandler::on_read/on_write()
        ├─ UDP 响应 → UdpHandler::on_response()
        ├─ 定时器 → 每 10s 输出统计信息
        └─ LRU 清理 → 每 400ms 清理超时连接
```

### TCP 转发流程 (event/tcp.rs)

```
on_accept():
  1. accept() 接受客户端连接
  2. 根据 FwdType 选择目标地址类型
  3. 创建远程 socket 并发起非阻塞 connect()
  4. 创建 TcpConnection (local/remote 两个 TcpEndpoint)
  5. 注册 local socket 为 READABLE

on_read():
  1. 从 local/remote socket 读取数据
  2. recv_len == 0 → 对端关闭，清理连接
  3. recv_len < 0 → 检查 WouldBlock
  4. 数据存入 TcpEndpoint.data 缓冲区
  5. send() 到对端 socket
  6. 未发送完则注册 WRITABLE 事件

on_write():
  1. 发送 pending 数据
  2. 连接完成时更新状态
  3. 处理缓冲区满 (WouldBlock)
```

### UDP 转发流程 (event/udp.rs)

```
on_datagram():
  1. 从监听 socket recvfrom() 接收数据
  2. 根据源地址查找或创建 UdpSession
  3. 创建已连接 UDP socket (connect 到远程)
  4. send() 数据到远程
  5. 注册 remote socket 为 READABLE

on_response():
  1. 从 remote socket recv() 接收响应
  2. 通过 fd64_to_addr 映射查找 UdpSession
  3. 通过监听 socket sendto() 发送到客户端
```

## 关键数据结构

### 地址类型 (types/address.rs)

```rust
pub struct Address {
    addr: SocketAddr,  // 内部使用 std::net::SocketAddr
}

// 格式支持：
// - IPv4: "1.2.3.4:443"
// - IPv6: "[::1]:443" (必须带方括号)
// - 4to6 翻译: to_ipv4_mapped_ipv6() → ::ffff:x.x.x.x
// - 6to4 翻译: from_ipv4_mapped_ipv6() → 提取 IPv4
```

### FD64 抽象 (fd_manager.rs)

```rust
pub struct Fd64(pub u64);  // 跨平台 fd 抽象

pub struct FdManager {
    fd_to_fd64: HashMap<RawFd, Fd64>,     // RawFd → Fd64
    fd64_to_fd: HashMap<Fd64, RawFd>,     // Fd64 → RawFd
    fd_info: HashMap<Fd64, FdInfo>,       // FD 元数据 (创建时间、活跃时间)
    counter: AtomicU64,                    // Fd64 生成器
}

// 用途：
// - 32 位兼容性 (Windows sockets)
// - Socket 生命周期内保持标识稳定
// - 连接管理器 O(1) 查找
```

### TCP 连接 (connection/mod.rs)

```rust
pub struct TcpEndpoint {
    fd64: Fd64,           // 关联的 Fd64
    data: Vec<u8>,        // 接收缓冲区 (预分配)
    begin: usize,         // 缓冲区起始位置
    data_len: usize,      // 有效数据长度
}

pub struct TcpConnection {
    local: TcpEndpoint,               // 客户端连接端
    remote: TcpEndpoint,              // 远程服务器端
    addr_s: String,                   // 客户端地址字符串
    create_time: u64,                 // 创建时间戳
    last_active_time: Arc<AtomicU64>, // 最后活跃时间
    remote_connecting: bool,          // 远程连接是否还在建立中
}
```

### UDP 会话 (connection/mod.rs)

```rust
pub struct UdpSession {
    address: Address,                 // 客户端地址 (会话唯一标识)
    fd64: Fd64,                       // 已连接 UDP socket
    local_listen_fd: Fd64,            // 监听 socket
    addr_s: String,                   // 地址字符串
    create_time: u64,
    last_active_time: Arc<AtomicU64>,
}
```

## 连接生命周期管理

### TCP 连接管理器 (manager/mod.rs)

```rust
pub struct TcpConnectionManager {
    connections: HashMap<Fd64, Arc<RwLock<TcpConnection>>>,
    lru: LruCollector<Fd64, Fd64>,    // 按活跃时间排序
    timeout: Duration,                 // 默认 360s
    conn_clear_ratio: u32,             // 每次清理比例 (1/30)
    conn_clear_min: u32,               // 最小清理数量 (1)
}

impl TcpConnectionManager {
    pub fn clear_inactive(&self) {
        // 1. 计算需清理数量: size / ratio + min
        // 2. 查找所有超时的连接
        // 3. 按最后活跃时间排序
        // 4. 清理最旧的 num_to_clean 个连接
    }
}
```

### UDP 会话管理器 (manager/mod.rs)

```rust
pub struct UdpSessionManager {
    sessions: HashMap<Address, Arc<RwLock<UdpSession>>>,  // 按客户端地址索引
    fd64_to_addr: HashMap<Fd64, Address>,                 // fd64 → 地址 (O(1) 查找)
    lru: LruCollector<Address, Address>,                  // 按地址活跃时间排序
    timeout: Duration,                                    // 默认 180s
}
```

### LRU 清理器 (lru.rs)

```rust
pub struct LruCollector<K, T> {
    values: HashMap<K, T>,            // 键值存储
    access_times: HashMap<K, u64>,    // 访问时间
    time_list: Vec<K>,                // 时间排序的键
    min_heap: Vec<(u64, K)>,          // 最小堆快速获取最旧元素
}

// 支持操作：
// - new_key()   添加新条目
// - update()    更新访问时间
// - erase()     删除条目
// - cleanup_timeout() 清理超时条目
// - peek_back() 获取最旧条目
```

## 配置常量 (config.rs)

| 常量 | 值 | 说明 |
|------|-----|------|
| `LISTEN_FD_BUF_SIZE` | 2MB | 监听 socket 缓冲区 |
| `TIMER_INTERVAL_MS` | 400ms | 定时器间隔 (清理检查) |
| `MAX_DATA_LEN_UDP` | 65536 | UDP 最大负载 |
| `MAX_DATA_LEN_TCP` | 16384 | TCP 缓冲区大小 |
| `DEFAULT_MAX_CONNECTIONS` | 20000 | 最大连接数 |
| `DEFAULT_TCP_TIMEOUT_MS` | 360000ms | TCP 连接超时 (6分钟) |
| `DEFAULT_UDP_TIMEOUT_MS` | 180000ms | UDP 会话超时 (3分钟) |
| `DEFAULT_CONN_CLEAR_RATIO` | 30 | 清理比例 |
| `DEFAULT_CONN_CLEAR_MIN` | 1 | 最小清理数量 |

## 日志宏 (lib.rs)

```rust
info!("message");    // 级别 4 (默认)
debug!("message");   // 级别 5
trace!("message");   // 级别 6
warn!("message");    // 级别 3
error!("message");   // 级别 2
fatal!("message");   // 级别 1 (设置 about_to_exit 标志)
log_bare!("raw");    // 无时间戳/级别前缀
```

启用方式：`--log-level <0-6>` 或 `--log-level fatal|error|warn|info|debug|trace`

## 性能优化

| 优化点 | 实现方式 |
|--------|----------|
| 零拷贝设计 | 预分配缓冲区，数据直接从 socket 读取后发送到对端 |
| 非阻塞 I/O | 所有 socket 设置 O_NONBLOCK |
| 批量事件处理 | mio poll 批量返回就绪事件 |
| O(1) 查找 | UDP 通过 fd64_to_addr 映射实现快速查找 |
| 原子操作 | 统计信息使用 AtomicU64 |
| 锁优化 | 使用 RwLock 保护共享数据 |
| 静态链接 | Release 构建使用 musl 静态链接 |

## 跨平台支持

| 平台 | 特殊处理 |
|------|----------|
| Windows | WSAStartup/WSACleanup、closesocket、RawSocket |
| Linux | SO_BINDTODEVICE (-e 参数)、SO_REUSEPORT |
| Unix | RawFd、信号处理 |

## 转发类型 (config.rs)

```rust
pub enum FwdType {
    Normal,        // 普通转发 (保持原地址类型)
    FwdType4to6,   // IPv4 → IPv6 (::ffff:x.x.x.x)
    FwdType6to4,   // IPv6 → IPv4
}
```

## 使用示例

```bash
# TCP + UDP 转发
./tinymapper -l0.0.0.0:1234 -r10.222.2.1:443 -t -u

# TCP 转发
./tinymapper -l0.0.0.0:1234 -r10.222.2.1:443 -t

# IPv6
./tinymapper -l[::]:1234 -r[2001:19f0:7001:1111::1]:443 -t -u

# 4to6 翻译
./tinymapper -l0.0.0.0:1234 -r[2001:19f0:7001:1111::1]:443 -t -u -4

# 绑定网络接口 (Linux)
./tinymapper -l:1234 -r:443 -t -u -e eth0

# 自定义缓冲区和日志
./tinymapper -l:1234 -r:443 -t -u --sock-buf 2048 --log-level debug --log-position

# 单元测试
./tinymapper --run-test
```

## 注意事项

- **静态链接**: musl 构建约 901KB (C++ 版本 2.1MB)
- **Git 版本**: 通过 build.rs 自动生成，记录 commit hash 和构建日期
- **调试模式**: 启用 `my_debug` feature 简化日志 (无时间戳/位置)
- **信号处理**: 独立线程等待 SIGTERM/SIGINT，忽略 SIGPIPE
