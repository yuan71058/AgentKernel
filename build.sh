#!/usr/bin/env bash
set -euo pipefail

# ─── 配置 ─────────────────────────────────
PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARY_NAME="agentkernel"

usage() {
  echo "用法: ./build.sh [选项]"
  echo ""
  echo "选项:"
  echo "  (无)        编译本机版本，输出到 dist/"
  echo "  --linux     交叉编译 Linux x86_64 静态二进制 (musl)"
  echo "  --all       同时编译本机 + Linux 版本"
  echo "  -o <dir>    自定义输出目录 (默认 dist/)"
  echo "  -h          显示帮助"
  exit 0
}

# ─── 参数解析 ─────────────────────────────
BUILD_NATIVE=true
BUILD_LINUX=false
OUTPUT_DIR="$PROJECT_DIR/dist"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --linux)  BUILD_NATIVE=false; BUILD_LINUX=true; shift ;;
    --all)    BUILD_LINUX=true; shift ;;
    -o)       OUTPUT_DIR="$2"; shift 2 ;;
    -h|--help) usage ;;
    *)        echo "未知参数: $1"; usage ;;
  esac
done

cd "$PROJECT_DIR"
mkdir -p "$OUTPUT_DIR"

# ─── 本机编译 ─────────────────────────────
if [ "$BUILD_NATIVE" = true ]; then
  echo "🔨 编译本机版本"
  echo "   cargo build --release --bin $BINARY_NAME"
  cargo build --release --bin "$BINARY_NAME"

  BIN="target/release/$BINARY_NAME"
  if [ ! -f "$BIN" ]; then
    echo "❌ 编译失败"; exit 1
  fi
  cp "$BIN" "$OUTPUT_DIR/$BINARY_NAME"
  chmod +x "$OUTPUT_DIR/$BINARY_NAME"
  SIZE=$(du -h "$OUTPUT_DIR/$BINARY_NAME" | cut -f1)
  echo "✅ 本机版本: $OUTPUT_DIR/$BINARY_NAME ($SIZE)"
  echo ""
fi

# ─── Linux x86_64 交叉编译 ────────────────
if [ "$BUILD_LINUX" = true ]; then
  TARGET="x86_64-unknown-linux-musl"
  LINUX_BIN="$OUTPUT_DIR/${BINARY_NAME}-linux-amd64"

  echo "🔨 交叉编译 Linux amd64"
  echo "   target: $TARGET"
  echo "   cargo build --release --bin $BINARY_NAME --target $TARGET"

  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-gcc \
  CC_x86_64_unknown_linux_musl=x86_64-linux-musl-gcc \
  cargo build --release --bin "$BINARY_NAME" --target "$TARGET"

  BIN="target/$TARGET/release/$BINARY_NAME"
  if [ ! -f "$BIN" ]; then
    echo "❌ 交叉编译失败"; exit 1
  fi
  cp "$BIN" "$LINUX_BIN"
  chmod +x "$LINUX_BIN"
  SIZE=$(du -h "$LINUX_BIN" | cut -f1)
  echo "✅ Linux amd64: $LINUX_BIN ($SIZE)"
  echo ""
fi

# ─── 完成 ─────────────────────────────────
echo "📦 输出目录: $OUTPUT_DIR"
ls -lh "$OUTPUT_DIR"/${BINARY_NAME}* 2>/dev/null
