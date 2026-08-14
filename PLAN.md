# 准记 Zhunji SwiftUI 原生重写（单进程化）

## Context

用户要求把 Zhunji（准记）从 Tauri + React/WebView 架构重写为 SwiftUI 原生 app，目标参照 AutoGLM.app（SwiftUI 原生、**单进程 63.8MB**）。动机：

- 现状 276MB / 4-5 个进程（WebKit 渲染拆分），用户明确"不喜欢这么重的"
- WebView 渲染进程由 macOS 强制拆分，Tauri 架构下无解；唯一路径是前端换原生 UI
- 用户已拍板：**必须重写**，范围 **MVP 先行**（先核心链路，后功能对齐），**UI 语言仅保留中文**（砍掉 zh-TW/en/ja/ko 多语言体系，不迁移 i18n 框架，文案直接写中文）。
- 用户对 P1 的硬性要求：① 所列功能**完整复刻**（交互行为/状态机/动画细节与原版一致，不是简化版）；② 内存必须 **<80MB**；③ 前端**一比一参照原版视觉**（不强制像素级，但必须美观）。

## 技术路线（已评估对比）

| 维度 | A 完全脱离 Tauri（**推荐**）    | B Tauri 壳 + Swift 插件              |
| :--- | :------------------------------ | :----------------------------------- |
| 内存 | 60-80MB 单进程 ✅ 达标          | 100-120MB，80 目标不可达 ❌          |
| 风险 | FFI 胶水为成熟模式              | 依赖官方实验插件，窗口生命周期风险高 |
| 打包 | xcodebuild + 嵌 dylib，自建脚本 | 现有 tauri build                     |

**路线 A**：SwiftUI app + Rust 核心编译 cdylib + 手写 C ABI（cbindgen）FFI 桥。

- Rust→Swift 事件：`extern "C"` 回调函数指针注册（Swift 持强引用）
- Swift→Rust：同步导出函数；长任务走 core 内部 tokio runtime + 完成回调
- 载荷统一 JSON（复用 types.rs 序列化），字符串借用语义
- 托盘 MenuBarExtra / autostart SMAppService / 单实例 / 文件对话框——全 Swift 原生，零新 crate

## 目录结构

```
zhunji-native/                          # 新工程根（原 zhunlu/ 冻结为行为参照，只读不删）
│
├── core/                               # Rust 核心，单 cdylib crate（lto+strip）
│   ├── Cargo.toml                      # crate-type=["cdylib"]，无任何 tauri 依赖
│   └── src/
│       ├── lib.rs                      # 模块声明 + 全局状态持有（Coordinator/ASR 实例）
│       ├── ffi.rs                      # 对外唯一接口：init / register_events / 命令分发
│       ├── event_bus.rs                # 事件总线（原 tauri emit/emit_to 的替代）
│       ├── coordinator/                # 从 zhunlu/src-tauri/src/coordinator 迁移+解耦
│       │   ├── mod.rs                  # 听写状态机（去 AppHandle/Emitter）
│       │   ├── asr_wiring.rs           # ASR 引擎接线（providers 数据下沉 core）
│       │   └── dictation.rs            # async_runtime → tokio
│       ├── asr/                        # 纯逻辑直接迁移
│       │   ├── grok_stt.rs             # 修反向依赖 get_terms → core 词典模块
│       │   └── doubao.rs / doubao_proto.rs
│       ├── recorder.rs                 # cpal 录音
│       ├── hotkey.rs                   # CGEventTap 全局热键
│       ├── insertion.rs                # CGEvent 文字注入
│       ├── persistence/                # 设置/历史持久化（目录与现版一致，零迁移）
│       ├── providers.rs                # 供应商 CRUD（从 commands 下沉）
│       ├── dictionary.rs               # 热词词典（从 commands 下沉）
│       └── net.rs / types.rs / permissions.rs / audio_mute.rs / shortcut_binding.rs …
│
├── app/                                # SwiftUI 工程（Xcode）
│   ├── Zhunji.xcodeproj
│   ├── Entitlements.plist              # 麦克风 + 辅助功能
│   ├── Sources/
│   │   ├── ZhunjiApp.swift             # @main、MenuBarExtra、SMAppService、单实例
│   │   ├── FFI/                        # C ABI 声明 + 回调注册封装（唯一碰 Rust 的地方）
│   │   │   ├── Core.swift              # zhunji_init / register_events / command 封装
│   │   │   └── EventSink.swift         # 事件回调 → @MainActor 分发
│   │   ├── Windows/
│   │   │   ├── MainWindow.swift        # 设置主窗口（NSWindow + NSHostingView）
│   │   │   └── CapsulePanel.swift      # 胶囊 NSPanel（置顶/穿透/定位几何）
│   │   ├── Views/
│   │   │   ├── Capsule/                # 胶囊 UI：状态机视图 + 电平条 + 动画
│   │   │   ├── Onboarding/             # 权限引导
│   │   │   └── Settings/               # 设置页（引擎/凭据/热键/麦克风）
│   │   └── Services/                   # AudioCue 提示音、NSEvent 热键录制等
│   └── Resources/                      # 图标、Assets
│
└── scripts/
    └── build_core.sh                   # cargo build --release + 拷贝 libcore.dylib
                                        # （Xcode Build Phase 调用）
```

## 分阶段实施

### P0 尖峰（决策门，~1-2 周）

SwiftUI 空壳 + 剥离 tauri 的 cdylib 编译通过 + init/事件回调打通。
**验证：实测 RSS ≤ 80MB（用户硬性指标），不达标则审视方案。**

1. core crate 骨架：迁纯逻辑模块（asr/*、net、recorder、hotkey、insertion、persistence、types、coordinator_state……）
2. 修复两处反向依赖：grok_stt.rs 的 `commands::get_terms`、asr_wiring 的 `list_providers` → providers/词典数据下沉 core
3. coordinator 事件解耦：`app.emit/emit_to` 收敛为内部事件总线 + `register_events` 回调；`tauri::async_runtime` → tokio
4. Xcode 工程 + build_core.sh + FFI 冒烟（init → 事件回调收到第一条）

### P1 MVP（核心链路，完整复刻标准）

托盘 MenuBarExtra、全局热键（Rust 侧 CGEventTap 保留）、权限引导（Swift 申请麦克风，Rust 查询）、按住说话 → 录音 → grok_stt/豆包转写 → CGEvent 注入、胶囊浮窗（**NSPanel** 置顶/穿透 + classic 样式 + 全状态机 + partial-text 中间文本 + 取消/确认）、核心设置页（ASR 引擎选择、Grok 凭据、热键、麦克风）。文案直接写死中文，不建 i18n 框架。

**完整复刻清单（对照原版 Capsule.tsx 行为）**：warming 预备态、进出场缩放动画（capsule-in/out）、录音电平实时条、转写中 thinking 态、partial-text 中间文本实时上屏、polishing 态、done 显示"已插入 N 字"后自动隐藏、Esc 取消 / ✓ 确认、录音提示音（Web Audio → Swift AVFoundation 复刻）、Shift 触发音中翻译（若在 P1 范围则复刻）。视觉一比一参照原版毛玻璃/配色/排版。

**验证：TextEdit 全链路听写可用且交互与原版一致；常驻内存 <80MB（硬性）；warmup/保活/请求耗时日志正常（沿用 grok_stt 优化成果）。**

### P2 功能对齐

历史（播放/重转录/导出）、概览、供应商×2 管理、翻译、词典、主题、热键录制 UI、麦克风电平监听。（多语言已砍，无 i18n 项。）

### P3 打磨

siri 胶囊动画（Metal 复刻原 WebGL 声波，一比一参照 SiriGL.tsx，禁止嵌 WKWebView）、签名公证。

## 关键风险与对策

1. **coordinator 事件/窗口耦合**（coordinator.rs 2206 行 + capsule_focus.rs 862 行）：capsule 窗口定位/穿透是纯几何计算，移植为 Swift NSPanel（.nonactivatingPanel），无需 FFI
2. **60Hz 录音电平流**：Rust 16ms 批量取最新值经回调；Swift 回调线程只更新原子值，TimelineView/Canvas 按显示刷新率渲染
3. **热键录制 UI**：NSEvent local monitor 原生实现，序列化格式复用 shortcut_binding.rs
4. **数据零迁移**：持久化目录与现版一致（~/.doudou_mac_grok_stt.json、prefs 等），历史/设置直接沿用

## 参照文件（旧 zhunlu/ 只读）

- coordinator.rs — 事件解耦核心；commands/mod.rs — 50 命令 FFI 分组基准
- capsule_focus.rs — NSPanel 移植源；lib.rs — tray/plugin 拆除参照
- grok_stt.rs / asr_wiring.rs — 反向依赖修复点

## 验证方式

- P0：`cargo build --release`（core）+ xcodebuild 出 app，`ps` 确认单进程，RSS ≤85MB
- P1：TextEdit 中按住热键说话 → 文字上屏；日志出现"首次预热完成"；常驻 <80MB
- P2：逐页对照现版功能清单（探索报告已有）
