// 历史页数据模型（原版 History.tsx）：
// - 列表：zhunji_list_history（完整 DictationSession 字段，详情页流水线明细用）
// - 删除 / 清空：zhunji_delete_history_entry / zhunji_clear_history（core 发
//   history:changed → 本模型与 OverviewModel 一起全量刷新，与原版双页同模式）
// - 重转录：zhunji_retranscribe_recording 发起，`history:retranscribed` 事件
//   回带更新后的整条记录 → 局部替换（原版 setItems(map)）
// - 录音：readAudio（data URL → Data，AVAudioPlayer 播放）/ export（NSSavePanel
//   选路径后 zhunji_export_audio_recording 拷贝）。wav 被 retention/cap 清理后
//   首次 IPC 报 "recording not found" 的 id 记入 audioMissingIds，按钮永久隐藏。

import AppKit
import AVFoundation
import Foundation

@MainActor
final class HistoryModel: ObservableObject {
    static let shared = HistoryModel()

    /// 完整历史条目（原版 DictationSession，camelCase，字段与 core serde 输出一致）。
    struct Session: Decodable {
        let id: String
        let createdAt: String
        let rawTranscript: String
        let finalText: String
        let mode: String
        let insertStatus: String
        let errorCode: String?
        let durationMs: Int?
        let appName: String?
        let asrProvider: String?
        let asrModel: String?
        let asrMs: Int?
        let llmProvider: String?
        let llmModel: String?
        let polishMs: Int?
        let dictionaryEntryCount: Int?
        let hasAudioRecording: Bool?
    }

    @Published var items: [Session] = []
    @Published var loading = false
    @Published var loadError: String?
    @Published var actionError: String?
    @Published var selectedId: String?
    @Published var retranscribing = false
    /// lazily-detected missing：wav 被清理但条目 hasAudioRecording 仍 true 的 id。
    @Published var audioMissingIds: Set<String> = []

    private init() {}

    /// 原版 refresh（listHistory + selectedId 保持/回退 data[0]）。
    func refresh() {
        loading = true
        loadError = nil
        guard let json = coreJsonString(zhunji_list_history),
              let data = json.data(using: .utf8),
              let arr = try? JSONDecoder().decode([Session].self, from: data)
        else {
            loadError = "加载历史失败"
            loading = false
            return
        }
        items = arr
        actionError = nil
        if let selectedId, !arr.contains(where: { $0.id == selectedId }) {
            self.selectedId = arr.first?.id
        } else if selectedId == nil {
            self.selectedId = arr.first?.id
        }
        loading = false
    }

    /// 原版 onDelete：删除成功后 core 发 history:changed → 事件驱动全量刷新。
    func delete(_ id: String) {
        actionError = nil
        let result = id.withCString { zhunji_delete_history_entry($0) }
        if result != 0 {
            actionError = "删除失败：\(result)"
        }
    }

    /// 原版 onClear：confirm 由 View 弹 NSAlert；清空后事件驱动刷新。
    func clear() {
        actionError = nil
        let result = zhunji_clear_history()
        if result != 0 {
            actionError = "清空失败：\(result)"
        }
    }

    /// 原版 onRetranscribe：禁用按钮 + 事件回带整条记录局部替换。
    func retranscribe(_ id: String) {
        guard !retranscribing else { return }
        retranscribing = true
        actionError = nil
        let result = id.withCString { zhunji_retranscribe_recording($0) }
        if result != 0 {
            retranscribing = false
            actionError = "重新转录失败：\(result)"
        }
    }

    /// history:retranscribed 事件（原版 retranscribeRecording 返回的整条记录）。
    func applyRetranscribed(_ payload: [String: Any]) {
        retranscribing = false
        if let err = payload["error"] as? String {
            actionError = "重新转录失败：\(err)"
            return
        }
        guard let entryData = payload["entry"] else { return }
        // entry 是字典 → 重新 JSON 化后按 Session 解码（避免手写字段映射）。
        guard let data = try? JSONSerialization.data(withJSONObject: entryData),
              let entry = try? JSONDecoder().decode(Session.self, from: data)
        else { return }
        items = items.map { $0.id == entry.id ? entry : $0 }
    }

    /// 录音文件缺失 → 记入集合（原版 markAudioMissing）。
    func markAudioMissing(_ id: String) {
        guard !audioMissingIds.contains(id) else { return }
        audioMissingIds.insert(id)
    }

    // MARK: - 录音读取 / 导出

    /// 读取录音 wav 字节（data URL 解码）。找不到返回 nil（调用方 markAudioMissing）。
    func readAudioData(_ id: String) -> Data? {
        guard let json = coreJsonString({
            id.withCString { zhunji_read_audio_recording($0) }
        }),
        let data = json.data(using: .utf8),
        let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        if let err = obj["error"] as? String {
            if err.contains("not found") { markAudioMissing(id) }
            return nil
        }
        guard let dataUrl = obj["data"] as? String,
              let comma = dataUrl.firstIndex(of: ",")
        else { return nil }
        return Data(base64Encoded: String(dataUrl[dataUrl.index(after: comma)...]))
    }

    /// 导出录音（原版 export_audio_recording：NSSavePanel 在 Swift 侧，core 拷贝）。
    /// 返回 true 成功；false + error 输出错误原因（"not found" 时按钮隐藏）。
    @discardableResult
    func exportAudio(_ id: String, error: inout String?) -> Bool {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.wav]
        panel.nameFieldStringValue = "zhunji-recording-\(id).wav"
        panel.canCreateDirectories = true
        guard panel.runModal() == .OK, let url = panel.url else { return false }
        error = nil
        let result = id.withCString { sid in
            url.path.withCString { dest in
                zhunji_export_audio_recording(sid, dest)
            }
        }
        switch result {
        case 0: return true
        case 3:
            markAudioMissing(id)
            error = "recording not found"
            return false
        default:
            error = "导出录音失败，请重试。"
            return false
        }
    }

    // MARK: - 格式化（原版 History.tsx 底部函数）

    /// 原版 formatTime：今天 HH:mm，否则 M/d HH:mm。
    static func formatTime(_ iso: String) -> String {
        guard let d = parseISO8601(iso) else { return iso }
        let cal = Calendar.current
        let now = Date()
        let fmt = DateFormatter()
        if cal.isDate(d, inSameDayAs: now) {
            fmt.dateFormat = "HH:mm"
        } else {
            fmt.dateFormat = "M/d HH:mm"
        }
        return fmt.string(from: d)
    }

    /// 原版 formatDuration：<=0 或 nil → "—"；<60s → "%.1f 秒"；否则 "M:SS 分钟"。
    /// 原版 i18n：durationSeconds "{{value}} 秒" / durationMinutes "{{value}} 分钟"，
    /// 分钟用 toFixed(1)（如 "1.2 分钟"）。
    static func formatDuration(_ ms: Int?) -> String {
        guard let ms, ms > 0 else { return "—" }
        let sec = Double(ms) / 1000.0
        if sec < 60 { return String(format: "%.1f 秒", sec) }
        return String(format: "%.1f 分钟", sec / 60.0)
    }

    /// 原版 formatStepDuration：<1s 显示整数毫秒（流式收尾几十 ms，0.1s 精度
    /// 会把不同结果拍成同一个值）；≥1s 沿用 0.1s 精度。
    static func formatStepDuration(_ ms: Int) -> String {
        if ms < 1000 { return "\(ms) 毫秒" }
        return formatDuration(ms)
    }

    /// 原版 MODE_LABEL（zh-CN style.modes.*.name）。
    static func modeLabel(_ mode: String) -> String {
        switch mode {
        case "raw": "原文"
        case "light": "轻度润色"
        case "structured": "清晰结构"
        case "formal": "正式表达"
        default: mode
        }
    }

    /// 原版 groupByDay：今天/昨天/YYYY年M月D日 星期X，组内保持 newest-first。
    static func groupByDay(_ items: [Session]) -> [(label: String, items: [Session])] {
        let cal = Calendar.current
        let todayStart = cal.startOfDay(for: Date())
        let yesterdayStart = cal.date(byAdding: .day, value: -1, to: todayStart)!
        var groups: [(String, [Session])] = []
        for item in items {
            guard let d = parseISO8601(item.createdAt) else { continue }
            let dayStart = cal.startOfDay(for: d)
            let label: String
            if dayStart == todayStart {
                label = "今天"
            } else if dayStart == yesterdayStart {
                label = "昨天"
            } else {
                let fmt = DateFormatter()
                fmt.locale = Locale(identifier: "zh_CN")
                fmt.dateFormat = "yyyy年M月d日 EEEE"
                label = fmt.string(from: d)
            }
            if var last = groups.last, last.0 == label {
                last.1.append(item)
                groups[groups.count - 1] = last
            } else {
                groups.append((label, [item]))
            }
        }
        return groups
    }

    /// ISO-8601 → Date（原版 new Date(s.createdAt)）。
    static func parseISO8601(_ text: String) -> Date? {
        let trimmed = text.hasSuffix("Z") ? text : text + "Z"
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = formatter.date(from: trimmed) { return date }
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.date(from: trimmed)
    }
}

// MARK: - 录音播放（原版 AudioRecordingPlayer：加载按钮 → 原生 audio controls）

/// 播放器状态机（idle → loading → ready/error）。ready 后 AVAudioPlayer 播放，
/// 支持播放/暂停 + 进度；加载失败报错（"not found" → 父组件隐藏按钮）。
@MainActor
final class RecordingPlayer: NSObject, ObservableObject, AVAudioPlayerDelegate {
    @Published var status: Status = .idle
    @Published var isPlaying = false
    @Published var currentTime: TimeInterval = 0

    enum Status {
        case idle
        case loading
        case ready
        case error(String)
    }

    private var player: AVAudioPlayer?
    private var timer: Timer?

    func load(sessionId: String, model: HistoryModel) {
        status = .loading
        guard let data = model.readAudioData(sessionId) else {
            status = .idle
            return
        }
        guard let player = try? AVAudioPlayer(data: data) else {
            status = .error("音频解码失败")
            return
        }
        player.delegate = self
        self.player = player
        status = .ready
        play()
    }

    func togglePlayPause() {
        guard let player else { return }
        if player.isPlaying {
            player.pause()
            isPlaying = false
            timer?.invalidate()
        } else {
            player.play()
            isPlaying = true
            startTimer()
        }
    }

    private func play() {
        guard let player else { return }
        player.play()
        isPlaying = true
        startTimer()
    }

    private func startTimer() {
        timer?.invalidate()
        timer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            guard let self, let player = self.player else { return }
            self.currentTime = player.currentTime
        }
    }

    func stop() {
        timer?.invalidate()
        timer = nil
        player?.stop()
        player = nil
        isPlaying = false
    }

    func audioPlayerDidFinishPlaying(_ player: AVAudioPlayer, successfully flag: Bool) {
        isPlaying = false
        timer?.invalidate()
        currentTime = 0
    }

    var duration: TimeInterval { player?.duration ?? 0 }

    func seek(to time: TimeInterval) {
        guard let player else { return }
        player.currentTime = time
        currentTime = time
    }
}
