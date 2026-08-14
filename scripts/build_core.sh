#!/usr/bin/env bash
# 构建 core cdylib 并拷贝到 app 工程 Resources（Xcode Build Phase 调用）。
# 用法：scripts/build_core.sh [--debug]
set -euo pipefail
cd "$(dirname "$0")/../core"

if [[ "${1:-}" == "--debug" ]]; then
    cargo build
    DYLIB=target/debug/libzhunji_core.dylib
else
    cargo build --release
    DYLIB=target/release/libzhunji_core.dylib
fi

# cargo 产出的 dylib install_name 是绝对路径（target/release/...），不改的话
# 链接器会把它写进 app 的 LC_LOAD_DYLIB → dyld 启动时加载源码目录的旧文件，
# bundle 里拷的副本永远用不上（曾致打包版行为与本地构建不一致、日志不写）。
# 改为 @rpath：配合 Xcode LD_RUNPATH_SEARCH_PATHS（@executable_path/../Frameworks）
# 从 app 包内加载。install_name_tool 会破坏签名，dylib 是 ad-hoc 无签名，无碍。
install_name_tool -id @rpath/libzhunji_core.dylib "$DYLIB"

mkdir -p ../app/Resources
cp "$DYLIB" ../app/Resources/libzhunji_core.dylib
SIZE=$(ls -lh ../app/Resources/libzhunji_core.dylib | awk '{print $5}')
echo "core dylib ready: app/Resources/libzhunji_core.dylib ($SIZE)"
