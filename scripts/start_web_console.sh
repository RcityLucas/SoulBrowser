#!/bin/bash

# SoulBrowser Web Console 快速启动脚本

set -e

# 颜色定义
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}   SoulBrowser Web Console 启动器${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# 获取项目根目录
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
cd "$PROJECT_ROOT"

# 检查二进制文件
if [ ! -f "target/release/soulbrowser" ] && [ ! -f "target/debug/soulbrowser" ]; then
    echo -e "${YELLOW}未找到编译好的二进制文件，正在编译...${NC}"
    cargo build --release
fi

# 选择二进制文件
BINARY=""
if [ -f "target/release/soulbrowser" ]; then
    BINARY="target/release/soulbrowser"
    echo -e "${GREEN}✓ 使用 Release 版本${NC}"
elif [ -f "target/debug/soulbrowser" ]; then
    BINARY="target/debug/soulbrowser"
    echo -e "${YELLOW}⚠ 使用 Debug 版本（性能较低）${NC}"
fi

# 配置参数
BACKEND_PORT="${BACKEND_PORT:-8791}"
FRONTEND_PORT="${FRONTEND_PORT:-5173}"
WS_URL="${SOULBROWSER_WS_URL:-}"

echo ""
echo -e "${BLUE}配置:${NC}"
echo -e "  后端端口: ${GREEN}$BACKEND_PORT${NC}"
echo -e "  前端端口: ${GREEN}$FRONTEND_PORT${NC}"

# 检查是否在 WSL 环境
if grep -qi microsoft /proc/version 2>/dev/null; then
    echo -e "${YELLOW}⚠ 检测到 WSL 环境${NC}"

    if [ -z "$WS_URL" ]; then
        echo ""
        echo -e "${YELLOW}建议：${NC}"
        echo -e "  1. 在 Windows PowerShell 中启动 Chrome:"
        echo -e "     ${GREEN}\"C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe\" --remote-debugging-port=9222 --user-data-dir=C:\\ChromeRemote${NC}"
        echo ""
        echo -e "  2. 然后设置环境变量:"
        echo -e "     ${GREEN}export SOULBROWSER_WS_URL=http://127.0.0.1:9222${NC}"
        echo ""
        read -p "是否继续? (y/n) " -n 1 -r
        echo
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            exit 1
        fi
    else
        echo -e "  WebSocket URL: ${GREEN}$WS_URL${NC}"
    fi
fi

echo ""
echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}   启动服务${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""

# 启动后端服务
echo -e "${GREEN}▶ 启动后端服务...${NC}"

BACKEND_CMD="$BINARY --metrics-port 0 serve --port $BACKEND_PORT"
if [ -n "$WS_URL" ]; then
    BACKEND_CMD="$BACKEND_CMD --ws-url $WS_URL"
fi

echo -e "${YELLOW}命令: $BACKEND_CMD${NC}"

# 在后台启动后端
$BACKEND_CMD > /tmp/soulbrowser_backend.log 2>&1 &
BACKEND_PID=$!

echo -e "${GREEN}✓ 后端服务已启动 (PID: $BACKEND_PID)${NC}"
echo -e "  日志: /tmp/soulbrowser_backend.log"

# 等待后端启动
echo -e "${YELLOW}等待后端服务就绪...${NC}"
for i in {1..30}; do
    if curl -s http://localhost:$BACKEND_PORT/health > /dev/null 2>&1; then
        echo -e "${GREEN}✓ 后端服务就绪${NC}"
        break
    fi
    if [ $i -eq 30 ]; then
        echo -e "${RED}✗ 后端启动超时${NC}"
        echo -e "${RED}请查看日志: tail -f /tmp/soulbrowser_backend.log${NC}"
        kill $BACKEND_PID 2>/dev/null || true
        exit 1
    fi
    sleep 1
done

# 启动前端服务
echo ""
echo -e "${GREEN}▶ 启动前端服务...${NC}"

cd web-console

# 检查依赖
if [ ! -d "node_modules" ]; then
    echo -e "${YELLOW}安装前端依赖...${NC}"
    npm install
fi

# 启动前端（前台运行）
echo -e "${GREEN}✓ 前端服务启动中...${NC}"
echo ""
echo -e "${BLUE}========================================${NC}"
echo -e "${GREEN}   🎉 启动成功！${NC}"
echo -e "${BLUE}========================================${NC}"
echo ""
echo -e "${GREEN}访问地址:${NC}"
echo -e "  前端: ${BLUE}http://localhost:$FRONTEND_PORT${NC}"
echo -e "  后端: ${BLUE}http://localhost:$BACKEND_PORT${NC}"
echo ""
echo -e "${YELLOW}提示:${NC}"
echo -e "  • 按 ${GREEN}Ctrl+C${NC} 停止服务"
echo -e "  • 后端日志: ${GREEN}tail -f /tmp/soulbrowser_backend.log${NC}"
echo ""
echo -e "${BLUE}========================================${NC}"
echo ""

# 清理函数
cleanup() {
    echo ""
    echo -e "${YELLOW}正在停止服务...${NC}"
    kill $BACKEND_PID 2>/dev/null || true
    echo -e "${GREEN}✓ 服务已停止${NC}"
    exit 0
}

trap cleanup SIGINT SIGTERM

# 启动前端开发服务器
npm run dev

# 如果前端退出，清理后端
cleanup
