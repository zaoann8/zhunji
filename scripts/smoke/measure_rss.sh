#!/usr/bin/env bash
# P0.5 决策门：测 FFI 冒烟进程的常驻 RSS。
# 用法：scripts/smoke/measure_rss.sh [预热秒数，默认 3]
set -euo pipefail
cd "$(dirname "$0")"

WARMUP="${1:-3}"

../build_core.sh --debug

CORE_DYLIB=../../core/target/debug/libzhunji_core.dylib
BUILD_DIR=$(mktemp -d)
trap 'rm -rf "$BUILD_DIR"' EXIT

swiftc -o "$BUILD_DIR/smoke_ffi" SmokeFFI.swift \
    -L "$(dirname "$CORE_DYLIB")" -lzhunji_core \
    -Xlinker -rpath -Xlinker @loader_path \
    2>&1
cp "$CORE_DYLIB" "$BUILD_DIR/"

# 后台运行并驻留，预热后采 RSS
SMOKE_HOLD_SECONDS=30 "$BUILD_DIR/smoke_ffi" &
PID=$!

sleep "$WARMUP"
echo "── RSS（$WARMUP s 预热后）──"
ps -o pid,rss,vsz,comm -p "$PID" | tail -1
RSS_KB=$(ps -o rss= -p "$PID" | tr -d ' ')
echo "RSS: $((RSS_KB / 1024)) MB ($RSS_KB KB)"
if (( RSS_KB <= 80 * 1024 )); then
    echo "✅ 决策门通过：RSS ≤ 80MB"
else
    echo "❌ 决策门未通过：RSS > 80MB"
fi

wait "$PID"
