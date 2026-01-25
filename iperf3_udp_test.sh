#!/bin/bash
# tinyPortMapper Go版本 UDP iperf3 性能测试脚本
# 用法: ./iperf3_udp_test.sh

set -e

TINYPORTMAPPER="/root/tinyportmapper/tinyPortMapper-rust/target/release/tinyportmapper"
LISTEN_PORT=3322
SERVER_PORT=5201
TEST_DURATION=5
PARALLEL_STREAMS=4
LOG_DIR="/tmp/iperf3_test"
TARGET_BITRATE="1G"  # UDP 目标比特率

mkdir -p "$LOG_DIR"

# 预先清理
echo "清理残留进程..."
pkill -9 -f "tinyportmapper-go.*-l.*$LISTEN_PORT" 2>/dev/null || true
pkill -9 -f "iperf3.*-s.*-p.*$SERVER_PORT" 2>/dev/null || true
sleep 2

echo "=============================================="
echo "  UDP 性能测试 - 直接连接 vs tinyportmapper 转发"
echo "=============================================="
echo ""

echo "[1/4] 启动 iperf3 UDP 服务器..."
iperf3 -s -p $SERVER_PORT &
IPERF_PID=$!
sleep 2

echo ""
echo "[2/4] 第一轮: 直接 UDP 连接测试 (4流, 1Gbps/流)..."
echo "     命令: iperf3 -c 127.0.0.1 -p $SERVER_PORT -u -b $TARGET_BITRATE -t $TEST_DURATION -P $PARALLEL_STREAMS"
iperf3 -c 127.0.0.1 -p $SERVER_PORT -u -b $TARGET_BITRATE -t $TEST_DURATION -P $PARALLEL_STREAMS --json > $LOG_DIR/direct_udp.json

# 停止 iperf3 服务器
kill $IPERF_PID 2>/dev/null || true
wait $IPERF_PID 2>/dev/null || true
sleep 1

echo ""
echo "[3/4] 重新启动 iperf3 UDP 服务器..."
iperf3 -s -p $SERVER_PORT &
IPERF_PID=$!
sleep 2

echo ""
echo "[4/4] 第二轮: UDP 转发测试 (4流, 1Gbps/流)..."
echo "     命令: iperf3 -c 127.0.0.1 -p $LISTEN_PORT -u -b $TARGET_BITRATE -t $TEST_DURATION -P $PARALLEL_STREAMS"

# 先启动 tinyportmapper
$TINYPORTMAPPER -l 127.0.0.1:$LISTEN_PORT -r 127.0.0.1:$SERVER_PORT -t -u 2>&1 &
TINYPORT_PID=$!
sleep 2

# 测试 UDP 转发
iperf3 -c 127.0.0.1 -p $LISTEN_PORT -u -b $TARGET_BITRATE -t $TEST_DURATION -P $PARALLEL_STREAMS --json > $LOG_DIR/forward_udp.json

# 停止进程
kill $TINYPORT_PID 2>/dev/null || true
wait $TINYPORT_PID 2>/dev/null || true
kill $IPERF_PID 2>/dev/null || true
wait $IPERF_PID 2>/dev/null || true

echo ""
echo "=============================================="
echo "  测试结果对比"
echo "=============================================="
python3 << 'PYEOF'
import json
import os

LOG_DIR = "/tmp/iperf3_test"

def parse_udp_result(json_file):
    """解析 UDP 测试结果"""
    try:
        with open(os.path.join(LOG_DIR, json_file), 'r') as f:
            data = json.load(f)

        # 检查是否有错误
        if "error" in data:
            return {"error": data["error"]}

        end = data.get('end', {})
        if 'sum_sent' in end:
            sent = end['sum_sent']
            sent_bps = sent.get('bits_per_second', 0)
            sent_mbps = sent_bps / 1e6
            sent_gbps = sent_bps / 1e9

            lost_packets = sent.get('lost_packets', 0)
            total_packets = sent.get('packets', 1)
            loss_percent = (lost_packets / total_packets * 100) if total_packets > 0 else 0

            jitter_ms = sent.get('jitter_ms', 0)

            return {
                "bits_per_second": sent_bps,
                "sent_mbps": sent_mbps,
                "sent_gbps": sent_gbps,
                "lost_packets": lost_packets,
                "total_packets": total_packets,
                "loss_percent": loss_percent,
                "jitter_ms": jitter_ms,
                "error": None
            }
        return {"error": "No sum_sent data"}
    except Exception as e:
        return {"error": str(e)}

direct = parse_udp_result("direct_udp.json")
forward = parse_udp_result("forward_udp.json")

print()
print("┌─────────────────────────────────────────────────────────────────────────────┐")
print("│                           UDP 测试结果                                       │")
print("├─────────────────────────────────────────────────────────────────────────────┤")

if direct.get("error"):
    print(f"│  直接连接: 错误 - {direct['error']:<55} │")
else:
    if direct["sent_gbps"] > 1:
        print(f"│  直接连接: {direct['sent_gbps']:.2f} Gbits/sec | 丢包: {direct['loss_percent']:.2f}% | 抖动: {direct['jitter_ms']:.3f} ms")
    else:
        print(f"│  直接连接: {direct['sent_mbps']:.2f} Mbits/sec | 丢包: {direct['loss_percent']:.2f}% | 抖动: {direct['jitter_ms']:.3f} ms")

if forward.get("error"):
    print(f"│  转发连接: 错误 - {forward['error']:<55} │")
else:
    if forward["sent_gbps"] > 1:
        print(f"│  转发连接: {forward['sent_gbps']:.2f} Gbits/sec | 丢包: {forward['loss_percent']:.2f}% | 抖动: {forward['jitter_ms']:.3f} ms")
    else:
        print(f"│  转发连接: {forward['sent_mbps']:.2f} Mbits/sec | 丢包: {forward['loss_percent']:.2f}% | 抖动: {forward['jitter_ms']:.3f} ms")

print("└─────────────────────────────────────────────────────────────────────────────┘")

# 性能对比
if not direct.get("error") and not forward.get("error"):
    if direct["bits_per_second"] > 0:
        ratio = forward["bits_per_second"] / direct["bits_per_second"] * 100
        print()
        print(f"📊 性能对比: UDP 转发性能是直接的 {ratio:.1f}%")
        print()
        if ratio > 80:
            print("✅ UDP 转发性能优秀")
        elif ratio > 50:
            print("⚠️  UDP 转发性能中等，可能存在优化空间")
        else:
            print("❌ UDP 转发性能较差，需要排查问题")

# 错误分析
print()
print("┌─────────────────────────────────────────────────────────────────────────────┐")
print("│                           丢包统计                                           │")
print("├─────────────────────────────────────────────────────────────────────────────┤")
if not direct.get("error"):
    print(f"│  直接连接: 丢包 {direct.get('lost_packets', 'N/A')}/{direct.get('total_packets', 'N/A')} ({direct.get('loss_percent', 0):.2f}%)")
else:
    print(f"│  直接连接: N/A (测试出错)")
if not forward.get("error"):
    print(f"│  转发连接: 丢包 {forward.get('lost_packets', 'N/A')}/{forward.get('total_packets', 'N/A')} ({forward.get('loss_percent', 0):.2f}%)")
else:
    print(f"│  转发连接: N/A (测试出错)")
print("└─────────────────────────────────────────────────────────────────────────────┘")
PYEOF

echo ""
echo "测试完成! 结果保存在 $LOG_DIR/"
ls -la $LOG_DIR/*.json 2>/dev/null || true
