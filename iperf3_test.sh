#!/bin/bash
# tinyPortMapper Rust版本 iperf3 性能测试脚本
# 用法: ./iperf3_test.sh [--debug]
#
# 测试模式:
#   1. 直接连接测试 - iperf3 客户端直接连接 iperf3 服务器
#   2. 转发测试 - iperf3 客户端连接 tinyPortMapper，tinyPortMapper 转发到 iperf3 服务器
#
# 性能对比:
#   - 直接连接应该达到 ~90-100 Gbits/sec (本地环回)
#   - 转发测试应该接近直接连接的性能 (~85-95%)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TINYPORTMAPPER="${SCRIPT_DIR}/target/release/tinyportmapper"
LISTEN_PORT=3322
SERVER_PORT=5201
TEST_DURATION=5
PARALLEL_STREAMS=4
LOG_DIR="/tmp/iperf3_test"
DEBUG_MODE=false

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
            echo "测试流程:"
            echo "  1. 启动 iperf3 服务器 (端口 $SERVER_PORT)"
            echo "  2. 直接连接测试 (客户端直接连服务器)"
            echo "  3. 启动 tinyPortMapper (监听 $LISTEN_PORT -> 转发到 $SERVER_PORT)"
            echo "  4. 转发测试 (客户端连 tinyPortMapper)"
            exit 0
            ;;
    esac
done

mkdir -p "$LOG_DIR"

cleanup() {
    echo "清理中..."
    pkill -9 iperf3 2>/dev/null || true
    pkill -9 tinyportmapper 2>/dev/null || true
    sleep 1
}
trap cleanup EXIT

echo "=============================================="
echo "   tinyPortMapper Rust 版本性能测试"
echo "=============================================="
echo ""

# 清理之前的进程
pkill -9 iperf3 2>/dev/null || true
pkill -9 tinyportmapper 2>/dev/null || true
sleep 1

echo "步骤 1/4: 启动 iperf3 服务器 (端口 $SERVER_PORT)..."
iperf3 -s -p $SERVER_PORT -D 2>/dev/null || iperf3 -s -p $SERVER_PORT &
sleep 2

# 检查服务器是否启动成功
if ! netstat -tlnp 2>/dev/null | grep -q ":$SERVER_PORT" && ! ss -tlnp | grep -q ":$SERVER_PORT"; then
    echo "错误: iperf3 服务器启动失败!"
    exit 1
fi
echo "  ✓ iperf3 服务器已启动"
echo ""

echo "步骤 2/4: 直接连接测试 (基准性能)..."
echo "  运行 ${TEST_DURATION} 秒，${PARALLEL_STREAMS} 个并行流..."
if [ "$DEBUG_MODE" = true ]; then
    iperf3 -c 127.0.0.1 -p $SERVER_PORT -t $TEST_DURATION -P $PARALLEL_STREAMS 2>&1 | tee $LOG_DIR/direct_output.txt
else
    iperf3 -c 127.0.0.1 -p $SERVER_PORT -t $TEST_DURATION -P $PARALLEL_STREAMS --json > $LOG_DIR/direct_tcp.json
fi
echo "  ✓ 直接测试完成"
echo ""

echo "步骤 3/4: 启动 tinyPortMapper (端口 $LISTEN_PORT -> $SERVER_PORT)..."
LOG_OPTIONS=""
if [ "$DEBUG_MODE" = true ]; then
    LOG_OPTIONS="--log-level debug"
fi

$TINYPORTMAPPER -l 127.0.0.1:$LISTEN_PORT -r 127.0.0.1:$SERVER_PORT -t $LOG_OPTIONS > $LOG_DIR/tinyportmapper.log 2>&1 &
TPM_PID=$!
sleep 2

# 检查 tinyPortMapper 是否启动成功
if ! kill -0 $TPM_PID 2>/dev/null; then
    echo "错误: tinyPortMapper 启动失败!"
    cat $LOG_DIR/tinyportmapper.log
    exit 1
fi
echo "  ✓ tinyPortMapper 已启动 (PID: $TPM_PID)"
echo ""

echo "步骤 4/4: 转发测试..."
echo "  运行 ${TEST_DURATION} 秒，${PARALLEL_STREAMS} 个并行流..."
if [ "$DEBUG_MODE" = true ]; then
    echo "  [Debug 模式: 查看 $LOG_DIR/tinyportmapper.log 获取详细日志]"
    iperf3 -c 127.0.0.1 -p $LISTEN_PORT -t $TEST_DURATION -P $PARALLEL_STREAMS 2>&1 | tee $LOG_DIR/forward_output.txt
else
    iperf3 -c 127.0.0.1 -p $LISTEN_PORT -t $TEST_DURATION -P $PARALLEL_STREAMS --json > $LOG_DIR/forward_tcp.json
fi
echo "  ✓ 转发测试完成"
echo ""

# 停止 tinyPortMapper
pkill -9 tinyportmapper 2>/dev/null || true

echo "=============================================="
echo "              测试结果汇总"
echo "=============================================="
echo ""

# 解析测试结果
parse_result() {
    local json_file="$1"
    local key="${2:-sum_sent}"
    try:
        with open(os.path.join(LOG_DIR, json_file), 'r') as f:
            data = json.load(f)
        end = data.get('end', {})
        if key in end and 'bits_per_second' in end[key]:
            bits = end[key]['bits_per_second']
            if bits > 1e9:
                return f"{bits/1e9:.2f} Gbits/sec"
            else:
                return f"{bits/1e6:.2f} Mbits/sec"
        return "N/A"
    except Exception as e:
        return f"Error: {e}"
}

TCP_DIRECT=$(parse_result "direct_tcp.json")
TCP_FORWARD=$(parse_result "forward_tcp.json")

echo "┌─────────────────────────────────────────────────────────────┐"
echo "│                        TCP 性能测试                         │"
echo "├─────────────────────────────────────────────────────────────┤"
echo "│  测试项目              │           传输速率                  │"
echo "├─────────────────────────────────────────────────────────────┤"
printf "│  1. 直接连接 (基准)    │  %-35s │\n" "$TCP_DIRECT"
printf "│  2. 转发连接 (tinyPortMapper) │  %-35s │\n" "$TCP_FORWARD"
echo "└─────────────────────────────────────────────────────────────┘"
echo ""

# 计算性能比
if [[ "$TCP_DIRECT" =~ ([0-9.]+)[[:space:]]*Gbits/sec ]] && [[ "$TCP_FORWARD" =~ ([0-9.]+)[[:space:]]*Gbits/sec ]]; then
    direct_val="${BASH_REMATCH[1]}"
    forward_val="${BASH_REMATCH[1]}"
    ratio=$(echo "scale=1; $forward_val / $direct_val * 100" | bc 2>/dev/null || echo "N/A")
    echo "📊 性能分析:"
    echo "   转发性能是直接连接的 ${ratio}%"
    if (( $(echo "$ratio > 85" | bc -l) )); then
        echo "   ✅ 优秀: 转发性能损失很小 (<15%)"
    elif (( $(echo "$ratio > 70" | bc -l) )); then
        echo "   ⚠️  一般: 转发性能有一定损失"
    else
        echo "   ❌ 较差: 转发性能损失较大，需要优化"
    fi
elif [[ "$TCP_DIRECT" =~ Error ]] || [[ "$TCP_FORWARD" =~ Error ]]; then
    echo "⚠️  测试结果解析失败，请检查日志文件"
fi

echo ""
echo "📁 日志文件:"
echo "   - 直接测试: $LOG_DIR/direct_tcp.json"
echo "   - 转发测试: $LOG_DIR/forward_tcp.json"
if [ "$DEBUG_MODE" = true ]; then
    echo "   - tinyPortMapper 日志: $LOG_DIR/tinyportmapper.log"
fi
echo ""
echo "测试完成! 🎉"
