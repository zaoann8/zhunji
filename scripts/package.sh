#!/usr/bin/env bash
# 打包 Zhunji Release 版 → dist/Zhunji-<版本>.dmg（ad-hoc 签名，拖拽安装镜像）。
# 用法：scripts/package.sh
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# 1. core dylib 保证最新（Xcode Build Phase 不自动构建，core 有改动时重建）
bash scripts/build_core.sh

# 2. Release 构建
cd "$ROOT/app"
xcodebuild -project Zhunji.xcodeproj -scheme Zhunji -configuration Release \
    -derivedDataPath build/DerivedData build

# 3. 打包 dmg：临时目录放 app + /Applications 快捷方式（拖拽安装布局），
#    hdiutil 压缩为只读 UDZO 镜像。中途失败也清理临时目录。
PRODUCTS="$PWD/build/DerivedData/Build/Products/Release"
VERSION=$(defaults read "$PRODUCTS/Zhunji.app/Contents/Info" CFBundleShortVersionString)
mkdir -p "$ROOT/dist"
OUT="$ROOT/dist/Zhunji-$VERSION.dmg"
rm -f "$OUT"

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
ditto "$PRODUCTS/Zhunji.app" "$STAGE/Zhunji.app"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "Zhunji" -srcfolder "$STAGE" -ov -format UDZO "$OUT"
SIZE=$(ls -lh "$OUT" | awk '{print $5}')
echo "打包完成：$OUT（$SIZE）"
