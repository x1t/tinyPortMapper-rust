#!/bin/bash
# tinyPortMapper Rust版本 iperf3 TCP 性能测试脚本
# 用法: ./iperf3_tcp_test.sh [--debug]

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TINYPORTMAPPER="/root/tinyportmapper/tinyPortMapper-rust/target/release/tinyportmapper"
LISTEN_PORT=3322
SERVER_PORT=5201
TEST_DURATION=5
PARALLEL_STREAMS=4
LOG_DIR="/tmp/iperf3_test"
DEBUG_MODE=false

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo_color() {
    local color=$1
    local msg=$2
    echo -e "${color}${msg}${NC}"
}

# 解析命令行参数
for arg in "$@"; do
    case $arg in
        --debug)
            DEBUG_MODE=true
            ;;
        --help|-h)
            echo "用法: $0 [--debug]"
            echo ""
            echo "选项:"
            echo "  --debug    启用 debug 日志模式"
            echo "  --help     显示此帮助信息"
            echo ""
            exit 0
            ;;
    esac
done

mkdir -p "$LOG_DIR"

cleanup() {
    echo_color $BLUE "清理进程中..."
    pkill -9 -f "iperf3.*-s.*-p.*$SERVER_PORT" 2>/dev/null || true
    pkill -9 -f "tinyportmapper.*-l.*$LISTEN_PORT" 2>/dev/null || true
    sleep 1
}
trap cleanup EXIT

echo_color $GREEN "╔════════════════════════════════════════════════════════════════╗"
echo_color $GREEN "║           tinyPortMapper Rust 版本 TCP 性能测试                ║"
echo_color $GREEN "╚════════════════════════════════════════════════════════════════╝"
echo ""

# 预清理
cleanup

echo_color $BLUE "[1/4] 启动 iperf3 TCP 服务器 (端口 $SERVER_PORT)..."
iperf3 -s -p $SERVER_PORT -D 2>/dev/null || iperf3 -s -p $SERVER_PORT &
sleep 2

# 检查服务器是否启动成功
if ! netstat -tlnp 2>/dev/null | grep -q ":$SERVER_PORT" && ! ss -tlnp | grep -q ":$SERVER_PORT"; then
    echo_color $RED "错误: iperf3 服务器启动失败!"
    exit 1
fi
echo_color $GREEN "  ✓ iperf3 服务器已启动"
echo ""

echo_color $BLUE "[2/4] 直接连接测试 (基准性能)..."
echo "  运行 ${TEST_DURATION} 秒，${PARALLEL_STREAMS} 个并行流..."
if [ "$DEBUG_MODE" = true ]; then
    iperf3 -c 127.0.0.1 -p $SERVER_PORT -t $TEST_DURATION -P $PARALLEL_STREAMS 2>&1 | tee $LOG_DIR/direct_output.txt
else
    iperf3 -c 127.0.0.1 -p $SERVER_PORT -t $TEST_DURATION -P $PARALLEL_STREAMS --json > $LOG_DIR/direct_tcp.json
fi
echo_color $GREEN "  ✓ 直接测试完成"
echo ""

echo_color $BLUE "[3/4] 启动 tinyPortMapper (端口 $LISTEN_PORT -> $SERVER_PORT)..."
LOG_OPTIONS=""
if [ "$DEBUG_MODE" = true ]; then
    LOG_OPTIONS="--log-level debug"
fi

$TINYPORTMAPPER -l 127.0.0.1:$LISTEN_PORT -r 127.0.0.1:$SERVER_PORT -t $LOG_OPTIONS > $LOG_DIR/tinyportmapper.log 2>&1 &
TPM_PID=$!
sleep 2

# 检查 tinyPortMapper 是否启动成功
if ! kill -0 $TPM_PID 2>/dev/null; then
    echo_color $RED "错误: tinyPortMapper 启动失败!"
    cat $LOG_DIR/tinyportmapper.log
    exit 1
fi
echo_color $GREEN "  ✓ tinyPortMapper 已启动 (PID: $TPM_PID)"
echo ""

echo_color $BLUE "[4/4] 转发测试..."
echo "  运行 ${TEST_DURATION} 秒，${PARALLEL_STREAMS} 个并行流..."
if [ "$DEBUG_MODE" = true ]; then
    iperf3 -c 127.0.0.1 -p $LISTEN_PORT -t $TEST_DURATION -P $PARALLEL_STREAMS 2>&1 | tee $LOG_DIR/forward_output.txt
else
    iperf3 -c 127.0.0.1 -p $LISTEN_PORT -t $TEST_DURATION -P $PARALLEL_STREAMS --json > $LOG_DIR/forward_tcp.json
fi
echo_color $GREEN "  ✓ 转发测试完成"
echo ""

# 停止 tinyPortMapper 以准备结果展示
pkill -9 -f "tinyportmapper.*-l.*$LISTEN_PORT" 2>/dev/null || true

# 使用 Python 汇总结果
echo_color $YELLOW "═══════════════════════════════════════════════════════════════════"
echo_color $YELLOW "  测试结果汇总"
echo_color $YELLOW "═══════════════════════════════════════════════════════════════════"

python3 << 'PYEOF'
import json
import os

LOG_DIR = "/tmp/iperf3_test"

def parse_tcp_result(json_file):
    try:
        with open(os.path.join(LOG_DIR, json_file), 'r') as f:
            data = json.load(f)
        end = data.get('end', {})
        if 'sum_sent' in end and 'bits_per_second' in end['sum_sent']:
            return {"bps": end['sum_sent']['bits_per_second'], "error": None}
        return {"error": "No data in JSON"}
    except Exception as e:
        return {"error": str(e)}

def format_bps(bps):
    if bps >= 1e9: return f"{bps/1e9:.2f} Gbps"
    return f"{bps/1e6:.2f} Mbps"

direct = parse_tcp_result("direct_tcp.json")
forward = parse_tcp_result("forward_tcp.json")

print("\n┌─────────────────────────────────────────────────────────────┐")
print("│                        TCP 性能展示                         │")
print("├─────────────────────────────────────────────────────────────┤")
if direct["error"]:
    print(f"│  1. 直接连接 (基准): 错误 {direct['error']:<28} │")
else:
    print(f"│  1. 直接连接 (基准): {format_bps(direct['bps']):<30} │")

if forward["error"]:
    print(f"│  2. 转发连接 (TPM):  错误 {forward['error']:<28} │")
else:
    print(f"│  2. 转发连接 (TPM):  {format_bps(forward['bps']):<30} │")
print("└─────────────────────────────────────────────────────────────┘")

if not direct["error"] and not forward["error"]:
    ratio = forward["bps"] / direct["bps"] * 100
    status = "✅ 优秀" if ratio > 85 else ("⚠️ 一般" if ratio > 70 else "❌ 较差")
    print(f"\n📊 性能分析: 转发性能是直接连接的 {ratio:.1f}%  {status}")
PYEOF

echo ""
echo_color $BLUE "测试完成! 🎉"
echo ""
