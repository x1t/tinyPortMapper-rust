# tinyPortMapper-rust

轻量级高性能端口转发工具（Rust 重写版）

[![Rust Version](https://img.shields.io/badge/Rust-1.70+-orange)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/License-MIT-blue)](LICENSE)
[![Platform](https://img.shields.io/badge/Platform-Linux%20%7C%20Windows%20%7C%20macOS-lightgrey)](#支持平台)

基于事件驱动架构的高性能端口转发工具，使用 Rust 语言重写 C++ 原版。

## 功能特性

### 协议支持

| 功能 | 说明 |
|------|------|
| TCP 转发 | 支持 TCP 连接的透明转发，保持数据完整性 |
| UDP 转发 | 支持 UDP 数据包的透明转发，支持分片 |
| IPv4/IPv6 | 标准地址格式，支持方括号语法 |
| 地址翻译 | 4to6/6to4 地址转换 |

### 核心特性

- **事件驱动 I/O**: 基于 mio 库实现高效事件循环
- **零拷贝转发**: Linux splice() 系统调用，支持 recv/send 回退
- **连接管理**: LRU 超时清理，TCP 360s / UDP 180s 超时
- **流量统计**: 实时显示 TCP/UDP 带宽和连接数
- **七级日志**: never/fatal/error/warn/info/debug/trace
- **优雅退出**: SIGTERM/SIGINT 信号处理

### 平台支持

| 平台 | 架构 | 说明 |
|------|------|------|
| Linux | x86_64/aarch64/armv7/mips | 原生支持，推荐使用 musl 静态构建 |
| Windows | x86_64/i686 | MinGW 交叉编译 |
| macOS | x86_64/aarch64 | Homebrew 构建 |
| OpenWRT | 多架构 | ARM/MIPS/x86 目标支持 |

## 性能指标

- **TCP 转发带宽**: ~29 Gbps (iperf3 实测)
- **UDP 转发带宽**: 满速率转发 (iperf3 实测，4Gbps+ 无丢包)
- **内存占用**: 极低，无运行时依赖
- **二进制大小**: ~900 KB（静态链接）
- **并发连接**: 20000+（可配置）

### 性能测试 (iperf3)

在本地回环环境下进行压力测试的结果：

| 协议 | 直接连接 (基准) | 转发连接 (TPM) | 效率 | 备注 |
|------|----------------|---------------|------|------|
| **TCP** | 89.21 Gbps | **28.91 Gbps** | ~32% | 高吞吐量转发 |
| **UDP** | 4.00 Gbps | **4.00 Gbps** | 100% | 0% 丢包，极低抖动 |

## 快速开始

### 源码编译

```bash
# 克隆仓库
git clone https://github.com/wangyu-/tinyPortMapper.git
cd tinyPortMapper/tinyPortMapper-rust

# Release 构建（默认）
make

# Debug 构建
make debug

# musl 静态链接构建（推荐）
make musl
```

### 预编译下载

从 [Releases](https://github.com/wangyu-/tinyPortMapper/releases) 页面下载对应平台的二进制文件。

### 验证安装

```bash
./tinymapper --version
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

# IPv4 监听转发到 IPv6 目标
./tinymapper -l0.0.0.0:1234 -r[2001:19f0:7001::1]:443 -t -u -6
```

### 地址翻译

```bash
# 4to6 翻译：IPv4 → ::ffff:x.x.x.x
./tinymapper -l0.0.0.0:1234 -r[2001:19f0:7001::1]:443 -t -u -4

# 6to4 翻译：从 IPv6 映射地址提取 IPv4
./tinymapper -l[::]:1234 -r10.222.2.1:443 -t -u -6
```

### 高级选项

```bash
# 自定义缓冲区大小（10-10240 KB）
./tinymapper -l:1234 -r:443 -t -u --sock-buf 2048

# 启用调试日志
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
```

## 命令行参数

| 短参数 | 长参数 | 默认值 | 说明 |
|--------|--------|--------|------|
| -l | listen | 必填 | 监听地址和端口 |
| -r | remote | 必填 | 远程目标地址和端口 |
| -t | tcp | false | 启用 TCP 转发 |
| -u | udp | false | 启用 UDP 转发 |
| -4 | - | false | 启用 4to6 翻译 |
| -6 | - | false | 启用 6to4 翻译 |
| -e | bind-interface | - | 绑定网络接口 |
| -d | - | false | 启用 UDP 分片 |
| - | sock-buf | 1024 | 缓冲区大小（KB） |
| - | log-level | info | 日志级别 |
| - | log-position | false | 输出位置信息 |
| - | disable-color | false | 禁用颜色 |
| - | log-file | - | 日志文件路径 |
| - | max-connections | 20000 | 最大连接数 |
| - | tcp-timeout | 360 | TCP 超时（秒） |
| - | udp-timeout | 180 | UDP 超时（秒） |
| - | conn-clear-ratio | 30 | 清理比例 |
| - | conn-clear-min | 1 | 最小清理数 |
| - | disable-conn-clear | false | 禁用自动清理 |
| - | run-test | false | 运行单元测试 |
| -h | help | - | 显示帮助 |

### 日志级别

| 级别 | 值 | 说明 |
|------|-----|------|
| never | 0 | 禁用日志 |
| fatal | 1 | 致命错误 |
| error | 2 | 错误 |
| warn | 3 | 警告 |
| info | 4 | 信息（默认） |
| debug | 5 | 调试 |
| trace | 6 | 追踪 |

## 构建命令

```bash
# 本地构建
make              # Release（默认）
make debug        # Debug 构建
make fast         # 快速构建（无优化）

# musl 静态链接
make musl         # x86_64
make musl-aarch64 # aarch64

# OpenWRT 目标
make arm          # ARMv7
make amd64        # x86_64
make mips24kc_be  # MIPS 大端
make mips24kc_le  # MIPS 小端
make x86          # i686

# 跨平台
make mingw        # Windows x86_64
make mingw32      # Windows i386
make macos        # macOS x86_64
make macos-aarch64 # macOS ARM64

# 代码质量
make test         # cargo test --release
make test-verbose # 详细测试输出
make check        # cargo check
make clippy       # 代码检查
make fmt          # 代码格式化

# 清理
make clean        # 清理构建产物
make distclean    # 清理所有产物
```

## 架构设计

```
main.rs           # CLI 解析，socket 创建，事件循环启动
lib.rs            # 模块导出，日志宏
config.rs         # 配置和常量

event/
├── mod.rs        # EventLoop（mio Poll），TokenManager
├── tcp.rs        # TcpHandler：accept → connect → 转发
├── udp.rs        # UdpHandler：datagram → 会话 → 转发
├── timer.rs      # 定时器（10秒统计）
└── signals.rs    # SIGTERM/SIGINT 处理

connection/
└── mod.rs        # TcpConnection，UdpSession

manager/
└── mod.rs        # TcpConnectionManager，UdpSessionManager

fd_manager.rs     # Fd64 ↔ RawFd 映射
lru.rs            # LRU 超时清理
log.rs            # 七级日志系统
stats.rs          # 流量统计
types/address.rs  # IPv4/IPv6 地址处理
```

### 核心数据流

```
main()
├─ CLI 解析 → Config
├─ 创建管理器（FdManager, TcpConnectionManager, UdpSessionManager）
├─ 创建监听 socket（SO_REUSEADDR, SO_REUSEPORT）
└─ EventLoop::run()
    ├─ poll.poll() 等待 I/O 事件
    ├─ TCP 监听 → on_accept() → TcpConnection
    ├─ UDP 监听 → on_datagram() → UdpSession
    ├─ 数据转发 → tcp.on_read/on_write() 或 udp.on_response()
    ├─ 定时器（10s）→ 打印统计
    └─ 定时器（400ms）→ LRU 清理
```

### 关键抽象

**Fd64**: u64 包装类型，提供跨平台 FD 抽象。统一处理 Windows RawSocket 和 Unix RawFd，为连接生命周期提供稳定标识符。

**SplicePipe**（Linux only）: 使用 splice() 系统调用实现零拷贝转发。非 Linux 平台回退到 recv/send。

**LruCollector**: 基于最小堆的 LRU 超时清理，O(log n) 复杂度。TCP 360s / UDP 180s 超时。

## 常见问题

**Q: 如何选择 TCP 和 UDP 转发？**
A: 取决于目标协议。HTTP/HTTPS 用 TCP，DNS 用 UDP。不确定时可同时启用 -t -u。

**Q: 连接数达到上限会怎样？**
A: 新连接被拒绝。可用 `--max-connections` 配置，或调整 `ulimit -n`。

**Q: 如何调试连接问题？**
A: `tinymapper -l:1234 -r:443 -t -u --log-level debug --log-position`

**Q: 4to6 和 6to4 翻译的区别？**
A: -4 将 IPv4 转成 ::ffff:x.x.x.x，-6 从 IPv6 映射地址提取 IPv4。

**Q: 如何优雅停止？**
A: `kill $(pidof tinymapper)` 或 Ctrl+C。

## 许可证

MIT License

## 相关链接

- [GitHub](https://github.com/wangyu-/tinyPortMapper)
- [原版 C++](https://github.com/wangyu-/tinyPortMapper)
- [Issues](https://github.com/wangyu-/tinyPortMapper/issues)
