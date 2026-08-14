#!/usr/bin/env bash
# P0.4 FFI 冒烟：编译最小 Swift 程序链接 core dylib 并运行。
set -euo pipefail
cd "$(dirname "$0")"

# 确保 dylib 最新
../build_core.sh --debug

CORE_DYLIB=../../core/target/debug/libzhunji_core.dylib
BUILD_DIR=$(mktemp -d)

swiftc -o "$BUILD_DIR/smoke_ffi" SmokeFFI.swift \
    -L "$(dirname "$CORE_DYLIB")" -lzhunji_core \
    -Xlinker -rpath -Xlinker @loader_path \
    2>&1

cp "$CORE_DYLIB" "$BUILD_DIR/"
echo "── 运行冒烟 ──"
"$BUILD_DIR/smoke_ffi"
echo "── 冒烟退出码: $? ──"
rm -rf "$BUILD_DIR"
