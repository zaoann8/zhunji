// 设置页数据模型 — 与 core 的 preferences.json（camelCase 全量 JSON）双向同步。
//
// 加载：zhunji_get_prefs / zhunji_list_microphone_devices
// 保存：zhunji_set_prefs（基于上次读到的全量 JSON 合并改动字段再发回——
//      serde default 会兜底缺失字段，但全量回传保证 40+ 字段无一丢失）；
//      热键字段变化 core 侧自动重注册（ffi.rs 的 diff 逻辑）。
// 事件：device:changed → 刷新麦克风列表；prefs:changed → 重载（其他入口改配置时同步）。
//
// P1.4 只管理 P1 范围字段；其余字段原样保留在 basePrefs 里不动。

import Foundation
import AppKit

@MainActor
final class SettingsModel: ObservableObject {
    static let shared = SettingsModel()

    // MARK: - P1.4 字段

    /// 听写热键（primary = 键名，modifiers = ["cmd","option","ctrl","shift"]）。
    struct ShortcutBinding: Codable, Equatable {
        var primary: String
        var modifiers: [String]
    }

    /// 麦克风设备（isDefault 由 core 标记）。
    struct MicDevice: Codable, Identifiable {
        let name: String
        let isDefault: Bool
        var id: String { name }
    }

    /// ASR 引擎（原版 list_providers → 注册表实时数据，内置豆包常驻）。
    struct AsrOption: Identifiable {
        let id: String
        let label: String
        var isBuiltin: Bool { id == "builtin-doubao" }
    }

    /// 引擎下拉选项（core 注册表；加载失败回退内置豆包）。
    @Published var providerOptions: [AsrOption] = [
        AsrOption(id: "builtin-doubao", label: "豆包 IME"),
    ]

    // 默认 = 原版 hotkey.ts defaultDictationShortcut：RightControl（右 Control）。
    @Published var dictationHotkey = ShortcutBinding(primary: "RightControl", modifiers: [])
    @Published var openAppHotkey: ShortcutBinding? = nil
    @Published var hotkeyMode = "hold" // toggle / hold / auto（prefs.hotkey.mode）
    @Published var activeAsrProvider = "builtin-doubao"
    @Published var microphoneDeviceName = "" // "" = 系统默认
    @Published var microphones: [MicDevice] = []
    @Published var audioCueOnRecord = true
    @Published var showCapsule = true
    @Published var capsuleStyle = "siri" // siri / classic
    @Published var muteDuringRecording = false
    @Published var streamingInsert = true
    @Published var restoreClipboardAfterPaste = false
    @Published var startMinimized = false
    @Published var themeMode = "system" // system / light / dark（原版 ThemeMode camelCase）
    @Published var showOverviewActivityHeatmap = true

    /// 高级页：保留原始录音（调试）+ 保留条数（nil = 不限制，core 走 200 默认）。
    @Published var recordAudioForDebug = false
    @Published var audioRecordingMaxEntries: Int? = nil

    /// 麦克风实时电平 0..1（下拉打开时经 microphone:level 事件更新）。
    @Published var micLevel: Double = 0

    /// 网络连通性（network:result 事件更新）。
    @Published var networkOnline: Bool?
    @Published var networkLatencyMs: Int?

    /// 热键监听状态（zhunji_get_hotkey_status）。
    @Published var hotkeyState = "starting" // installed / starting / failed
    @Published var hotkeyMessage = ""

    /// 加载状态（设置页首屏骨架展示用）。
    @Published var isLoading = true

    /// core 上次返回的全量 prefs JSON（改动字段时合并回传）。
    private var basePrefs: [String: Any] = [:]

    private init() {}

    // MARK: - 加载

    /// 全量加载：prefs + 供应商注册表 + 麦克风列表。
    func load() {
        loadPrefs()
        loadProviders()
        loadMicrophones()
        isLoading = false
    }

    /// 引擎下拉选项（原版 list_providers；内置豆包不在列表时 core 侧兜底）。
    func loadProviders() {
        guard let json = coreJsonString(zhunji_list_providers),
              let data = json.data(using: .utf8),
              let arr = try? JSONDecoder().decode([ProviderEntry].self, from: data)
        else {
            log("供应商列表读取失败")
            return
        }
        providerOptions = arr.map { AsrOption(id: $0.id, label: $0.name) }
        // 原版：修复旧 activeAsrProvider 与新供应商 id 不匹配（如默认供应商被删），
        // 回落到 providers 里的 default（原版 RecordingInputSection 90-98 行）。
        if !arr.contains(where: { $0.id == activeAsrProvider }),
           let def = arr.first(where: { $0.isDefault == true }) {
            activeAsrProvider = def.id
            save(["activeAsrProvider": def.id])
        }
        log("供应商注册表：\(arr.count) 个")
    }

    /// zhunji_list_providers 的条目（camelCase）。
    private struct ProviderEntry: Decodable {
        let id: String
        let name: String
        var isDefault: Bool?

        enum CodingKeys: String, CodingKey {
            case id, name
            case isDefault = "default"
        }
    }

    private func loadPrefs() {
        guard let json = coreJsonString(zhunji_get_prefs),
              let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            log("读取偏好失败（core 未初始化？）")
            return
        }
        basePrefs = obj
        if let hk = obj["dictationHotkey"] as? [String: Any],
           let primary = hk["primary"] as? String {
            dictationHotkey = ShortcutBinding(
                primary: primary,
                modifiers: hk["modifiers"] as? [String] ?? []
            )
        }
        if let hk = obj["openAppHotkey"] as? [String: Any],
           let primary = hk["primary"] as? String {
            openAppHotkey = ShortcutBinding(
                primary: primary,
                modifiers: hk["modifiers"] as? [String] ?? []
            )
        } else {
            openAppHotkey = nil
        }
        if let hotkey = obj["hotkey"] as? [String: Any],
           let mode = hotkey["mode"] as? String {
            hotkeyMode = mode
        }
        activeAsrProvider = obj["activeAsrProvider"] as? String ?? "builtin-doubao"
        microphoneDeviceName = obj["microphoneDeviceName"] as? String ?? ""
        audioCueOnRecord = obj["audioCueOnRecord"] as? Bool ?? true
        showCapsule = obj["showCapsule"] as? Bool ?? true
        capsuleStyle = obj["capsuleStyle"] as? String ?? "siri"
        muteDuringRecording = obj["muteDuringRecording"] as? Bool ?? false
        streamingInsert = obj["streamingInsert"] as? Bool ?? true
        restoreClipboardAfterPaste = obj["restoreClipboardAfterPaste"] as? Bool ?? false
        startMinimized = obj["startMinimized"] as? Bool ?? false
        themeMode = obj["themeMode"] as? String ?? "system"
        showOverviewActivityHeatmap = obj["showOverviewActivityHeatmap"] as? Bool ?? true
        recordAudioForDebug = obj["recordAudioForDebug"] as? Bool ?? false
        audioRecordingMaxEntries = obj["audioRecordingMaxEntries"] as? Int
        log("偏好已加载：引擎=\(activeAsrProvider) 麦克风=\(microphoneDeviceName) 样式=\(capsuleStyle)")
    }

    func loadMicrophones() {
        guard let json = coreJsonString(zhunji_list_microphone_devices),
              let data = json.data(using: .utf8),
              let arr = try? JSONDecoder().decode([MicDevice].self, from: data)
        else {
            log("麦克风列表读取失败")
            return
        }
        microphones = arr
        log("麦克风设备：\(arr.count) 个")
    }

    // MARK: - 保存

    /// 合并改动字段到全量 prefs 并回写 core。返回 0 成功。
    @discardableResult
    func save(_ updates: [String: Any]) -> Int32 {
        for (key, value) in updates {
            basePrefs[key] = value
        }
        guard var data = try? JSONSerialization.data(withJSONObject: basePrefs) else {
            log("偏好序列化失败")
            return 4
        }
        // Rust 侧 read_c_string 按 C 字符串读（需 \0 结尾）；JSONSerialization 输出
        // 没有结尾 0 字节，不补会越界读到内存垃圾 → serde 报 "trailing characters"
        // 解析失败（曾致热键保存概率性失败，用户录的快捷键落不了盘）。
        data.append(0)
        let result = data.withUnsafeBytes { raw -> Int32 in
            guard let base = raw.baseAddress else { return 4 }
            return zhunji_set_prefs(base.assumingMemoryBound(to: CChar.self))
        }
        log("保存偏好：\(updates.keys) → \(result)")
        return result
    }

    /// 保存录音方式（hotkey.mode）——hotkey 是嵌套 dict，需在原值上合并。
    func saveHotkeyMode(_ mode: String) {
        var hotkey = basePrefs["hotkey"] as? [String: Any] ?? [:]
        hotkey["mode"] = mode
        save(["hotkey": hotkey])
        hotkeyMode = mode
    }

    // MARK: - 权限页状态（热键 / 网络）

    /// 拉取热键监听状态（installed / starting / failed）。
    func loadHotkeyStatus() {
        guard let json = coreJsonString(zhunji_get_hotkey_status),
              let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }
        hotkeyState = obj["state"] as? String ?? "starting"
        hotkeyMessage = obj["message"] as? String ?? ""
    }

    /// 发起网络连通性检查（结果异步经 network:result 事件回调）。
    func checkNetwork() {
        networkOnline = nil
        zhunji_check_network()
    }

    /// 应用 network:result 事件 payload。
    func applyNetworkResult(_ payload: [String: Any]) {
        if let online = payload["online"] as? Bool {
            networkOnline = online
        }
        networkLatencyMs = payload["latencyMs"] as? Int
    }

    // MARK: - 麦克风电平（设置页麦克风下拉）

    /// 开始监听电平（空名 = 系统默认）。内部同步构造音频流（数十 ms），放后台队列。
    func startLevelMonitor() {
        micLevel = 0
        let device = microphoneDeviceName
        DispatchQueue.global(qos: .userInitiated).async {
            _ = device.withCString { zhunji_start_microphone_level_monitor($0) }
        }
    }

    /// 停止电平监听。
    func stopLevelMonitor() {
        micLevel = 0
        DispatchQueue.global(qos: .userInitiated).async {
            zhunji_stop_microphone_level_monitor()
        }
    }

    private func log(_ message: String) {
        NSLog("[SettingsModel] %@", message)
    }
}
