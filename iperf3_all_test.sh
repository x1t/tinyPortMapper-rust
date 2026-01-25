#!/bin/bash
# tinyPortMapper Go版本 综合性能测试脚本 (TCP + UDP)
# 用法: ./iperf3_all_test.sh

set -e

TINYPORTMAPPER="/root/tinyportmapper/tinyPortMapper-rust/target/release/tinyportmapper"
LISTEN_PORT=3322
SERVER_PORT=5201
TEST_DURATION=5
PARALLEL_STREAMS=4
LOG_DIR="/tmp/iperf3_test"
UDP_BITRATE="1G"

mkdir -p "$LOG_DIR"

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

# 清理函数
cleanup() {
    pkill -9 -f "tinyportmapper-go.*-l.*$LISTEN_PORT" 2>/dev/null || true
    pkill -9 -f "iperf3.*-s.*-p.*$SERVER_PORT" 2>/dev/null || true
    sleep 1
}

# 预清理
echo_color $BLUE "清理残留进程..."
cleanup
sleep 2

echo ""
echo_color $GREEN "╔════════════════════════════════════════════════════════════════╗"
echo_color $GREEN "║        tinyPortMapper Go 版本 综合性能测试 (TCP + UDP)        ║"
echo_color $GREEN "╚════════════════════════════════════════════════════════════════╝"
echo ""

# ==================== TCP 测试 ====================
echo_color $YELLOW "═══════════════════════════════════════════════════════════════════"
echo_color $YELLOW "  TCP 测试"
echo_color $YELLOW "═══════════════════════════════════════════════════════════════════"
echo ""

echo_color $BLUE "[TCP-1] 启动 iperf3 TCP 服务器..."
iperf3 -s -p $SERVER_PORT &
IPERF_PID=$!
sleep 2

echo_color $BLUE "[TCP-2] 第一轮: 直接 TCP 连接测试..."
iperf3 -c 127.0.0.1 -p $SERVER_PORT -t $TEST_DURATION -P $PARALLEL_STREAMS --json > $LOG_DIR/direct_tcp.json

kill $IPERF_PID 2>/dev/null || true
wait $IPERF_PID 2>/dev/null || true
sleep 1

echo_color $BLUE "[TCP-3] 重新启动 iperf3 TCP 服务器..."
iperf3 -s -p $SERVER_PORT &
IPERF_PID=$!
sleep 2

echo_color $BLUE "[TCP-4] 启动 tinyportmapper (TCP模式)..."
$TINYPORTMAPPER -l 127.0.0.1:$LISTEN_PORT -r 127.0.0.1:$SERVER_PORT -t 2>&1 &
TINYPORT_PID=$!
sleep 2

echo_color $BLUE "[TCP-5] 第二轮: TCP 转发测试..."
iperf3 -c 127.0.0.1 -p $LISTEN_PORT -t $TEST_DURATION -P $PARALLEL_STREAMS --json > $LOG_DIR/forward_tcp.json

kill $TINYPORT_PID 2>/dev/null || true
wait $TINYPORT_PID 2>/dev/null || true
kill $IPERF_PID 2>/dev/null || true
wait $IPERF_PID 2>/dev/null || true

# ==================== UDP 测试 ====================
echo ""
echo_color $YELLOW "═══════════════════════════════════════════════════════════════════"
echo_color $YELLOW "  UDP 测试"
echo_color $YELLOW "═══════════════════════════════════════════════════════════════════"
echo ""

echo_color $BLUE "[UDP-1] 启动 iperf3 UDP 服务器..."
iperf3 -s -p $SERVER_PORT &
IPERF_PID=$!
sleep 2

echo_color $BLUE "[UDP-2] 第一轮: 直接 UDP 连接测试 (4流, ${UDP_BITRATE}/流)..."
iperf3 -c 127.0.0.1 -p $SERVER_PORT -u -b $UDP_BITRATE -t $TEST_DURATION -P $PARALLEL_STREAMS --json > $LOG_DIR/direct_udp.json

kill $IPERF_PID 2>/dev/null || true
wait $IPERF_PID 2>/dev/null || true
sleep 1

echo_color $BLUE "[UDP-3] 重新启动 iperf3 UDP 服务器..."
iperf3 -s -p $SERVER_PORT &
IPERF_PID=$!
sleep 2

echo_color $BLUE "[UDP-4] 启动 tinyportmapper (TCP+UDP模式)..."
$TINYPORTMAPPER -l 127.0.0.1:$LISTEN_PORT -r 127.0.0.1:$SERVER_PORT -t -u 2>&1 &
TINYPORT_PID=$!
sleep 2

echo_color $BLUE "[UDP-5] 第二轮: UDP 转发测试 (4流, ${UDP_BITRATE}/流)..."
iperf3 -c 127.0.0.1 -p $LISTEN_PORT -u -b $UDP_BITRATE -t $TEST_DURATION -P $PARALLEL_STREAMS --json > $LOG_DIR/forward_udp.json

kill $TINYPORT_PID 2>/dev/null || true
wait $TINYPORT_PID 2>/dev/null || true
kill $IPERF_PID 2>/dev/null || true
wait $IPERF_PID 2>/dev/null || true

# ==================== 结果汇总 ====================
echo ""
echo_color $GREEN "╔════════════════════════════════════════════════════════════════╗"
echo_color $GREEN "║                        测试结果汇总                             ║"
echo_color $GREEN "╚════════════════════════════════════════════════════════════════╝"
echo ""

python3 << 'PYEOF'
import json
import os

LOG_DIR = "/tmp/iperf3_test"

def parse_tcp_result(json_file):
    """解析 TCP 测试结果"""
    try:
        with open(os.path.join(LOG_DIR, json_file), 'r') as f:
            data = json.load(f)
        end = data.get('end', {})
        if 'sum_sent' in end and 'bits_per_second' in end['sum_sent']:
            bps = end['sum_sent']['bits_per_second']
            return {"bits_per_second": bps, "error": None}
        return {"error": "No data"}
    except Exception as e:
        return {"error": str(e)}

def parse_udp_result(json_file):
    """解析 UDP 测试结果"""
    try:
        with open(os.path.join(LOG_DIR, json_file), 'r') as f:
            data = json.load(f)
        if "error" in data:
            return {"error": data["error"]}
        end = data.get('end', {})
        if 'sum_sent' in end:
            sent = end['sum_sent']
            return {
                "bits_per_second": sent.get('bits_per_second', 0),
                "lost_packets": sent.get('lost_packets', 0),
                "total_packets": sent.get('packets', 1),
                "loss_percent": sent.get('lost_percent', 0),
                "jitter_ms": sent.get('jitter_ms', 0),
                "error": None
            }
        return {"error": "No data"}
    except Exception as e:
        return {"error": str(e)}

# 解析结果
tcp_direct = parse_tcp_result("direct_tcp.json")
tcp_forward = parse_tcp_result("forward_tcp.json")
udp_direct = parse_udp_result("direct_udp.json")
udp_forward = parse_udp_result("forward_udp.json")

# 格式化带宽
def format_bps(bps):
    if bps >= 1e9:
        return f"{bps/1e9:.2f} Gbps"
    else:
        return f"{bps/1e6:.2f} Mbps"

# TCP 结果
print("┌─────────────────────────────────────────────────────────────────────────────┐")
print("│                           TCP 测试结果                                       │")
print("├─────────────────────────────────────────────────────────────────────────────┤")
if tcp_direct.get("error"):
    print(f"│  直接连接:  错误 - {tcp_direct['error']:<54} │")
else:
    print(f"│  直接连接:  {format_bps(tcp_direct['bits_per_second']):<18}                                        │")
if tcp_forward.get("error"):
    print(f"│  转发连接:  错误 - {tcp_forward['error']:<54} │")
else:
    print(f"│  转发连接:  {format_bps(tcp_forward['bits_per_second']):<18}                                        │")
print("└─────────────────────────────────────────────────────────────────────────────┘")

# UDP 结果
print()
print("┌─────────────────────────────────────────────────────────────────────────────┐")
print("│                           UDP 测试结果                                       │")
print("├─────────────────────────────────────────────────────────────────────────────┤")
if udp_direct.get("error"):
    print(f"│  直接连接:  错误 - {udp_direct['error']:<54} │")
else:
    loss = udp_direct.get('loss_percent', 0)
    print(f"│  直接连接:  {format_bps(udp_direct['bits_per_second']):<18}  丢包: {loss:.2f}%                     │")
if udp_forward.get("error"):
    print(f"│  转发连接:  错误 - {udp_forward['error']:<54} │")
else:
    loss = udp_forward.get('loss_percent', 0)
    print(f"│  转发连接:  {format_bps(udp_forward['bits_per_second']):<18}  丢包: {loss:.2f}%                     │")
print("└─────────────────────────────────────────────────────────────────────────────┘")

# 性能对比
print()
print("┌─────────────────────────────────────────────────────────────────────────────┐")
print("│                           性能对比总结                                       │")
print("├─────────────────────────────────────────────────────────────────────────────┤")

tcp_ratio = 0
udp_ratio = 0

if not tcp_direct.get("error") and not tcp_forward.get("error"):
    tcp_ratio = tcp_forward['bits_per_second'] / tcp_direct['bits_per_second'] * 100
    tcp_status = "✅ 优秀" if tcp_ratio > 80 else ("⚠️  中等" if tcp_ratio > 50 else "❌ 较差")
    print(f"│  TCP 转发性能: {tcp_ratio:.1f}%  {tcp_status:<15}                               │")

if not udp_direct.get("error") and not udp_forward.get("error"):
    udp_ratio = udp_forward['bits_per_second'] / udp_direct['bits_per_second'] * 100
    udp_status = "✅ 完美" if udp_ratio > 95 else ("✅ 优秀" if udp_ratio > 80 else "⚠️  中等")
    print(f"│  UDP 转发性能: {udp_ratio:.1f}%  {udp_status:<15}                               │")

print("└─────────────────────────────────────────────────────────────────────────────┘")

# 结论
print()
if tcp_ratio > 0 and udp_ratio > 0:
    if udp_ratio > tcp_ratio + 30:
        print("💡 分析: UDP 性能显著优于 TCP，因为 UDP 转发更简单（无连接、无双向流控）")
    elif tcp_ratio > 40:
        print("💡 分析: TCP 性能正常，用户态代理 40-50% 是合理水平")
PYEOF

echo ""
echo_color $BLUE "测试完成! 结果保存在: $LOG_DIR/"
ls -la $LOG_DIR/*.json
echo ""
