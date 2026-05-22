#!/usr/bin/env bash
set -euo pipefail

# ─── 配置 ─────────────────────────────────
PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
OUTPUT_DIR="${1:-$PROJECT_DIR/dist}"
BINARY_NAME="agentkernel"

echo "🔨 AgentKernel Build"
echo "   项目: $PROJECT_DIR"
echo "   输出: $OUTPUT_DIR"
echo ""

# ─── 编译 ─────────────────────────────────
echo "▶ cargo build --release --bin $BINARY_NAME ..."
cd "$PROJECT_DIR"
cargo build --release --bin "$BINARY_NAME"

BINARY_PATH="$PROJECT_DIR/target/release/$BINARY_NAME"
if [ ! -f "$BINARY_PATH" ]; then
  echo "❌ 编译失败：找不到 $BINARY_PATH"
  exit 1
fi

# ─── 输出目录 ─────────────────────────────
mkdir -p "$OUTPUT_DIR"
cp "$BINARY_PATH" "$OUTPUT_DIR/$BINARY_NAME"
chmod +x "$OUTPUT_DIR/$BINARY_NAME"

# ─── 复制前端静态文件 ─────────────────────
if [ -d "$PROJECT_DIR/web/static" ]; then
  mkdir -p "$OUTPUT_DIR/web/static"
  cp -r "$PROJECT_DIR/web/static/"* "$OUTPUT_DIR/web/static/"
  echo "   前端文件 → $OUTPUT_DIR/web/static/"
fi

# ─── 完成 ─────────────────────────────────
SIZE=$(du -h "$OUTPUT_DIR/$BINARY_NAME" | cut -f1)
echo ""
echo "✅ 编译完成"
echo "   二进制: $OUTPUT_DIR/$BINARY_NAME ($SIZE)"
echo "   前端:   $OUTPUT_DIR/web/static/"
echo ""
echo "   启动: $OUTPUT_DIR/$BINARY_NAME"
