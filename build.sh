#!/usr/bin/env bash
set -euo pipefail

# ─── 配置 ─────────────────────────────────
PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
BINARY_NAME="agentkernel"

usage() {
  echo "用法: ./build.sh [选项]"
  echo ""
  echo "选项:"
  echo "  (无)          编译本机版本"
  echo "  --linux       Linux x86_64 静态二进制 (musl)"
  echo "  --windows     Windows x86_64 exe (mingw)"
  echo "  --all         本机 + Linux + Windows 全部编译"
  echo "  -o <dir>      自定义输出目录 (默认 dist/)"
  echo "  -h            显示帮助"
  exit 0
}

# ─── 参数解析 ─────────────────────────────
BUILD_NATIVE=true
BUILD_LINUX=false
BUILD_WINDOWS=false
OUTPUT_DIR="$PROJECT_DIR/dist"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --linux)    BUILD_NATIVE=false; BUILD_LINUX=true; shift ;;
    --windows)  BUILD_NATIVE=false; BUILD_WINDOWS=true; shift ;;
    --all)      BUILD_LINUX=true; BUILD_WINDOWS=true; shift ;;
    -o)         OUTPUT_DIR="$2"; shift 2 ;;
    -h|--help)  usage ;;
    *)          echo "未知参数: $1"; usage ;;
  esac
done

cd "$PROJECT_DIR"
mkdir -p "$OUTPUT_DIR"

build_ok=0

# ─── 本机编译 ─────────────────────────────
if [ "$BUILD_NATIVE" = true ]; then
  echo "🔨 编译本机版本"
  echo "   cargo build --release --bin $BINARY_NAME"
  cargo build --release --bin "$BINARY_NAME"

  BIN="target/release/$BINARY_NAME"
  if [ ! -f "$BIN" ]; then echo "❌ 编译失败"; exit 1; fi
  cp "$BIN" "$OUTPUT_DIR/$BINARY_NAME"
  chmod +x "$OUTPUT_DIR/$BINARY_NAME"
  SIZE=$(du -h "$OUTPUT_DIR/$BINARY_NAME" | cut -f1)
  echo "✅ 本机版本: $OUTPUT_DIR/$BINARY_NAME ($SIZE)"
  echo ""
  build_ok=$((build_ok+1))
fi

# ─── Linux x86_64 ────────────────────────
if [ "$BUILD_LINUX" = true ]; then
  TARGET="x86_64-unknown-linux-musl"
  OUT="$OUTPUT_DIR/${BINARY_NAME}-linux-amd64"

  echo "🔨 交叉编译 Linux amd64 ($TARGET)"
  CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=x86_64-linux-musl-gcc \
  CC_x86_64_unknown_linux_musl=x86_64-linux-musl-gcc \
  cargo build --release --bin "$BINARY_NAME" --target "$TARGET"

  BIN="target/$TARGET/release/$BINARY_NAME"
  if [ ! -f "$BIN" ]; then echo "❌ Linux 编译失败"; exit 1; fi
  cp "$BIN" "$OUT"
  chmod +x "$OUT"
  SIZE=$(du -h "$OUT" | cut -f1)
  echo "✅ Linux amd64: $OUT ($SIZE)"
  echo ""
  build_ok=$((build_ok+1))
fi

# ─── Windows x86_64 ──────────────────────
if [ "$BUILD_WINDOWS" = true ]; then
  TARGET="x86_64-pc-windows-gnu"
  OUT="$OUTPUT_DIR/${BINARY_NAME}-windows-amd64.exe"

  echo "🔨 交叉编译 Windows amd64 ($TARGET)"
  CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
  cargo build --release --bin "$BINARY_NAME" --target "$TARGET"

  BIN="target/$TARGET/release/$BINARY_NAME.exe"
  if [ ! -f "$BIN" ]; then echo "❌ Windows 编译失败"; exit 1; fi
  cp "$BIN" "$OUT"
  SIZE=$(du -h "$OUT" | cut -f1)
  echo "✅ Windows amd64: $OUT ($SIZE)"
  echo ""
  build_ok=$((build_ok+1))
fi

# ─── 完成 ─────────────────────────────────
echo "📦 输出目录: $OUTPUT_DIR  ($build_ok 个构建)"
ls -lh "$OUTPUT_DIR"/${BINARY_NAME}* 2>/dev/null
