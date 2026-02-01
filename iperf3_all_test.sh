#!/bin/bash
# tinyPortMapper Rust版本 综合性能测试脚本 (TCP + UDP)
# 用法: ./iperf3_all_test.sh [--debug]

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TCP_TEST="$SCRIPT_DIR/iperf3_tcp_test.sh"
UDP_TEST="$SCRIPT_DIR/iperf3_udp_test.sh"

# 颜色定义
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo_color() {
    local color=$1
    local msg=$2
    echo -e "${color}${msg}${NC}"
}

echo_color $GREEN "╔════════════════════════════════════════════════════════════════╗"
echo_color $GREEN "║           tinyPortMapper Rust 版本 综合性能测试                ║"
echo_color $GREEN "╚════════════════════════════════════════════════════════════════╝"
echo ""

# 检查分项脚本是否存在
if [[ ! -f "$TCP_TEST" || ! -f "$UDP_TEST" ]]; then
    echo "错误: 找不到分项测试脚本 ($TCP_TEST 或 $UDP_TEST)"
    exit 1
fi

# 传递参数给子脚本
EXTRA_ARGS=""
for arg in "$@"; do
    EXTRA_ARGS="$EXTRA_ARGS $arg"
done

echo_color $BLUE ">>> 开始进行 TCP 性能测试..."
bash "$TCP_TEST" $EXTRA_ARGS
echo ""

echo_color $BLUE ">>> 开始进行 UDP 性能测试..."
bash "$UDP_TEST" $EXTRA_ARGS
echo ""

echo_color $GREEN "╔════════════════════════════════════════════════════════════════╗"
echo_color $GREEN "║                    综合性能测试全部完成!                       ║"
echo_color $GREEN "╚════════════════════════════════════════════════════════════════╝"
echo ""
echo "提示: 详细 JSON 日志已保存在 /tmp/iperf3_test/"
echo ""
