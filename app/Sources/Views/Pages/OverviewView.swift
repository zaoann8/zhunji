// 概览页 — 1:1 对照原版 Overview.tsx（838 行）+ Heatmap.tsx（222 行）。
// 结构：PageHeader（仅 title 26/600）→ 引擎状态卡 → 4 Metric → 年度活动热力图卡
// （可关，prefs.showOverviewActivityHeatmap）→ 底部 1fr 1.4fr：周历 + 最近识别 5 条。
// 数据源：OverviewModel（list_history / get_activity_stats / get_credentials /
// get_engine_status / test_engine）；事件刷新：history:changed / capsule:state。

import AppKit
import SwiftUI

struct OverviewView: View {
    @ObservedObject private var model = OverviewModel.shared
    @ObservedObject private var settings = SettingsModel.shared
    var onOpenHistory: (() -> Void)? = nil

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // 原版 PageHeader：h1 26/600/-0.02em，仅 title。
            Text("今日概览")
                .font(.system(size: 26, weight: .semibold))
                .kerning(-0.5)
                .foregroundStyle(Color.zhInk)
                .padding(.bottom, 24)

            engineCard
                .padding(.bottom, 12)

            metricsRow
                .padding(.bottom, 18)

            // 热力图独立于历史内容存储（清空历史不影响）；设置 → 通用 → 外观 可关。
            if settings.showOverviewActivityHeatmap,
               let activity = model.activity, !activity.isEmpty {
                heatmapCard(activity)
                    .padding(.bottom, 18)
            }

            // 底部一行 flex:1 撑满剩余高度（原版 issue #243 布局）。
            bottomRow
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .padding(.horizontal, 28)
        .padding(.top, 24)
        .padding(.bottom, 32)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .onAppear { model.refresh() }
    }

    // MARK: - 引擎状态卡（原版 Overview.tsx 247-301 行）

    private var engineCard: some View {
        let failed = model.engineStatus?.ok == false
        return OlCard(padding: 14) {
            HStack(spacing: 10) {
                // 32×32 r9 图标块：错误 → err-soft(#fff0f0)+#dc2626；否则 blue-soft+blue。
                Image(systemName: "mic")
                    .font(.system(size: 15, weight: .medium))
                    .foregroundStyle(failed ? errColor : Color.zhBlue)
                    .frame(width: 32, height: 32)
                    .background(
                        RoundedRectangle(cornerRadius: 9)
                            .fill(failed ? errSoftColor : Color.zhBlueSoft)
                    )
                VStack(alignment: .leading, spacing: 2) {
                    Text(model.engineName.isEmpty ? "加载中…" : model.engineName)
                        .font(.system(size: 12.5, weight: .semibold))
                        .foregroundStyle(Color.zhInk)
                        .lineLimit(1)
                    Text(statusText)
                        .font(.system(size: 11.5))
                        .foregroundStyle(failed ? errColor : Color.zhOK)
                        .lineLimit(2)
                        .truncationMode(.middle)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                OlGhostBtn(model.testing ? "检测中…" : "检测") {
                    model.runEngineTest()
                }
            }
        }
    }

    /// 原版：null → engineChecking；ok → engineOk；error → engineFail：err。
    private var statusText: String {
        switch model.engineStatus {
        case nil: return "检测中…"
        case .some(let st) where st.ok: return "✓ 引擎可用"
        case .some(let st): return "✗ 引擎不可用：\(st.error ?? "未知错误")"
        }
    }

    /// 原版硬编码错误色（--ol-err 浅色段 #dc2626 / err-soft fallback #fff0f0）。
    private let errColor = Color(nsColor: NSColor(hex: 0xDC2626))
    private let errSoftColor = Color(nsColor: NSColor(hex: 0xFFF0F0))

    // MARK: - 4 Metric（原版 303-352 行）

    private var metricsRow: some View {
        let m = model.dailyMetrics
        return HStack(spacing: 12) {
            MetricView(
                icon: "number",
                label: "今日字数",
                value: model.historyError ? "—" : Self.groupFormatter.string(from: NSNumber(value: m.chars)) ?? "\(m.chars)",
                trend: model.historyError
                    ? "历史读取失败"
                    : "\(m.segments) 段"
            )
            MetricView(
                icon: "mic",
                label: "今日总时长",
                value: model.historyError ? "—" : Self.formatDurationFull(m.totalDurationMs),
                trend: model.historyError ? "历史读取失败" : ""
            )
            MetricView(
                icon: "clock",
                label: "平均段落",
                value: model.historyError ? "—" : Self.formatDuration(m.avgLatencyMs),
                trend: model.historyError
                    ? "历史读取失败"
                    : (m.segments > 0 ? "今日均值" : "暂无数据")
            )
            MetricView(
                icon: "bolt",
                label: "累计记录",
                value: model.historyError
                    ? "—"
                    : "\(model.history.count) / \(model.historyCapDisplayValue())",
                trend: model.historyError ? "历史读取失败" : "",
                accent: true
            )
        }
    }

    /// 原版 Metric：icon 13 + label 11.5（ink-3）→ value 26/600 accent 可蓝 → trend 11（ink-4）。
    private struct MetricView: View {
        let icon: String
        let label: String
        let value: String
        let trend: String
        var accent = false

        var body: some View {
            OlCard(padding: 16) {
                HStack(spacing: 6) {
                    Image(systemName: icon)
                        .font(.system(size: 13))
                    Text(label)
                        .font(.system(size: 11.5))
                }
                .foregroundStyle(Color.zhInk3)
                .padding(.bottom, 8)

                Text(value)
                    .font(.system(size: 26, weight: .semibold))
                    .kerning(-0.5)
                    .foregroundStyle(accent ? Color.zhBlue : Color.zhInk)
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)

                // 原版 trend || " "：空字符串也占一行保持高度一致。
                Text(trend.isEmpty ? " " : trend)
                    .font(.system(size: 11))
                    .foregroundStyle(Color.zhInk4)
                    .padding(.top, 6)
            }
            .frame(maxWidth: .infinity)
        }
    }

    // MARK: - 年度活动热力图卡（原版 ActivityHeatmapCard，526-602 行）

    private func heatmapCard(_ activity: [OverviewModel.ActivityDay]) -> some View {
        let today = Date()
        let start = Calendar.current.date(byAdding: .day, value: -364, to: today)!
        let todayIso = Self.isoString(today)
        let todayCount = activity.first { $0.date == todayIso }?.count ?? 0
        let totalCount = activity.reduce(0) { $0 + $1.count }
        return OlCard(padding: 18) {
            VStack(spacing: 12) {
                HStack {
                    Text("年度活动")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Color.zhInk2)
                    Spacer()
                    Text("\(todayCount) 次听写 · 全年 \(totalCount) 次")
                        .font(.system(size: 11))
                        .foregroundStyle(Color.zhInk4)
                        .fixedSize()
                }
                ZhHeatmap(
                    data: Dictionary(uniqueKeysWithValues: activity.map { ($0.date, $0.count) }),
                    startDate: start,
                    endDate: today
                )
            }
        }
    }

    // MARK: - 底部：周历 + 最近识别（原版 366-520 行，grid 1fr 1.4fr）

    private var bottomRow: some View {
        GeometryReader { geo in
            HStack(alignment: .top, spacing: 12) {
                weekCard
                    .frame(width: (geo.size.width - 12) * (1.0 / 2.4))
                recentCard
                    .frame(width: (geo.size.width - 12) * (1.4 / 2.4))
            }
        }
    }

    /// 周历卡：header + 7 天柱状图（100 高）+ 星期标签（今天起倒排）。
    private var weekCard: some View {
        OlCard(padding: 18) {
            VStack(spacing: 0) {
                HStack {
                    Text("近 7 天")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Color.zhInk2)
                    Spacer()
                    Text("条数 / 天")
                        .font(.system(size: 11))
                        .foregroundStyle(Color.zhInk4)
                }
                .padding(.bottom, 14)

                if model.historyError {
                    Text("历史读取失败")
                        .font(.system(size: 12))
                        .foregroundStyle(Color.zhInk4)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    WeekChartView(data: model.weeklyBuckets)
                }

                HStack {
                    ForEach(Array(Self.weekDayLabels(["日", "一", "二", "三", "四", "五", "六"]).enumerated()),
                            id: \.offset) { _, label in
                        Text(label)
                            .font(.system(size: 10))
                            .foregroundStyle(Color.zhInk4)
                            .frame(maxWidth: .infinity)
                    }
                }
                .padding(.top, 8)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
        .clipShape(RoundedRectangle(cornerRadius: 14))
    }

    /// 最近识别卡：header（borderBottom line）+ 内部滚动列表。
    private var recentCard: some View {
        OlCard(padding: 0) {
            VStack(spacing: 0) {
                HStack {
                    Text("最近识别")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Color.zhInk2)
                    Spacer()
                    OlGhostBtn("全部记录 →") {
                        onOpenHistory?()
                    }
                }
                .padding(.horizontal, 18)
                .padding(.vertical, 14)
                .overlay(alignment: .bottom) {
                    Rectangle().fill(Color.zhLine).frame(height: 0.5)
                }

                ScrollView {
                    VStack(spacing: 0) {
                        if model.historyError {
                            VStack(spacing: 10) {
                                Text("无法读取最近识别，请重试。")
                                    .font(.system(size: 12))
                                    .foregroundStyle(Color.zhInk4)
                                OlGhostBtn("重试") { model.refreshHistory() }
                            }
                            .padding(24)
                            .frame(maxWidth: .infinity)
                        } else {
                            if model.history.isEmpty {
                                Text("还没有记录。按 \(hotkeyLabel) 开始第一次录音。")
                                    .font(.system(size: 12))
                                    .foregroundStyle(Color.zhInk4)
                                    .multilineTextAlignment(.center)
                                    .padding(24)
                                    .frame(maxWidth: .infinity)
                            }
                            ForEach(model.history.prefix(5), id: \.id) { session in
                                RecentRowView(session: session)
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .top)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
        .clipShape(RoundedRectangle(cornerRadius: 14))
    }

    /// 原版 formatComboLabel(prefs.dictationHotkey)：comboParts 无分隔拼接。
    private var hotkeyLabel: String {
        ShortcutRecorderView.comboParts(settings.dictationHotkey).joined()
    }

    // MARK: - 格式化（原版 formatTime / formatDurationFull / formatDuration）

    /// 千分位（原版 toLocaleString()）。
    private static let groupFormatter: NumberFormatter = {
        let f = NumberFormatter()
        f.numberStyle = .decimal
        f.groupingSeparator = ","
        f.locale = Locale(identifier: "en_US_POSIX")
        return f
    }()

    /// 列表用时长：秒 → "X.X秒"；分钟 → "M:SS"；<=0 → "—"。
    static func formatDuration(_ ms: Int) -> String {
        if ms <= 0 { return "—" }
        let sec = Double(ms) / 1000
        if sec < 60 { return String(format: "%.1f秒", sec) }
        return "\(Int(floor(sec / 60))):\(String(format: "%02d", Int(sec) % 60))"
    }

    /// 累计时长：X时X分X秒 / X分X秒 / X秒（今日总时长用）；<=0 → "—"。
    static func formatDurationFull(_ ms: Int) -> String {
        if ms <= 0 { return "—" }
        let totalSec = Int((Double(ms) / 1000).rounded())
        let h = totalSec / 3600
        let m = (totalSec % 3600) / 60
        let s = totalSec % 60
        if h > 0 { return "\(h)时\(m)分\(s)秒" }
        if m > 0 { return "\(m)分\(s)秒" }
        return "\(s)秒"
    }

    /// 原版 weekDayLabels：左→右 = 6 天前 → 今天（names 按 周日~周六 排列）。
    static func weekDayLabels(_ names: [String]) -> [String] {
        let today = (Calendar.current.component(.weekday, from: Date()) - 1) % 7 // 0=周日
        return (0..<7).map { out in
            let i = 6 - out
            return names[(today - i + 7) % 7]
        }
    }

    /// yyyy-MM-dd（本地时区，原版 isoOf）。
    static func isoString(_ date: Date) -> String {
        let f = DateFormatter()
        f.dateFormat = "yyyy-MM-dd"
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = .current
        return f.string(from: date)
    }
}

// MARK: - 周历柱状图（原版 WeekChart，645-690 行）

private struct WeekChartView: View {
    let data: [Int]

    var body: some View {
        let maxValue = data.max() ?? 1
        HStack(alignment: .bottom, spacing: 8) {
            ForEach(Array(data.enumerated()), id: \.offset) { i, v in
                let isToday = i == 6
                VStack(spacing: 4) {
                    Text("\(v)")
                        .font(.system(size: 9.5, weight: isToday ? .semibold : .regular))
                        .foregroundStyle(isToday ? Color.zhBlue : Color.zhInk4)
                    RoundedRectangle(cornerRadius: 4)
                        .fill(isToday ? Color.zhBlue : Color.zhInk4)
                        .frame(height: Swift.max(CGFloat(v) * 80 / CGFloat(maxValue), 2))
                        .opacity(v == 0 ? 0.15 : (isToday ? 1 : 0.85))
                }
                .frame(maxWidth: .infinity)
            }
        }
        .frame(height: 100, alignment: .bottom)
    }
}

// MARK: - 最近识别行（原版 RecentRow，692-797 行）

private struct RecentRowView: View {
    let session: OverviewModel.HistoryEntry

    @State private var copied = false

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            // 左列：时间（mono）+ ASR 引擎蓝色小徽标
            VStack(alignment: .leading, spacing: 4) {
                Text(formatTime(session.createdAt))
                    .font(.system(size: 11))
                    .monospacedDigit()
                    .foregroundStyle(Color.zhInk3)
                if let provider = session.asrProvider, !provider.isEmpty {
                    OlPill(tone: .blue, size: .sm) {
                        Text(provider)
                    }
                }
            }
            .frame(minWidth: 60, alignment: .leading)

            // 中列：finalText 首行，2 行截断（原版 WebkitLineClamp 2）
            Text(session.finalText.split(separator: "\n").first.map(String.init) ?? "")
                .font(.system(size: 12.5))
                .foregroundStyle(Color.zhInk2)
                .lineSpacing(6.9)
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)

            // 右列：时长（mono）+ 复制按钮
            VStack(alignment: .trailing, spacing: 6) {
                Text(OverviewView.formatDuration(session.durationMs ?? 0))
                    .font(.system(size: 10.5))
                    .monospacedDigit()
                    .foregroundStyle(Color.zhInk4)
                Button {
                    copyEntry()
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: copied ? "checkmark" : "doc.on.doc")
                            .font(.system(size: 13, weight: .medium))
                        Text(copied ? "已复制" : "复制")
                            .font(.system(size: 12, weight: .medium))
                    }
                }
                .buttonStyle(OlGhostButtonStyle(hPadding: 8, vPadding: 3))
            }
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 12)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Color.zhLineSoft).frame(height: 0.5)
        }
    }

    /// 与历史页一致：润色失败/未产出时 finalText 为空，回退识别原文。
    private func copyEntry() {
        let text = session.finalText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? session.rawTranscript
            : session.finalText
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setString(text, forType: .string)
        copied = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
            copied = false
        }
    }

    /// 原版 formatTime：今天 → "HH:mm"；否则 → "M/d"。
    private func formatTime(_ iso: String) -> String {
        guard let d = OverviewModel.shared.parseISO8601(iso) else { return iso }
        let cal = Calendar.current
        if cal.isDateInToday(d) {
            let c = cal.dateComponents([.hour, .minute], from: d)
            return String(format: "%02d:%02d", c.hour ?? 0, c.minute ?? 0)
        }
        return "\(cal.component(.month, from: d))/\(cal.component(.day, from: d))"
    }
}

// MARK: - 年度活动热力图（1:1 对照 Heatmap.tsx 222 行）
//
// GitHub 贡献图式：列 = 周（周日起），行 = 星期；顶部月份标签，左侧 Mon/Wed/Fri。
// 颜色 sqrt 插值（minColor #bfdbfe → maxColor #1d4ed8，固定 hex 不随主题）；
// 零值格子用 surface-2。hover 显示原生 title 等价的黑底 tooltip（date · value）。
// 高度随格子自适应：HeatmapLayout 用父级宽度提案一次算出 cell 与总高（原版
// fitCell「贴边撑满」），再给 GeometryReader 有限提案，无两阶段测量。

struct ZhHeatmap: View {
    /// YYYY-MM-DD → 次数。
    let data: [String: Int]
    let startDate: Date
    let endDate: Date

    private static let monthLabels = (1...12).map { "\($0)月" }
    /// 原版 Intl zh-CN weekday short（anchor 2026-01-04 周日）。
    private static let dayLabels = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"]
    /// HeatmapLayout（同文件另一类型）也要用。
    fileprivate static let dayLabelWidth: CGFloat = 20

    struct Week {
        let days: [Day]
        let monthStart: Int?
    }
    struct Day {
        let iso: String
        let date: Date
        let value: Int?
    }

    /// hover 格子的 (列, 行, tooltip 文案) + 当时的 cell/gap（tooltip 定位用）。
    @State private var hovered: (col: Int, row: Int, text: String)?
    @State private var hoverCell: CGFloat = 11
    @State private var hoverStep: CGFloat = 14
    @State private var hoverTask: DispatchWorkItem?

    /// 原版 useMemo（73-107 行）：列从 startDate 所在周周日铺到 endDate；防御上限 260 周。
    private var weeksData: (weeks: [Week], max: Int) {
        let cal = Calendar.current
        var cursor = cal.startOfDay(for: startDate)
        let weekday = cal.component(.weekday, from: cursor) // 1=周日
        cursor = cal.date(byAdding: .day, value: -(weekday - 1), to: cursor) ?? cursor
        let startIso = OverviewView.isoString(startDate)
        let endIso = OverviewView.isoString(endDate)
        var columns: [Week] = []
        var maxValue = 0
        var lastMonth = -1
        while cursor <= endDate && columns.count < 260 {
            var days: [Day] = []
            var monthStart: Int? = nil
            for _ in 0..<7 {
                let iso = OverviewView.isoString(cursor)
                let inRange = iso >= startIso && iso <= endIso
                let value = inRange ? (data[iso] ?? 0) : nil
                if let v = value, v > maxValue { maxValue = v }
                // 月份标签挂在「该月 1 日所在列」上。
                if inRange, cal.component(.day, from: cursor) == 1,
                   cal.component(.month, from: cursor) != lastMonth {
                    let month = cal.component(.month, from: cursor)
                    monthStart = month
                    lastMonth = month
                }
                days.append(Day(iso: iso, date: cursor, value: value))
                cursor = cal.date(byAdding: .day, value: 1, to: cursor) ?? cursor
            }
            columns.append(Week(days: days, monthStart: monthStart))
        }
        return (columns, maxValue)
    }

    var body: some View {
        let wd = weeksData
        HeatmapLayout(weekCount: wd.weeks.count) {
            GeometryReader { geo in
                heatmapContent(weeks: wd.weeks, maxValue: wd.max, size: geo.size)
            }
        }
        .frame(maxWidth: .infinity)
        .overlay(alignment: .topLeading) {
            tooltipOverlay()
        }
    }

    /// 标签列 + 月份行 + 格子网格（cell 由可用宽度均分，原版 fitCell 公式）。
    private func heatmapContent(weeks: [Week], maxValue: Int, size: CGSize) -> some View {
        let gridWidth = Swift.max(size.width - Self.dayLabelWidth - 6, 1)
        let per = gridWidth / CGFloat(weeks.count)
        let g: CGFloat = per >= 13 ? 3 : 2
        let cell = Swift.max(5, per - g)
        let cellGap: CGFloat = cell >= 11 ? 3 : 2
        let step = cell + cellGap
        return HStack(spacing: 6) {
            dayLabelColumn(cell: cell, gap: cellGap)
                .frame(width: Self.dayLabelWidth)
            VStack(alignment: .leading, spacing: 0) {
                monthRow(step: step, cell: cell)
                cellGrid(weeks: weeks, maxValue: maxValue, cell: cell, gap: cellGap)
            }
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .clipped()
        }
        .frame(width: size.width, height: size.height, alignment: .topLeading)
    }

    /// 星期标签列：仅 Mon/Wed/Fri 可见（原版 158-175 行）。
    private func dayLabelColumn(cell: CGFloat, gap: CGFloat) -> some View {
        VStack(spacing: gap) {
            ForEach(0..<7, id: \.self) { i in
                Text(Self.dayLabels[i])
                    .font(.system(size: 9))
                    .foregroundStyle(Color.zhInk4)
                    .frame(height: cell, alignment: .leading)
                    .opacity((i == 1 || i == 3 || i == 5) ? 1 : 0) // Mon/Wed/Fri
            }
        }
        .padding(.top, 16)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// 月份标签行：absolute 在每月起始列上方（原版 178-195 行）。
    private func monthRow(step: CGFloat, cell: CGFloat) -> some View {
        ZStack(alignment: .topLeading) {
            ForEach(Array(weeksData.weeks.enumerated()), id: \.offset) { i, week in
                if let m = week.monthStart {
                    Text(Self.monthLabels[m - 1])
                        .font(.system(size: 9))
                        .foregroundStyle(Color.zhInk4)
                        .fixedSize()
                        .offset(x: CGFloat(i) * step, y: 0)
                }
            }
        }
        .frame(height: 14, alignment: .topLeading)
        .padding(.bottom, 2)
    }

    /// 格子区：每周一列 7 格（原版 196-218 行）。
    private func cellGrid(weeks: [Week], maxValue: Int, cell: CGFloat, gap: CGFloat) -> some View {
        HStack(alignment: .top, spacing: gap) {
            ForEach(Array(weeks.enumerated()), id: \.offset) { wi, week in
                VStack(spacing: gap) {
                    ForEach(Array(week.days.enumerated()), id: \.offset) { di, day in
                        cellView(day: day, col: wi, row: di, max: maxValue, cell: cell, gap: gap)
                    }
                }
            }
        }
    }

    private func cellView(day: ZhHeatmap.Day, col: Int, row: Int, max: Int, cell: CGFloat, gap: CGFloat) -> some View {
        let radius: CGFloat = cell < 8 ? 2 : 2.5
        let background: Color
        if let value = day.value {
            background = colorOf(value, max: max)
        } else {
            background = .clear // 范围外占位
        }
        return RoundedRectangle(cornerRadius: radius)
            .fill(background)
            .frame(width: cell, height: cell)
            .overlay(
                RoundedRectangle(cornerRadius: radius)
                    .stroke(Color.black.opacity(0.06), lineWidth: 0.5)
            )
            .onHover { hovering in
                handleHover(day: day, col: col, row: row, cell: cell, step: cell + gap, hovering: hovering)
            }
    }

    /// 原版 colorOf（109-119 行）：零值/全零 → surface-2；sqrt 插值。
    private func colorOf(_ value: Int, max: Int) -> Color {
        if max <= 0 || value <= 0 { return Color.zhSurface2 }
        let t = sqrt(Double(value) / Double(max))
        return Self.mixedColor(min(1, t))
    }

    /// mixHex(#bfdbfe, #1d4ed8, t)（原版 50-55 行，固定 hex 不随主题）。
    private static func mixedColor(_ t: Double) -> Color {
        let ca: [Double] = [0xBF, 0xDB, 0xFE].map { $0 / 255 }
        let cb: [Double] = [0x1D, 0x4E, 0xD8].map { $0 / 255 }
        let r = ca[0] + (cb[0] - ca[0]) * t
        let g = ca[1] + (cb[1] - ca[1]) * t
        let b = ca[2] + (cb[2] - ca[2]) * t
        return Color(red: r, green: g, blue: b)
    }

    // MARK: hover tooltip（原版原生 title：date · value，延迟出现）

    private func handleHover(day: ZhHeatmap.Day, col: Int, row: Int, cell: CGFloat, step: CGFloat, hovering: Bool) {
        hoverTask?.cancel()
        if hovering, let value = day.value {
            let dateStr = Self.dateDisplayFormatter.string(from: day.date)
            let text = "\(dateStr) · \(value) 次听写"
            let item = DispatchWorkItem {
                hoverCell = cell
                hoverStep = step
                hovered = (col, row, text)
            }
            hoverTask = item
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5, execute: item)
        } else {
            hovered = nil
        }
    }

    private static let dateDisplayFormatter: DateFormatter = {
        let f = DateFormatter()
        f.locale = Locale(identifier: "zh_CN")
        f.dateStyle = .medium // 「2026年8月14日」（原版 Intl dateStyle medium）
        f.timeStyle = .none
        return f
    }()

    /// 黑底白字小框，定位在格子中心（相对热力图根，含 26/16 偏移；原版 title 视觉近似）。
    private func tooltipOverlay() -> some View {
        Group {
            if let h = hovered {
                Text(h.text)
                    .font(.system(size: 11))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(RoundedRectangle(cornerRadius: 4).fill(Color.black.opacity(0.9)))
                    .fixedSize()
                    .position(
                        x: Self.dayLabelWidth + 6 + CGFloat(h.col) * hoverStep + hoverCell / 2,
                        y: 16 + CGFloat(h.row) * hoverStep + hoverCell / 2
                    )
            }
        }
        .allowsHitTesting(false)
    }
}

/// 热力图高度适配：用父级宽度提案算 cell/总高（原版 fitCell），
/// 再给子视图（GeometryReader）有限提案——高度随宽度一次成型。
private struct HeatmapLayout: Layout {
    var weekCount: Int

    struct Cache {
        var cell: CGFloat = 11
        var gap: CGFloat = 3
    }

    func makeCache(subviews: Subviews) -> Cache { Cache() }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout Cache) -> CGSize {
        let width = proposal.width ?? 0
        let gridWidth = max(width - ZhHeatmap.dayLabelWidth - 6, 1)
        let per = gridWidth / CGFloat(max(weekCount, 1))
        let g: CGFloat = per >= 13 ? 3 : 2
        cache.cell = max(5, per - g)
        cache.gap = cache.cell >= 11 ? 3 : 2
        // 高度 = 月份行(14+2) + 7 行格子 + paddingBottom 2（原版 158/176 行）。
        let height = 16 + 7 * cache.cell + 6 * cache.gap + 2
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout Cache) {
        subviews[0].place(
            at: bounds.origin,
            proposal: ProposedViewSize(width: bounds.width, height: bounds.height)
        )
    }
}
