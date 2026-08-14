// 设置页 — 1:1 复刻原版 zhunlu/src/pages/settings/：
//   通用 tab = RecordingInputSection（录音与输入 Card）
//             + ShortcutsSection（快捷键速查 Card）
//   隐私 tab = PrivacyTab 本地优先说明条 + PermissionsSection（权限 Card 四行）
// 视觉 token 逐项取自原版 tokens.css（OlCard/SectionTitle/SettingRow/OlToggle/
// OlSegmented/OlSelectLite/OlPill/Kbd/LevelMeter 全部自绘，不依赖系统控件）。
// 注：Grok STT 凭据不做独立卡片——第三方引擎统一走「ASR」页供应商接入
//（core 侧听写路径自动把供应商 url/apiKey 搬运到 grok_stt 凭据文件）。

import SwiftUI
import AppKit

// MARK: - 通用 tab

struct SettingsGeneralView: View {
    @ObservedObject var model = SettingsModel.shared

    var body: some View {
        ScrollView {
            VStack(spacing: 16) {
                recordingCard
                insertClipboardGroup
                startupGroup
                shortcutsCard
                appearanceCard
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 24)
            .padding(.bottom, 8)
        }
    }

    // MARK: 插入与剪贴板（原版 RecordingInputSection 折叠组）

    private var insertClipboardGroup: some View {
        OlCollapsible(title: "插入与剪贴板") {
            SettingRow(label: "插入后恢复剪贴板", hint: "粘贴成功后恢复你原来的剪贴板内容（仅 Windows / Linux）。") {
                OlToggle(on: model.restoreClipboardAfterPaste) {
                    model.restoreClipboardAfterPaste = $0
                    model.save(["restoreClipboardAfterPaste": $0])
                }
            }
            SettingRow(label: "流式输入", hint: "逐字实时插入，降低感知延迟。不满足条件时回落到一次性粘贴。") {
                OlToggle(on: model.streamingInsert) {
                    model.streamingInsert = $0
                    model.save(["streamingInsert": $0])
                }
            }
        }
    }

    // MARK: 启动（原版 RecordingInputSection 折叠组：AutostartRow + 静默启动）

    private var startupGroup: some View {
        OlCollapsible(title: "启动") {
            AutostartRow()
            SettingRow(label: "启动时静默运行", hint: "所有启动路径都不弹主窗口，仅菜单栏 / 托盘运行。") {
                OlToggle(on: model.startMinimized) {
                    model.startMinimized = $0
                    model.save(["startMinimized": $0])
                }
            }
        }
    }

    // MARK: 外观（原版 ThemeSection：主题选择 + 概览热力图）

    private var appearanceCard: some View {
        OlCard {
            SectionTitle { Text("外观") }
            SettingRow(label: "主题") {
                OlSelectLite(
                    value: model.themeMode,
                    options: [
                        .init(value: "system", label: "跟随系统"),
                        .init(value: "light", label: "浅色"),
                        .init(value: "dark", label: "深色"),
                    ],
                    onChange: { mode in
                        // @Published themeMode 变化 → PermissionGateView 观察并
                        // 立即应用 preferredColorScheme（原版 setThemePreference）。
                        model.themeMode = mode
                        model.save(["themeMode": mode])
                    },
                    width: 220
                )
            }
            // 概览页年度活动热力图开关：关闭只隐藏卡片，活动计数照常记录。
            SettingRow(label: "概览页显示年度活动热力图") {
                OlToggle(on: model.showOverviewActivityHeatmap) {
                    model.showOverviewActivityHeatmap = $0
                    model.save(["showOverviewActivityHeatmap": $0])
                }
            }
        }
    }

    // MARK: 录音与输入

    private var recordingCard: some View {
        OlCard {
            SectionTitle(hint: "全局录音的快捷键与触发方式。") {
                Text("录音与输入")
            }
            SettingRow(label: "录音快捷键", hint: "按下开始捕获语音，全局生效（需辅助功能权限）。") {
                ShortcutRecorderView(
                    binding: hotkeyBinding,
                    disableDisabled: true,
                    disableHint: "核心快捷键不可停用，录音必须绑定一个热键",
                    onReset: {
                        model.dictationHotkey = SettingsModel.ShortcutBinding(
                            primary: "RightControl", modifiers: [])
                        saveHotkey(model.dictationHotkey)
                    }
                )
            }
            SettingRow(label: "录音方式", hint: "切换式按一次开始、再按一次结束；按住说话按下保持、松开结束。") {
                OlSegmented(
                    options: [("toggle", "切换式"), ("hold", "按住说话"), ("auto", "自动")],
                    selected: model.hotkeyMode,
                    onSelect: { model.saveHotkeyMode($0) }
                )
            }
            SettingRow(label: "语音引擎", hint: "识别语音的引擎。豆包无需配置；Grok STT 走你的 worker-search 中转（SSO 池，非流式）。") {
                OlSelectLite(
                    value: model.activeAsrProvider,
                    options: model.providerOptions.map { .init(value: $0.id, label: $0.label) },
                    onChange: { id in
                        // 原版 onEngineChange：保存 prefs 后再同步 providers.json default
                        //（core 内部幂等：prefs 已同值则跳过，仍 emit prefs:changed）。
                        model.activeAsrProvider = id
                        model.save(["activeAsrProvider": id])
                        _ = id.withCString { zhunji_set_default_provider($0) }
                    },
                    width: 220
                )
            }
            SettingRow(label: "首选麦克风", hint: "选择优先输入设备。设备断开时自动切到系统默认。") {
                MicrophoneSelectView(
                    devices: model.microphones,
                    selectedName: model.microphoneDeviceName,
                    onSelect: { model.microphoneDeviceName = $0; model.save(["microphoneDeviceName": $0]) },
                    onOpen: { model.loadMicrophones() }
                )
            }
            SettingRow(label: "录音胶囊", hint: "录音 / 转写时显示屏幕底部胶囊。") {
                OlToggle(on: model.showCapsule) {
                    model.showCapsule = $0
                    model.save(["showCapsule": $0])
                }
            }
            SettingRow(label: "胶囊样式") {
                OlSelectLite(
                    value: model.capsuleStyle,
                    options: [
                        .init(value: "siri", label: "流光 Siri 风格"),
                        .init(value: "classic", label: "准记 默认风格"),
                    ],
                    onChange: { model.capsuleStyle = $0; model.save(["capsuleStyle": $0]) },
                    width: 200
                )
            }
            SettingRow(label: "录音时静音", hint: "录音期间临时静音系统输出，避免扬声器回音。") {
                OlToggle(on: model.muteDuringRecording) {
                    model.muteDuringRecording = $0
                    model.save(["muteDuringRecording": $0])
                }
            }
            SettingRow(label: "录音提示音", hint: "按下热键开始录音时播放一段合成提示音，提醒已开始录音。胶囊隐藏时也会响。") {
                HStack(spacing: 10) {
                    OlToggle(on: model.audioCueOnRecord) {
                        model.audioCueOnRecord = $0
                        model.save(["audioCueOnRecord": $0])
                    }
                    Button("试听") { AudioCue.playRecordStart() }
                        .buttonStyle(OlGhostButtonStyle())
                }
            }
        }
    }

    private var hotkeyBinding: Binding<SettingsModel.ShortcutBinding?> {
        Binding(
            get: { model.dictationHotkey },
            set: { newValue in
                // 录音快捷键不可停用（disableDisabled）；set 里防御性忽略 nil。
                if let newValue {
                    model.dictationHotkey = newValue
                    saveHotkey(newValue)
                }
            }
        )
    }

    private func saveHotkey(_ binding: SettingsModel.ShortcutBinding) {
        let dict = try? JSONEncoder().encode(binding)
        let json = dict.flatMap { String(data: $0, encoding: .utf8) }
        if let json, let data = json.data(using: .utf8),
           let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
            model.save(["dictationHotkey": obj])
        }
    }

    // MARK: 快捷键速查（原版 ShortcutsSection）

    private var shortcutsCard: some View {
        OlCard {
            Text("快捷键速查")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Color.zhInk)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.bottom, 6)
            SettingRow(label: "开始 / 停止录音") {
                VStack(alignment: .leading, spacing: 6) {
                    ShortcutRecorderView(
                        binding: hotkeyBinding,
                        disableDisabled: true,
                        disableHint: "核心快捷键不可停用，录音必须绑定一个热键"
                    )
                    Text(modeSuffix)
                        .font(.system(size: 11))
                        .foregroundStyle(Color.zhInk4)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            SettingRow(label: "打开 准记") {
                ShortcutRecorderView(
                    binding: openAppBinding,
                    onReset: {
                        // 原版 defaultOpenAppShortcut()（mac：⌘⇧O）。
                        model.openAppHotkey = SettingsModel.ShortcutBinding(
                            primary: "O", modifiers: ["cmd", "shift"])
                        saveOpenApp(model.openAppHotkey)
                    },
                    onDisable: { saveOpenApp(nil) }
                )
            }
            SettingRow(label: "取消本次录音") {
                ReadonlyKbd("Esc")
            }
            SettingRow(label: "胶囊确认插入") {
                ReadonlyKbd("点击右侧 ✓")
            }
        }
    }

    private var openAppBinding: Binding<SettingsModel.ShortcutBinding?> {
        Binding(
            get: { model.openAppHotkey },
            set: { model.openAppHotkey = $0; saveOpenApp($0) }
        )
    }

    private func saveOpenApp(_ binding: SettingsModel.ShortcutBinding?) {
        if let binding {
            let dict = try? JSONEncoder().encode(binding)
            let json = dict.flatMap { String(data: $0, encoding: .utf8) }
            if let json, let data = json.data(using: .utf8),
               let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
                model.save(["openAppHotkey": obj])
            }
        } else {
            model.save(["openAppHotkey": NSNull()])
        }
    }

    private var modeSuffix: String {
        switch model.hotkeyMode {
        case "hold": return "（按住说话）"
        case "auto": return "（自动识别）"
        default: return "（开始 / 停止）"
        }
    }
}

// MARK: - 隐私 tab

struct SettingsPrivacyView: View {
    @ObservedObject var model = SettingsModel.shared
    @StateObject var permissions = PermissionModel()
    @State private var permissionTimer: Timer?
    @State private var networkTimer: Timer?

    var body: some View {
        ScrollView {
            VStack(spacing: 16) {
                localFirstBanner
                permissionsCard
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 24)
            .padding(.bottom, 8)
        }
        .onAppear {
            permissions.refresh()
            model.loadHotkeyStatus()
            model.checkNetwork()
            // 原版：权限 10s 轮询 + 网络 30s 轮询；热键状态事件驱动，不轮询。
            permissionTimer = Timer.scheduledTimer(withTimeInterval: 10, repeats: true) { _ in
                permissions.refresh()
            }
            networkTimer = Timer.scheduledTimer(withTimeInterval: 30, repeats: true) { _ in
                model.checkNetwork()
            }
        }
        .onDisappear {
            permissionTimer?.invalidate()
            networkTimer?.invalidate()
        }
        .onReceive(NotificationCenter.default.publisher(
            for: NSApplication.didBecomeActiveNotification)) { _ in
            // 原版 focus/visibilitychange → 全量刷新。
            permissions.refresh()
            model.loadHotkeyStatus()
            model.checkNetwork()
        }
    }

    /// 本地优先说明条（原版 PrivacyTab 顶部蓝底条）。
    private var localFirstBanner: some View {
        HStack(alignment: .center, spacing: 10) {
            Text("本地优先")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Color.zhBlue)
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background(
                    Capsule().fill(Color.zhSurface)
                )
                .fixedSize()
            Text("录音可能会发送到你配置的云端服务商进行转写。")
                .font(.system(size: 11.5))
                .foregroundStyle(Color.zhInk3)
                .lineSpacing(1)
        }
        .padding(10)
        .padding(.horizontal, 2)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color.zhBlueSoft))
        .padding(.bottom, 2)
    }

    private var permissionsCard: some View {
        OlCard {
            Text("权限")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Color.zhInk)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.bottom, 6)
            micRow
            accessibilityRow
            hotkeyRow
            networkRow
        }
    }

    // MARK: 权限行（原版行内控件右对齐 flex-end）

    private var micRow: some View {
        SettingRow(label: "麦克风") {
            HStack(spacing: 8) {
                PermissionPill(status: permissions.microphone)
                switch permissions.microphone {
                case .noDevice:
                    OlGhostBtn("重试") { permissions.refresh() }
                case .granted:
                    EmptyView()
                case .denied:
                    OlGhostBtn("打开系统设置") { permissions.requestMicrophone() }
                case .notDetermined:
                    OlGhostBtn("授权") { permissions.requestMicrophone() }
                }
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
        }
    }

    private var accessibilityRow: some View {
        SettingRow(label: "辅助功能") {
            HStack(spacing: 8) {
                PermissionPill(status: permissions.accessibility)
                switch permissions.accessibility {
                case .denied:
                    // 已拒绝时 TCC 不再弹窗：引导去系统设置（原版 reRequestAccessibility）。
                    OlGhostBtn("打开系统设置") { permissions.grantAccessibility() }
                    OlGhostBtn("重置授权并重启") { resetAccessibilityAndRestart() }
                case .notDetermined:
                    OlGhostBtn("授权") { permissions.grantAccessibility() }
                case .granted, .noDevice:
                    EmptyView()
                }
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
        }
    }

    private var hotkeyRow: some View {
        SettingRow(label: "全局快捷键") {
            HStack(spacing: 8) {
                if !model.hotkeyMessage.isEmpty {
                    Text(model.hotkeyMessage)
                        .font(.system(size: 11.5))
                        .foregroundStyle(Color.zhInk4)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                HotkeyStatusPill(state: model.hotkeyState)
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
        }
    }

    private var networkRow: some View {
        SettingRow(label: "网络") {
            HStack(spacing: 8) {
                if let latency = model.networkLatencyMs {
                    Text("\(latency)ms")
                        .font(.system(size: 11))
                        .foregroundStyle(Color.zhInk4)
                }
                NetworkStatusPill(online: model.networkOnline)
                if model.networkOnline == false {
                    OlGhostBtn("重试") { model.checkNetwork() }
                }
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
        }
    }

    /// 重置辅助功能授权并重启 app（原版 reset_accessibility_permission_and_restart_app）。
    private func resetAccessibilityAndRestart() {
        let bundleId = Bundle.main.bundleIdentifier ?? ""
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/tccutil")
        process.arguments = ["reset", "Accessibility", bundleId]
        try? process.run()
        process.waitUntilExit()
        let config = NSWorkspace.OpenConfiguration()
        NSWorkspace.shared.openApplication(at: Bundle.main.bundleURL, configuration: config)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
            NSApp.terminate(nil)
        }
    }
}

// MARK: - 权限状态徽标

/// 权限状态 Pill（原版 PermissionPill：ok = 已授权，outline = 未授权/未检测到）。
struct PermissionPill: View {
    let status: PermissionStatus

    var body: some View {
        switch status {
        case .granted:
            OlPill(tone: .ok) {
                Image(systemName: "checkmark").font(.system(size: 11))
                Text("已授权")
            }
        case .noDevice:
            OlPill(tone: .outline) { Text("未检测到麦克风") }
        case .denied:
            OlPill(tone: .outline) { Text("未授权") }
        case .notDetermined:
            OlPill(tone: .outline) { Text("未授权") }
        }
    }
}

/// 热键状态 Pill（原版 HotkeyStatusPill）。
struct HotkeyStatusPill: View {
    let state: String

    var body: some View {
        switch state {
        case "installed":
            OlPill(tone: .ok) {
                Image(systemName: "checkmark").font(.system(size: 11))
                Text("已安装")
            }
        case "starting":
            OlPill(tone: .default) { Text("安装中…") }
        default:
            OlPill(tone: .outline) { Text("监听失败") }
        }
    }
}

/// 网络状态 Pill（原版 NetworkStatusPill）。
struct NetworkStatusPill: View {
    let online: Bool?

    var body: some View {
        if let online {
            if online {
                OlPill(tone: .ok) {
                    Image(systemName: "checkmark").font(.system(size: 11))
                    Text("可用")
                }
            } else {
                OlPill(tone: .outline) { Text("不可用") }
            }
        } else {
            OlPill(tone: .default) { Text("检查中…") }
        }
    }
}

// MARK: - 基础原子（原版 shared.tsx / _atoms.tsx）

/// 卡片：surface 白底 + 0.5px line 描边 + radius 14 + padding 18 + 微阴影。
/// padding 可调（原版 Card padding prop：概览页 14/16/0 等变体）。
struct OlCard<Content: View>: View {
    let content: Content
    var padding: CGFloat

    init(padding: CGFloat = 18, @ViewBuilder content: () -> Content) {
        self.padding = padding
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            content
        }
        .padding(padding)
        .frame(maxWidth: .infinity, alignment: .leading)
        // 不用 clipShape——下拉浮层（SelectLite 菜单）溢出卡片时不能被裁掉。
        .background(
            RoundedRectangle(cornerRadius: 14)
                .fill(Color.zhSurface)
                .shadow(color: Color.black.opacity(0.03), radius: 1, y: 1)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 14)
                .stroke(Color.zhLine, lineWidth: 0.5)
        )
    }
}

/// 折叠栏（原版 _atoms Collapsible）：默认收起，标题行右侧 › 箭头点击展开/收起，
/// 展开时箭头旋转 90°。非 embedded 模式自带 Card 同款外观（0.5px 描边 / r14 / surface）。
struct OlCollapsible<Content: View>: View {
    let title: String
    let content: Content

    @State private var open = false

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(spacing: 0) {
            Button {
                withAnimation(.easeInOut(duration: 0.22)) { open.toggle() }
            } label: {
                HStack(spacing: 12) {
                    Text(title)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Color.zhInk)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    Image(systemName: "chevron.right")
                        .font(.system(size: 14, weight: .medium))
                        .foregroundStyle(Color.zhInk4)
                        .rotationEffect(.degrees(open ? 90 : 0))
                        .animation(.easeInOut(duration: 0.18), value: open)
                }
                .padding(.horizontal, 18)
                .padding(.vertical, 14)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if open {
                VStack(alignment: .leading, spacing: 0) {
                    content
                }
                .padding(.horizontal, 18)
                .padding(.bottom, 18)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .background(
            RoundedRectangle(cornerRadius: 14)
                .fill(Color.zhSurface)
                .shadow(color: Color.black.opacity(0.03), radius: 1, y: 1)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 14)
                .stroke(Color.zhLine, lineWidth: 0.5)
        )
    }
}

/// 开机自启行（原版 AutostartRow）：状态由 OS 持有（LaunchAgent plist），
/// 不存 prefs；切换失败时行内红字提示。
struct AutostartRow: View {
    @State private var enabled = false
    @State private var loaded = false
    @State private var errorMessage: String?

    var body: some View {
        SettingRow(label: "开机自启", hint: "登录系统时自动启动 准记。") {
            VStack(alignment: .leading, spacing: 4) {
                if loaded {
                    OlToggle(on: enabled, onToggle: toggle)
                }
                if let errorMessage {
                    Text(errorMessage)
                        .font(.system(size: 11))
                        .foregroundStyle(Color.red)
                        .lineLimit(2)
                        .padding(.top, 4)
                }
            }
        }
        .onAppear { load() }
    }

    private func load() {
        let v = Autostart.isEnabled()
        enabled = v
        loaded = true
    }

    private func toggle(_ next: Bool) {
        enabled = next
        errorMessage = nil
        do {
            if next {
                try Autostart.enable()
            } else {
                try Autostart.disable()
            }
        } catch {
            enabled = !next
            errorMessage = "开机自启切换失败：\(error.localizedDescription)"
        }
    }
}

/// 区块标题：14/600/ink + marginBottom 6。带 hint 时虚线下划线（悬停显示说明）。
struct SectionTitle<Content: View>: View {
    var hint: String? = nil
    let content: Content

    init(hint: String? = nil, @ViewBuilder content: () -> Content) {
        self.hint = hint
        self.content = content()
    }

    var body: some View {
        if let hint {
            content
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(Color.zhInk)
                .underline(true, color: Color.zhInk4)
                .help(hint)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.bottom, 6)
        } else {
            content
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(Color.zhInk)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.bottom, 6)
        }
    }
}

/// 设置行：label 固定 180px 左列 + 16px gap + control 1fr；
/// padding 14px 0；顶部 0.5px line-soft 分隔。带 hint 时标签虚线下划线悬停说明。
struct SettingRow<Control: View>: View {
    let label: String
    var hint: String? = nil
    let control: Control

    init(label: String, hint: String? = nil, @ViewBuilder control: () -> Control) {
        self.label = label
        self.hint = hint
        self.control = control()
    }

    var body: some View {
        HStack(spacing: 16) {
            if let hint {
                Text(label)
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(Color.zhInk)
                    .underline(true, color: Color.zhInk4)
                    .help(hint)
                    .frame(width: 180, alignment: .leading)
            } else {
                Text(label)
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(Color.zhInk)
                    .frame(width: 180, alignment: .leading)
            }
            control
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.vertical, 14)
        .overlay(alignment: .top) {
            Rectangle()
                .fill(Color.zhLineSoft)
                .frame(height: 0.5)
        }
    }
}

/// 开关（原版 Toggle：36×20 圆钮，on 蓝 / off 半透明黑，白 knob 16×16）。
struct OlToggle: View {
    let on: Bool
    let onToggle: (Bool) -> Void

    var body: some View {
        Button {
            onToggle(!on)
        } label: {
            ZStack(alignment: .leading) {
                Capsule()
                    .fill(on ? Color.zhBlue : Color.zhToggleOffBG)
                Circle()
                    .fill(Color.zhToggleKnob)
                    .frame(width: 16, height: 16)
                    .padding(2)
                    .offset(x: on ? 16 : 0)
            }
            .frame(width: 36, height: 20)
        }
        .buttonStyle(.plain)
        .animation(.easeOut(duration: 0.16), value: on)
    }
}

/// 分段控件（原版 segmentedTrackStyle：track 2px 内边距 r8 + 选项按钮 r6）。
struct OlSegmented: View {
    let options: [(id: String, label: String)]
    let selected: String
    let onSelect: (String) -> Void

    var body: some View {
        HStack(spacing: 0) {
            ForEach(options, id: \.id) { option in
                let isActive = option.id == selected
                Button {
                    onSelect(option.id)
                } label: {
                    Text(option.label)
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(isActive ? Color.zhInk : Color.zhInk3)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 5)
                        .background(
                            RoundedRectangle(cornerRadius: 6)
                                .fill(isActive ? Color.zhSegmentedActiveBG : Color.clear)
                                .shadow(
                                    color: isActive ? Color.zhSegmentedActiveShadow : .clear,
                                    radius: 1, y: 1
                                )
                        )
                }
                .buttonStyle(.plain)
                .animation(.easeOut(duration: 0.16), value: selected)
            }
        }
        .padding(2)
        .background(RoundedRectangle(cornerRadius: 8).fill(Color.zhSegmentedBG))
    }
}

/// 状态徽标（原版 Pill：default/blue/ok/outline 四 tone，sm/md 两尺寸）。
struct OlPill<Content: View>: View {
    enum Tone { case `default`, blue, ok, outline }
    enum Size { case sm, md }
    let tone: Tone
    var size: Size = .md
    @ViewBuilder let content: Content

    var body: some View {
        HStack(spacing: 6) {
            content
        }
        .font(.system(size: size == .sm ? 10.5 : 11.5, weight: .medium))
        .foregroundStyle(foreground)
        .padding(.horizontal, size == .sm ? 8 : 10)
        .padding(.vertical, size == .sm ? 2 : 4)
        .background(Capsule().fill(background))
        .overlay(Capsule().strokeBorder(border, lineWidth: 0.5))
        .fixedSize()
    }

    private var background: Color {
        switch tone {
        case .default: return Color.zhPillDefaultBG
        case .blue: return Color.zhPillBlueBG
        case .ok: return Color.zhPillOKBG
        case .outline: return Color.clear
        }
    }

    private var foreground: Color {
        switch tone {
        case .default: return Color.zhInk2
        case .blue: return Color.zhBlue
        case .ok: return Color.zhOK
        case .outline: return Color.zhInk3
        }
    }

    private var border: Color {
        switch tone {
        case .default: return .clear
        case .blue: return .clear
        case .ok: return .clear
        case .outline: return Color.zhLineStrong
        }
    }
}

/// 幽灵小按钮（原版 Btn ghost sm：r8 + 0.5px line-strong 描边，disabled 0.55）。
struct OlGhostButtonStyle: ButtonStyle {
    @Environment(\.isEnabled) private var isEnabled
    var hPadding: CGFloat = 10
    var vPadding: CGFloat = 5

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 12, weight: .medium))
            .foregroundStyle(Color.zhInk2)
            .padding(.horizontal, hPadding)
            .padding(.vertical, vPadding)
            .background(
                RoundedRectangle(cornerRadius: 8)
                    .fill(Color.clear)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 8)
                    .stroke(Color.zhLineStrong, lineWidth: 0.5)
            )
            .opacity(configuration.isPressed ? 0.6 : (isEnabled ? 1 : 0.55))
    }
}

/// 幽灵小按钮便捷视图（OlGhostButtonStyle + 文案）。
struct OlGhostBtn: View {
    let label: String
    let action: () -> Void

    init(_ label: String, action: @escaping () -> Void) {
        self.label = label
        self.action = action
    }

    var body: some View {
        Button(label, action: action)
            .buttonStyle(OlGhostButtonStyle())
    }
}

/// 输入框（原版 inputStyle：h32 r8 + 0.5px line-strong + surface-2 底）。
struct OlInputStyle: TextFieldStyle {
    func _body(configuration: TextField<Self._Label>) -> some View {
        configuration
            .font(.system(size: 12.5))
            .foregroundStyle(Color.zhInk)
            .padding(.horizontal, 10)
            .frame(height: 32)
            .background(RoundedRectangle(cornerRadius: 8).fill(Color.zhSurface2))
            .overlay(
                RoundedRectangle(cornerRadius: 8)
                    .stroke(Color.zhLineStrong, lineWidth: 0.5)
            )
    }
}

/// 键帽（原版 Kbd：h21 min-w20 r5 + 0.5px 描边 + 底边立体阴影）。
struct Kbd: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(text)
            .font(.system(size: 11.5, weight: .medium))
            .foregroundStyle(Color.zhInk2)
            .padding(.horizontal, 6)
            .frame(minWidth: 20, maxHeight: 21)
            .background(
                RoundedRectangle(cornerRadius: 5)
                    .fill(Color.zhSurface2)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 5)
                    .stroke(Color.zhLineStrong, lineWidth: 0.5)
            )
            .shadow(color: Color.zhLine, radius: 0, y: 1.5)
            .fixedSize()
    }
}

/// 键帽组（原版 KbdGroup：gap 4 并排）。
struct KbdGroup: View {
    let keys: [String]

    var body: some View {
        HStack(spacing: 4) {
            ForEach(Array(keys.enumerated()), id: \.offset) { _, key in
                Kbd(key)
            }
        }
    }
}

/// 只读 kbd（原版快捷键速查 readonlyRows 样式：padding 4/10、12px 等宽、
/// r6、0 1px 0 rgba(0,0,0,0.04) 底阴影）。
struct ReadonlyKbd: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(text)
            .font(.system(size: 12, design: .monospaced))
            .foregroundStyle(Color.zhInk2)
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .background(
                RoundedRectangle(cornerRadius: 6)
                    .fill(Color.zhSurface2)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 6)
                    .stroke(Color.zhLineStrong, lineWidth: 0.5)
            )
            .shadow(color: Color.black.opacity(0.04), radius: 0, y: 1)
            .fixedSize()
    }
}

// MARK: - 下拉选择（原版 SelectLite）

struct OlSelectOption: Identifiable {
    let id: String
    let label: String
    var trailing: AnyView? = nil

    init(value: String, label: String, trailing: AnyView? = nil) {
        self.id = value
        self.label = label
        self.trailing = trailing
    }
}

// MARK: - 下拉选择（原版 SelectLite，定位算法 1:1）

/// 触发器屏幕 frame 上报（左下原点屏幕坐标，AppKit 权威转换链：
/// convert(to: nil) → 窗口坐标 → convertPoint(toScreen:) → 屏幕坐标。
/// 不用 SwiftUI .global——其原点语义随窗口结构浮动，是「下拉漂移」的根源）。
struct TriggerFrameProxy: NSViewRepresentable {
    var onFrame: (NSRect) -> Void

    func makeNSView(context: Context) -> NSView { NSView() }

    func updateNSView(_ nsView: NSView, context: Context) {
        DispatchQueue.main.async {
            guard let window = nsView.window else { return }
            let boundsInWindow = nsView.convert(nsView.bounds, to: nil)
            let origin = window.convertPoint(toScreen: boundsInWindow.origin)
            onFrame(NSRect(
                x: origin.x,
                y: origin.y - nsView.bounds.height,
                width: nsView.bounds.width,
                height: nsView.bounds.height
            ))
        }
    }
}

/// 菜单浮层窗口（NSPanel，透明无边框，自绘内容）。
/// 独立窗口 = 独立渲染层——展开时永不被兄弟视图/卡片遮挡（原版 popover 行为）。
/// 定位 1:1 复刻原版 SelectLite.positionPopover（viewport 换算为左下系屏幕坐标）：
///   - 纵向：触发器正下方 4px；下方空间不足（< 高+8）且上方够 → 翻转到上方 4px
///   - 横向：clamp(触发器 left, 8, 屏宽 - 触发器宽 - 8)
///   - 宽度 = 触发器宽度；两阶段定位：先按估算高度，内容真实高度上报后重定位
/// 关闭行为：点击外部 / 外部滚轮滚动 / 窗口 resize / Esc / Tab；
/// 键盘：↑↓ 移动高亮（初始 = 当前选中项）、Return 确认；退出 140ms pop-out 动画。
@MainActor
final class SelectMenuPanel: NSPanel, ObservableObject {
    static let shared = SelectMenuPanel()

    @Published var options: [OlSelectOption] = []
    @Published var highlight = 0
    @Published var leaving = false
    @Published private(set) var isPresenting = false

    private var selectedIndex = 0
    private var anchor: NSRect = .zero // 触发器屏幕坐标（左下原点）
    private var onChange: ((String) -> Void)?
    private var onClose: (() -> Void)?
    private var clickMonitor: Any?
    private var keyMonitor: Any?
    private var resizeObserver: NSObjectProtocol?
    private var closeWork: DispatchWorkItem?

    /// 当前选中项 id（content 判蓝字/勾）。
    var selectedID: String? {
        options.indices.contains(selectedIndex) ? options[selectedIndex].id : nil
    }

    /// 面板宽度 = 触发器宽度（原版 width: anchor.width）。
    var anchorWidth: CGFloat { anchor.width }

    /// 内容真实高度上报（两阶段定位：估算 → 重定位）。
    var contentHeight: CGFloat = 0 {
        didSet {
            guard isPresenting, !leaving, abs(contentHeight - frame.height) > 0.5 else { return }
            setFrame(computeFrame(anchor: anchor, height: contentHeight), display: true)
        }
    }

    init() {
        super.init(
            contentRect: .zero,
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        isFloatingPanel = true
        level = .floating
        backgroundColor = .clear
        isOpaque = false
        hasShadow = false // 阴影自绘
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        isReleasedWhenClosed = false
    }

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    /// 展开菜单（幂等；已打开时忽略）。
    func present(
        anchor: NSRect,
        options: [OlSelectOption],
        selected: String,
        onChange: @escaping (String) -> Void,
        onClose: (() -> Void)?
    ) {
        guard !isPresenting else { return }
        self.anchor = anchor
        self.options = options
        self.selectedIndex = options.firstIndex { $0.id == selected } ?? 0
        self.highlight = selectedIndex // 初始高亮 = 当前选中项（原版 useEffect [open]）
        self.onChange = onChange
        self.onClose = onClose
        leaving = false
        closeWork?.cancel()

        let hosting = NSHostingView(rootView: SelectMenuContent(panel: self))
        let estHeight = min(CGFloat(options.count) * 34 + 8, 288)
        hosting.frame = NSRect(origin: .zero, size: NSSize(width: anchor.width, height: estHeight))
        contentView = hosting
        setFrame(computeFrame(anchor: anchor, height: estHeight), display: true)
        orderFrontRegardless()
        isPresenting = true
        installMonitors()
    }

    /// 关闭（幂等）：播 140ms pop-out 动画后真正收起（原版 leaving 状态）。
    func closeMenu() {
        guard isPresenting else { return }
        isPresenting = false
        leaving = true
        let work = DispatchWorkItem { [weak self] in self?.finalizeClose() }
        closeWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.14, execute: work)
    }

    /// 立即收起（视图消失等场景，不播动画）。
    func dismissNow() {
        guard isPresenting else { return }
        isPresenting = false
        closeWork?.cancel()
        leaving = false
        orderOut(nil)
        removeMonitors()
        let cb = onClose
        onClose = nil
        onChange = nil
        cb?()
    }

    /// 点击位置（屏幕坐标，左下原点）是否落在菜单内。
    func contains(_ point: NSPoint) -> Bool {
        frame.contains(point)
    }

    /// 选中索引（Return / 点击选项）。
    func select(_ index: Int) {
        guard options.indices.contains(index) else { return }
        onChange?(options[index].id)
        closeMenu()
    }

    // MARK: - 定位（原版 SelectLite.positionPopover）

    /// 原版算法：spaceBelow < popoverHeight + 8 且上方够高 → 翻转到上方；
    /// 否则正下方 4px。横向 clamp 8px 边距。宽度 = 触发器宽度。
    private func computeFrame(anchor: NSRect, height: CGFloat) -> NSRect {
        let sf = anchorScreenFrame()
        let spaceBelow = anchor.minY - 4
        let spaceAbove = sf.maxY - anchor.maxY
        let flipUp = spaceBelow < height + 8 && spaceAbove > height + 8
        let y = flipUp ? anchor.maxY + 4 + height : anchor.minY - 4
        let minLeft: CGFloat = 8
        let maxLeft = max(minLeft, sf.maxX - anchor.width - 8)
        let x = min(max(anchor.minX, minLeft), maxLeft)
        return NSRect(x: x, y: y, width: anchor.width, height: height)
    }

    private func anchorScreenFrame() -> NSRect {
        // 锚点所在屏（多屏时菜单贴触发器所在屏）。
        if let screen = NSScreen.screens.first(where: { $0.frame.insetBy(dx: -1, dy: -1).contains(anchor.origin) }) {
            return screen.frame
        }
        return NSScreen.main?.frame ?? .zero
    }

    // MARK: - 事件（点击外部 / 滚轮 / resize / 键盘）

    private func installMonitors() {
        // 点击外部 + 滚轮外部：放行点 = 面板自身 / 触发器（toggle 由触发器按钮处理）。
        clickMonitor = NSEvent.addLocalMonitorForEvents(
            matching: [.leftMouseDown, .rightMouseDown, .scrollWheel]
        ) { [weak self] event in
            guard let self else { return event }
            let point = NSEvent.mouseLocation // 屏幕坐标（左下原点）
            if event.type == .scrollWheel {
                // 菜单内滚动（选项滚动）放行；外部滚动关闭（原版 handleScrollOutside）。
                if frame.contains(point) { return event }
                closeMenu()
                return event
            }
            if event.window === self { return event }
            if anchor.contains(point) { return event }
            closeMenu()
            return event // 不吞事件，点击继续作用于下层（原版行为）
        }
        // 键盘：Esc/Tab 关闭，↑↓ 高亮移动（循环），Return 确认。
        keyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self else { return event }
            switch event.keyCode {
            case 53, 48: // Esc / Tab
                closeMenu()
            case 126: // ↑
                let n = options.count
                guard n > 0 else { return nil }
                highlight = (highlight - 1 + n) % n
            case 125: // ↓
                let n = options.count
                guard n > 0 else { return nil }
                highlight = (highlight + 1) % n
            case 36: // Return
                select(highlight)
            default:
                return event
            }
            return nil // 吞掉，菜单开着时键盘只服务菜单（原版 preventDefault）
        }
        // 窗口 resize → 关闭（原版 window resize 关闭）。面板自身两阶段重定位
        // 也会发 resize 通知，object === self 时忽略。
        resizeObserver = NotificationCenter.default.addObserver(
            forName: NSWindow.didResizeNotification,
            object: nil,
            queue: .main
        ) { [weak self] note in
            guard let self, (note.object as? NSWindow) !== self, isPresenting else { return }
            closeMenu()
        }
    }

    private func removeMonitors() {
        if let clickMonitor {
            NSEvent.removeMonitor(clickMonitor)
            self.clickMonitor = nil
        }
        if let keyMonitor {
            NSEvent.removeMonitor(keyMonitor)
            self.keyMonitor = nil
        }
        if let resizeObserver {
            NotificationCenter.default.removeObserver(resizeObserver)
            self.resizeObserver = nil
        }
    }

    private func finalizeClose() {
        orderOut(nil)
        removeMonitors()
        let cb = onClose
        onClose = nil
        onChange = nil
        cb?()
    }
}

/// 菜单内容（panel 宿主，每次展开重建；hover 与键盘共用 panel.highlight）。
private struct SelectMenuContent: View {
    @ObservedObject var panel: SelectMenuPanel
    @State private var appeared = false

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView(.vertical, showsIndicators: true) {
                VStack(spacing: 0) {
                    ForEach(Array(panel.options.enumerated()), id: \.offset) { index, option in
                        MenuRow(panel: panel, option: option, index: index)
                            .id(index)
                    }
                }
                .padding(4)
            }
            .frame(width: panel.anchorWidth)
            .frame(maxHeight: 280) // 原版 popover maxHeight 280，超出滚动
            .background(
                RoundedRectangle(cornerRadius: 10)
                    .fill(Color.zhSelectPopoverBG)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 10)
                    .stroke(Color.zhLineStrong.opacity(0.45), lineWidth: 0.5)
            )
            .shadow(color: Color.black.opacity(0.2), radius: 12, y: 4)
            .scaleEffect(!panel.leaving && appeared ? 1 : 0.96, anchor: .top)
            .opacity(!panel.leaving && appeared ? 1 : 0)
            .animation(.easeOut(duration: 0.14), value: appeared)
            .animation(.easeIn(duration: 0.14), value: panel.leaving)
            .onAppear {
                // 进入动画（pop .14s，transformOrigin top center）。
                withAnimation(.easeOut(duration: 0.14)) { appeared = true }
            }
            .background(GeometryReader { geo in
                Color.clear
                    .onAppear { panel.contentHeight = geo.size.height }
                    .onChange(of: geo.size.height) { h in panel.contentHeight = h }
            })
            .onChange(of: panel.highlight) { _ in
                withAnimation(.none) { proxy.scrollTo(panel.highlight, anchor: .center) }
            }
        }
    }
}

/// 单行选项：hover 高亮 + 选中蓝字加粗 + trailing + 勾（原版 SelectLite option）。
private struct MenuRow: View {
    @ObservedObject var panel: SelectMenuPanel
    let option: OlSelectOption
    let index: Int

    private var isSelected: Bool { option.id == panel.selectedID }
    private var isHighlighted: Bool { panel.highlight == index }

    var body: some View {
        Button {
            panel.select(index)
        } label: {
            HStack(spacing: 8) {
                Text(option.label)
                    .font(.system(size: 12.5, weight: isSelected ? .semibold : .medium))
                    .foregroundStyle(isSelected ? Color.zhBlue : Color.zhInk)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, alignment: .leading)
                option.trailing
                if isSelected {
                    Image(systemName: "checkmark")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Color.zhBlue)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(
                RoundedRectangle(cornerRadius: 6)
                    .fill(isHighlighted ? Color.zhSelectOptionHover : Color.clear)
            )
            .contentShape(RoundedRectangle(cornerRadius: 6))
        }
        .buttonStyle(.plain)
        .onHover { hovering in
            // 原版 onMouseEnter → setHighlight(index)：hover 与键盘共用高亮。
            if hovering { panel.highlight = index }
        }
    }
}

/// 自定义下拉：触发器（h32 r8 + 0.5px 描边 + chevron）+ SelectMenuPanel 浮层菜单。
/// 触发器屏幕 frame 由 TriggerFrameProxy（AppKit 转换链）上报，打开时交给面板定位；
/// 点击外部 / 滚轮 / resize / 键盘由面板统一监听（原版 SelectLite 行为）。
struct OlSelectLite: View {
    let value: String
    let options: [OlSelectOption]
    var onChange: (String) -> Void
    var onOpenChange: ((Bool) -> Void)? = nil
    var width: CGFloat = 200

    @State private var open = false
    @State private var triggerFrame: NSRect?

    var body: some View {
        Button {
            toggleMenu()
        } label: {
            HStack(spacing: 8) {
                Text(selectedLabel)
                    .font(.system(size: 12.5))
                    .foregroundStyle(Color.zhInk)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Image(systemName: "chevron.down")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Color.zhInk4)
                    .rotationEffect(.degrees(open ? 180 : 0))
            }
            .padding(.horizontal, 10)
            .frame(height: 32)
            .background(
                RoundedRectangle(cornerRadius: 8)
                    .fill(Color.zhSurface)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 8)
                    .stroke(Color.zhLineStrong, lineWidth: 0.5)
            )
            .frame(width: width)
        }
        .buttonStyle(.plain)
        .background(TriggerFrameProxy { frame in
            triggerFrame = frame
        })
        .onDisappear {
            guard open else { return }
            open = false
            SelectMenuPanel.shared.dismissNow()
        }
    }

    private var selectedLabel: String {
        options.first { $0.id == value }?.label ?? ""
    }

    private func toggleMenu() {
        // 用 open state 而非 isPresenting：关闭动画（140ms）期间 open 仍为 true，
        // 重复点击走 closeMenu 幂等分支（原版 leaving 期间同样忽略）。
        open ? closeMenu() : openMenu()
    }

    private func openMenu() {
        guard let triggerFrame else { return }
        open = true
        onOpenChange?(true)
        SelectMenuPanel.shared.present(
            anchor: triggerFrame,
            options: options,
            selected: value,
            onChange: onChange,
            onClose: {
                self.open = false
                self.onOpenChange?(false)
            }
        )
    }

    private func closeMenu() {
        SelectMenuPanel.shared.closeMenu()
        // onClose 回调（140ms 动画后）里复位 open。
    }
}

// MARK: - 麦克风选择（原版 MicrophoneSelect + LevelMeter）

/// 麦克风下拉：系统默认 + 设备列表，选中项右侧挂实时电平条 + 勾。
struct MicrophoneSelectView: View {
    let devices: [SettingsModel.MicDevice]
    let selectedName: String
    let onSelect: (String) -> Void
    let onOpen: () -> Void

    @ObservedObject var model = SettingsModel.shared
    @State private var options: [OlSelectOption] = []

    var body: some View {
        OlSelectLite(
            value: selectedName,
            options: options,
            onChange: onSelect,
            onOpenChange: { opening in
                if opening {
                    onOpen()
                    model.startLevelMonitor()
                } else {
                    model.stopLevelMonitor()
                }
            },
            width: 200
        )
        .onChange(of: model.micLevel) { _ in
            rebuildOptions()
        }
        .onAppear { rebuildOptions() }
        .onChange(of: devices.map(\.name)) { _ in rebuildOptions() }
    }

    private func rebuildOptions() {
        var built: [OlSelectOption] = [
            OlSelectOption(
                value: "",
                label: "系统默认麦克风",
                trailing: selectedName == ""
                    ? AnyView(LevelMeterView(level: model.micLevel))
                    : nil
            ),
        ]
        for device in devices {
            built.append(OlSelectOption(
                value: device.name,
                label: device.name,
                trailing: selectedName == device.name
                    ? AnyView(LevelMeterView(level: model.micLevel))
                    : nil
            ))
        }
        options = built
    }
}

/// 电平条（原版 LevelMeter：5 根竖条，level*4.5 放大，条高/透明度随强度）。
struct LevelMeterView: View {
    let level: Double

    private let bars: [Double] = [0.4, 0.7, 1, 0.7, 0.4]

    var body: some View {
        HStack(spacing: 3) {
            ForEach(Array(bars.enumerated()), id: \.offset) { _, weight in
                let amplified = min(1, max(0, level * 4.5))
                let intensity = min(1, amplified * (0.85 + weight * 0.35))
                let height = 4 + intensity * 10 * weight
                RoundedRectangle(cornerRadius: 999)
                    .fill(intensity > 0.08 ? Color.zhBlue : Color.black.opacity(0.14))
                    .frame(width: 3, height: height)
                    .opacity(0.4 + intensity * 0.6)
                    .animation(.linear(duration: 0.07), value: level)
            }
        }
        .frame(height: 14)
        .fixedSize()
    }
}

// MARK: - 热键录制（原版 ShortcutRecorder）

/// 热键录制：主行 = 键帽组 + 右侧 chevron 菜单（录制快捷键 / 重置 / 停用），
/// 录制态 = 蓝底面板「请按下快捷键组合…」+「Esc 取消」。
/// 键名与 core parse_primary / legacy_modifier_trigger 对齐；显示映射照抄
/// 原版 formatPrimary / sideModifierDisplayName（mac）。
struct ShortcutRecorderView: View {
    @Binding var binding: SettingsModel.ShortcutBinding?
    var disableDisabled = false
    var disableHint: String? = nil
    var onReset: (() -> Void)? = nil
    var onDisable: (() -> Void)? = nil

    @State private var isRecording = false
    @State private var menuOpen = false
    @State private var error: String? = nil
    @State private var monitor: Any?
    /// 单修饰键 pending（原版 pendingModifier）：按下修饰键后 650ms 无其他键 → 确认；
    /// 期间按了字符键 → 转组合键；期间松开 → 立即确认。
    @State private var pendingModifier: SettingsModel.ShortcutBinding?
    @State private var pendingTimer: Timer?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if isRecording {
                recordingPanel
            } else {
                idleRow
                if menuOpen {
                    menuRow
                }
            }
            if let error {
                Text(error)
                    .font(.system(size: 11))
                    .foregroundStyle(Color.zhErr)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// 主行：键帽 + 右侧 chevron 按钮（原版 recorderRow + chevronButton）。
    private var idleRow: some View {
        HStack(spacing: 8) {
            if let binding {
                KbdGroup(keys: Self.comboParts(binding))
            }
            Spacer(minLength: 0)
            Button {
                withAnimation(.easeOut(duration: 0.16)) {
                    menuOpen.toggle()
                }
            } label: {
                Image(systemName: "chevron.down")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(Color.zhInk4)
                    .rotationEffect(.degrees(menuOpen ? 180 : 0))
                    .frame(width: 26, height: 26)
                    .background(
                        RoundedRectangle(cornerRadius: 6)
                            .fill(Color.clear)
                    )
            }
            .buttonStyle(.plain)
            .help("更多操作")
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// 菜单行：录制快捷键（primary 蓝）/ 重置 / 停用（原版 menuRow）。
    private var menuRow: some View {
        HStack(spacing: 8) {
            Button {
                startRecording()
            } label: {
                Text("录制快捷键")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(Color.zhBlue)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 5)
                    .background(
                        RoundedRectangle(cornerRadius: 6)
                            .fill(Color.zhBlue.opacity(0.08))
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: 6)
                            .stroke(Color.zhBlue.opacity(0.25), lineWidth: 0.5)
                    )
            }
            .buttonStyle(.plain)
            if let onReset {
                Button { onReset(); menuOpen = false } label: {
                    Text("重置")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(Color.zhInk2)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 5)
                        .background(
                            RoundedRectangle(cornerRadius: 6)
                                .fill(Color.clear)
                        )
                        .overlay(
                            RoundedRectangle(cornerRadius: 6)
                                .stroke(Color.zhLineStrong, lineWidth: 0.5)
                        )
                }
                .buttonStyle(.plain)
            }
            let canDisable = onDisable != nil && !disableDisabled
            Button {
                if canDisable { onDisable?(); menuOpen = false }
            } label: {
                Text("停用")
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(Color.zhInk2)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 5)
                    .background(
                        RoundedRectangle(cornerRadius: 6)
                            .fill(Color.clear)
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: 6)
                            .stroke(Color.zhLineStrong, lineWidth: 0.5)
                    )
                    .opacity(canDisable ? 1 : 0.45)
            }
            .buttonStyle(.plain)
            .help(canDisable ? "" : (disableHint ?? ""))
        }
        .transition(.opacity.combined(with: .move(edge: .top)))
    }

    /// 录制面板：蓝淡底 + 1px 蓝描边（原版 motion.div 录制态）。
    private var recordingPanel: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("请按下快捷键组合…")
                .font(.system(size: 12))
                .foregroundStyle(Color.zhBlue)
            Text("Esc 取消")
                .font(.system(size: 11))
                .foregroundStyle(Color.zhInk4)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(Color.zhBlue.opacity(0.06))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.zhBlue.opacity(0.2), lineWidth: 1)
        )
        .transition(.opacity.combined(with: .offset(x: 48)))
        .onAppear {
            startKeyMonitor()
        }
        .onDisappear {
            removeKeyMonitor()
        }
    }

    // MARK: 录制逻辑

    private func startRecording() {
        menuOpen = false
        error = nil
        isRecording = true
    }

    private func startKeyMonitor() {
        // 修饰键（Option/Command/Control/Shift）单独按下只产生 .flagsChanged、
        // 不进 keyDown——两个都要监听才能录「右 Option」这类单修饰键
        // （原版 DOM keydown 能收到修饰键，isModifierKey 分支）。
        monitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .flagsChanged]) { event in
            handleKeyDown(event)
            return nil // 录制期间吞掉按键
        }
    }

    private func handleKeyDown(_ event: NSEvent) {
        if event.type == .flagsChanged {
            handleModifierFlagsChanged(event)
            return
        }
        if event.keyCode == 53 { // Esc 取消
            clearPendingModifier()
            isRecording = false
            error = nil
            return
        }
        guard let primary = Self.keyName(for: event.keyCode) else {
            return // 纯修饰键或未知键：继续等
        }
        // 字符键按下 → 取消 pending 的单修饰键，转组合键（原版 clearPendingModifier）。
        clearPendingModifier()
        var mods: [String] = []
        let flags = event.modifierFlags
        if flags.contains(.command) { mods.append("cmd") }
        if flags.contains(.option) { mods.append("option") }
        if flags.contains(.control) { mods.append("ctrl") }
        if flags.contains(.shift) { mods.append("shift") }
        binding = SettingsModel.ShortcutBinding(primary: primary, modifiers: mods)
        isRecording = false
        error = nil
    }

    /// 修饰键按下/松开（flagsChanged）：对齐原版 onKeyDown isModifierKey 分支 +
    /// onKeyUp 立即确认。按下 → 650ms pending；期间松开或超时 → 设为单修饰键热键。
    private func handleModifierFlagsChanged(_ event: NSEvent) {
        guard let name = Self.modifierName(for: event.keyCode) else { return }
        let isDown = event.modifierFlags.contains(Self.modifierFlag(for: event.keyCode))
        if isDown {
            // 原版：pending 的 primary 相同（自动重复）→ 忽略；不同 → 替换 pending。
            guard pendingModifier?.primary != name else { return }
            let candidate = SettingsModel.ShortcutBinding(primary: name, modifiers: [])
            pendingModifier = candidate
            pendingTimer?.invalidate()
            pendingTimer = Timer.scheduledTimer(withTimeInterval: 0.65, repeats: false) { _ in
                // @State setter 为 nonmutating（写共享 storage），闭包捕获的 struct
                // 拷贝上写入依然落到真状态；无需 weak（struct 非引用类型）。
                guard pendingModifier?.primary == name else { return }
                finishRecording(candidate)
            }
        } else {
            // 松开 → 原版 onKeyUp：pending 匹配则立即确认。
            guard pendingModifier?.primary == name else { return }
            clearPendingModifier()
            finishRecording(SettingsModel.ShortcutBinding(primary: name, modifiers: []))
        }
    }

    private func clearPendingModifier() {
        pendingTimer?.invalidate()
        pendingTimer = nil
        pendingModifier = nil
    }

    private func finishRecording(_ newBinding: SettingsModel.ShortcutBinding) {
        binding = newBinding
        isRecording = false
        error = nil
    }

    /// 修饰键 keyCode → core 键名（原版 modifierPrimaryFromCode，左右侧区分）。
    /// macOS：55=左⌘ 54=右⌘ 58=左⌥ 61=右⌥ 59=左⌃ 62=右⌃ 56=左⇧ 60=右⇧ 63=Fn。
    static func modifierName(for keyCode: UInt16) -> String? {
        switch keyCode {
        case 55: return "LeftCommand"
        case 54: return "RightCommand"
        case 58: return "LeftOption"
        case 61: return "RightOption"
        case 59: return "LeftControl"
        case 62: return "RightControl"
        case 56: return "LeftShift"
        case 60: return "RightShift"
        case 63: return "Fn"
        default: return nil
        }
    }

    /// 该修饰键对应的 flag 位（flagsChanged 事件里判断按下/松开）。
    static func modifierFlag(for keyCode: UInt16) -> NSEvent.ModifierFlags {
        switch keyCode {
        case 55, 54: return .command
        case 58, 61: return .option
        case 59, 62: return .control
        case 56, 60: return .shift
        case 63: return .function
        default: return []
        }
    }

    private func removeKeyMonitor() {
        if let monitor {
            NSEvent.removeMonitor(monitor)
            self.monitor = nil
        }
    }

    // MARK: 显示映射（对齐原版 formatPrimary / sideModifierDisplayName，mac 分支）

    static func comboParts(_ binding: SettingsModel.ShortcutBinding) -> [String] {
        var parts: [String] = []
        for modifier in binding.modifiers {
            parts.append(modifierGlyph(modifier))
        }
        parts.append(primaryGlyph(binding.primary))
        return parts
    }

    /// modifiers 通用名（cmd/option/ctrl/shift）→ 符号；侧向名（cmd-left 等）兼容显示。
    static func modifierGlyph(_ name: String) -> String {
        switch name {
        case "cmd": return "⌘"
        case "cmd-left": return "左 ⌘"
        case "cmd-right": return "右 ⌘"
        case "ctrl": return "⌃"
        case "ctrl-left": return "左 ⌃"
        case "ctrl-right": return "右 ⌃"
        case "option": return "⌥"
        case "option-left": return "左 ⌥"
        case "option-right": return "右 ⌥"
        case "alt": return "⌥"
        case "shift": return "⇧"
        case "shift-left": return "左 ⇧"
        case "shift-right": return "右 ⇧"
        default: return name
        }
    }

    /// primary → 显示（mac）：单字母大写；命名键符号；侧向修饰键「左/右 ⌥」式。
    static func primaryGlyph(_ name: String) -> String {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        if trimmed.count == 1, trimmed.first!.isLetter {
            return trimmed.uppercased()
        }
        switch trimmed.lowercased() {
        case "space": return "␣"
        case "return", "enter": return "↩"
        case "tab": return "⇥"
        case "escape", "esc": return "⎋"
        case "backspace": return "⌫"
        case "delete", "del": return "⌦"
        case "arrowup", "up": return "↑"
        case "arrowdown", "down": return "↓"
        case "arrowleft", "left": return "←"
        case "arrowright", "right": return "→"
        case "rightoption": return "右 ⌥"
        case "leftoption": return "左 ⌥"
        case "rightcontrol": return "右 ⌃"
        case "leftcontrol": return "左 ⌃"
        case "rightcommand": return "右 ⌘"
        case "leftcommand": return "左 ⌘"
        case "leftshift": return "左 ⇧"
        case "rightshift": return "右 ⇧"
        case "shift": return "⇧"
        case "fn": return "Fn"
        default: return trimmed
        }
    }

    /// keyCode → core 键名（对齐 core parse_primary）。返回 nil = 纯修饰键/不支持。
    static func keyName(for keyCode: UInt16) -> String? {
        switch keyCode {
        case 0: return "A"
        case 1: return "S"
        case 2: return "D"
        case 3: return "F"
        case 4: return "H"
        case 5: return "G"
        case 6: return "Z"
        case 7: return "X"
        case 8: return "C"
        case 9: return "V"
        case 11: return "B"
        case 12: return "Q"
        case 13: return "W"
        case 14: return "E"
        case 15: return "R"
        case 16: return "Y"
        case 17: return "T"
        case 32: return "U"
        case 34: return "I"
        case 31: return "O"
        case 35: return "P"
        case 40: return "K"
        case 37: return "L"
        case 38: return "J"
        case 45: return "N"
        case 46: return "M"
        case 18: return "1"
        case 19: return "2"
        case 20: return "3"
        case 21: return "4"
        case 23: return "5"
        case 22: return "6"
        case 26: return "7"
        case 28: return "8"
        case 25: return "9"
        case 29: return "0"
        case 49: return "Space"
        case 36: return "Return"
        case 48: return "Tab"
        case 51: return "Backspace"
        case 117: return "Delete"
        case 115: return "Home"
        case 119: return "End"
        case 116: return "PageUp"
        case 121: return "PageDown"
        case 126: return "ArrowUp"
        case 125: return "ArrowDown"
        case 123: return "ArrowLeft"
        case 124: return "ArrowRight"
        case 122: return "F1"
        case 120: return "F2"
        case 99: return "F3"
        case 118: return "F4"
        case 96: return "F5"
        case 97: return "F6"
        case 98: return "F7"
        case 100: return "F8"
        case 101: return "F9"
        case 109: return "F10"
        case 103: return "F11"
        case 111: return "F12"
        default: return nil
        }
    }
}
