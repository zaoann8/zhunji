// 概览页数据模型（原版 Overview.tsx 的数据来源）：
// - 历史列表（今日指标 / 周历 / 最近识别）：zhunji_list_history
// - 年度活动热力图（独立存储，清空历史不影响）：zhunji_get_activity_stats
// - 引擎状态（上次会话结果 + 主动测试）：zhunji_get_engine_status / zhunji_test_engine
// - 引擎名：creds.activeAsrProvider → SettingsModel.providerOptions 匹配
// 事件刷新：history:changed（识别/删除/清空）→ 全量刷新；engine:test-result →
// 引擎状态；capsule:state（done/error）→ 引擎状态（原版监听同一事件）。

import Foundation

@MainActor
final class OverviewModel: ObservableObject {
    static let shared = OverviewModel()

    /// 与 core HISTORY_CAP 同步：上限为 null（不限制）时显示此值（原版 HISTORY_CAP_DISPLAY）。
    private let historyCapDisplay = 5000

    struct ActivityDay: Decodable {
        let date: String
        let count: Int
    }

    struct Credentials: Decodable {
        let activeAsrProvider: String
        let asrConfigured: Bool
    }

    struct EngineStatus {
        var ok: Bool
        var error: String?
        /// nil = 尚未加载 / 检测中。
        var loading: Bool = false
    }

    /// 历史条目（原版 DictationSession，camelCase；只取概览页用到的字段）。
    struct HistoryEntry: Decodable {
        let id: String
        let createdAt: String
        let finalText: String
        /// 识别原文——润色失败/未产出时复制回退（原版 finalText.trim ? finalText : rawTranscript）。
        let rawTranscript: String
        let durationMs: Int?
        let insertStatus: String
        let mode: String
        let asrProvider: String?
        let errorCode: String?
    }

    @Published var history: [HistoryEntry] = []
    @Published var historyError = false
    @Published var activity: [ActivityDay]? = nil
    @Published var activeAsrProvider = ""
    @Published var engineName = ""
    @Published var engineStatus: EngineStatus? = nil
    @Published var testing = false
    @Published var historyMaxEntries: Int? = nil

    private init() {}

    /// 首次加载 + 事件刷新入口（原版 refreshHistory / refreshActivity / refreshCredentials）。
    func refresh() {
        refreshHistory()
        refreshActivity()
        refreshCredentials()
        refreshEngineStatus()
    }

    func refreshHistory() {
        historyError = false
        guard let json = coreJsonString(zhunji_list_history),
              let data = json.data(using: .utf8),
              let arr = try? JSONDecoder().decode([HistoryEntry].self, from: data)
        else {
            log("历史读取失败")
            historyError = true
            return
        }
        history = arr
        log("历史：\(arr.count) 条")
    }

    func refreshActivity() {
        guard let json = coreJsonString(zhunji_get_activity_stats),
              let data = json.data(using: .utf8),
              let arr = try? JSONDecoder().decode([ActivityDay].self, from: data)
        else {
            activity = nil
            return
        }
        activity = arr
    }

    func refreshCredentials() {
        guard let json = coreJsonString(zhunji_get_credentials),
              let data = json.data(using: .utf8),
              let creds = try? JSONDecoder().decode(Credentials.self, from: data)
        else { return }
        activeAsrProvider = creds.activeAsrProvider
        // 引擎名：当前 provider 匹配失败回退默认 provider（原版 list_providers 查找逻辑）。
        let options = SettingsModel.shared.providerOptions
        engineName = options.first { $0.id == creds.activeAsrProvider }?.label
            ?? options.first { $0.id == "builtin-doubao" }?.label
            ?? ""
    }

    func refreshEngineStatus() {
        guard let json = coreJsonString(zhunji_get_engine_status),
              let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }
        engineStatus = EngineStatus(
            ok: obj["ok"] as? Bool ?? false,
            error: obj["error"] as? String
        )
    }

    /// 主动测试引擎（原版 runEngineTest：testing 期间按钮显示「检查中…」）。
    func runEngineTest() {
        guard !testing else { return }
        testing = true
        engineStatus = EngineStatus(ok: false, error: nil, loading: true)
        zhunji_test_engine()
    }

    /// engine:test-result 事件回调。
    func applyTestResult(_ payload: [String: Any]) {
        testing = false
        engineStatus = EngineStatus(
            ok: payload["ok"] as? Bool ?? false,
            error: payload["error"] as? String
        )
    }

    /// capsule:state 事件（done → 引擎正常；error → 引擎错误；原版监听同一事件）。
    func applyCapsuleState(_ payload: [String: Any]) {
        switch payload["state"] as? String {
        case "done":
            engineStatus = EngineStatus(ok: true, error: nil)
        case "error":
            engineStatus = EngineStatus(
                ok: false,
                error: payload["message"] as? String ?? "未知错误"
            )
        default:
            break
        }
    }

    // MARK: - 派生指标（原版 metrics useMemo）

    /// 今日：字符数 / 片段数 / 总时长 / 平均延迟（durationMs）。
    struct DailyMetrics {
        var chars: Int = 0
        var segments: Int = 0
        var totalDurationMs: Int = 0
        var avgLatencyMs: Int = 0
    }

    var dailyMetrics: DailyMetrics {
        let calendar = Calendar.current
        let todayStart = calendar.startOfDay(for: Date())
        var m = DailyMetrics()
        for s in history {
            guard let created = parseISO8601(s.createdAt),
                  created >= todayStart
            else { continue }
            m.chars += s.finalText.count
            m.segments += 1
            m.totalDurationMs += s.durationMs ?? 0
        }
        if m.segments > 0 {
            m.avgLatencyMs = m.totalDurationMs / m.segments
        }
        return m
    }

    /// 周历：过去 7 天每天的条数（原版 weekly useMemo，buckets[6-diff]）。
    var weeklyBuckets: [Int] {
        let calendar = Calendar.current
        let todayStart = calendar.startOfDay(for: Date())
        var buckets = Array(repeating: 0, count: 7)
        for s in history {
            guard let d = parseISO8601(s.createdAt) else { continue }
            let dayStart = calendar.startOfDay(for: d)
            let diff = calendar.dateComponents([.day], from: dayStart, to: todayStart).day ?? 0
            if diff >= 0 && diff < 7 {
                buckets[6 - diff] += 1
            }
        }
        return buckets
    }

    /// 历史条数上限显示值（prefs.historyMaxEntries ?? 5000）。
    func historyCapDisplayValue() -> Int {
        if let historyMaxEntries { return historyMaxEntries }
        // 懒读一次 prefs（原版 prefs.historyMaxEntries ?? HISTORY_CAP_DISPLAY）。
        if let json = coreJsonString(zhunji_get_prefs),
           let data = json.data(using: .utf8),
           let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let cap = obj["historyMaxEntries"] as? Int {
            historyMaxEntries = cap
            return cap
        }
        return historyCapDisplay
    }

    /// ISO-8601 → Date（原版 new Date(s.createdAt)）。
    func parseISO8601(_ text: String) -> Date? {
        let trimmed = text.hasSuffix("Z") ? text : text + "Z"
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = formatter.date(from: trimmed) { return date }
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.date(from: trimmed)
    }

    private func log(_ message: String) {
        NSLog("[OverviewModel] %@", message)
    }
}
