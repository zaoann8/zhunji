# Zhunji（准记）

macOS 语音输入工具：按住热键说话，松手即把语音转成文字插入当前输入框。

由 Tauri + React 版（`zhunlu/`）重写为 **SwiftUI 原生 + Rust 核心**，单进程常驻内存 < 80MB。

## 功能

- **全局热键听写** — 按住热键说话，松手自动转写并注入文字；支持 Esc 取消、✓ 确认
- **双 ASR 通道** — 内置免费引擎（零配置）+ Grok STT（自建网关，免费）
- **热词词典** — 自定义热词，转写时偏向识别（如「vibe coding」不会变成「web coding」）
- **实时反馈** — 胶囊浮窗显示录音电平、转写中、插入结果状态
- **历史记录** — 会话历史、时长、重新转录
- **多供应商管理** — 自定义 OpenAI 兼容 ASR 网关

## 技术架构

```
SwiftUI (app/) ← FFI → Rust cdylib (core/) → ASR 网关 → 文本注入
```

- `core/` — Rust 核心：录音（cpal）、全局热键（CGEventTap）、听写状态机、ASR 引擎、持久化
- `app/` — SwiftUI：菜单栏、胶囊浮窗、设置页、历史
- 载荷统一 JSON，经 C ABI（`#[no_mangle]`）跨语言传递

## 构建

依赖：Rust nightly、Xcode、[uv](https://docs.astral.sh/uv/)（可选）

```bash
# 1. 编译 Rust 核心（产出 libzhunji_core.dylib）
cd core && cargo build --release
cd .. && ./scripts/build_core.sh

# 2. 新增/删除 Swift 文件后重新生成 Xcode 工程
python3 scripts/gen_xcodeproj.py

# 3. 构建 app
cd app && xcodebuild -project Zhunji.xcodeproj -scheme Zhunji \
  -configuration Debug -derivedDataPath build/DerivedData build

# 4. 打包 dmg
bash scripts/package.sh
```

## 数据位置

| 内容                 | 路径                                           |
| -------------------- | ---------------------------------------------- |
| 设置 / 历史 / 供应商 | `~/Library/Application Support/Zhunji/`        |
| Grok STT 凭据        | `~/.doudou_mac_grok_stt.json`                  |
| 日志                 | `~/Library/Logs/Zhunji/zhunji.log`（UTC 时间） |

## 权限

首次启动需授权：**麦克风**（录音）与**辅助功能**（全局热键 + 文字注入）。
