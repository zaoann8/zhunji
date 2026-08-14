// 历史页 — 1:1 对照原版 History.tsx（1191 行）。
// 结构：PageHeader（kicker + title + desc + 刷新/清空）→ 双列 300px 1fr：
// 左列 = 搜索 + 汇总 + 筛选 chips + 按日分组列表；右列 = 详情（播放/导出/重转录/
// 删除 + 双框原文/结果 + 流水线明细）。豆包版无润色模式：筛选只有「全部」。
// 数据源：HistoryModel（list_history / delete / clear / read_audio / retranscribe /
// export）；事件：history:changed 全量刷新、history:retranscribed 局部替换。

import AppKit
import SwiftUI

struct HistoryView: View {
    @ObservedObject private var model = HistoryModel.shared
    @ObservedObject private var settings = SettingsModel.shared

    @State private var query = ""
    @State private var debouncedQuery = ""
    @State private var filter: String = "all"
    @State private var justCopied = false
    @State private var justCopiedRaw = false
    @FocusState private var focusSearch: Bool
    @State private var player = RecordingPlayer()

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            HStack(alignment: .top, spacing: 14) {
                listColumn
                    .frame(width: 300)
                detailColumn
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }
            .frame(maxHeight: .infinity, alignment: .top)
        }
        .padding(.horizontal, 28)
        .padding(.top, 24)
        .padding(.bottom, 32)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .onAppear {
            model.refresh()
            startDebounce()
        }
        .onChange(of: query) { startDebounce() }
        // 原版 window keydown：⌘K 聚焦搜索 / ⌘R 刷新（隐藏 Button 注册快捷键）。
        .background(
            Button("") { focusSearch = true }
                .keyboardShortcut("k", modifiers: .command)
                .hidden()
        )
        .background(
            Button("") { model.refresh() }
                .keyboardShortcut("r", modifiers: .command)
                .hidden()
        )
    }

    // MARK: - PageHeader（原版 _atoms.tsx：kicker 11/600/.08em + title 26/600 + desc）

    private var header: some View {
        HStack(alignment: .top, spacing: 24) {
            VStack(alignment: .leading, spacing: 0) {
                Text("HISTORY")
                    .font(.system(size: 11, weight: .semibold))
                    .kerning(0.88)
                    .foregroundStyle(Color.zhInk4)
                    .padding(.bottom, 8)
                Text("历史记录")
                    .font(.system(size: 26, weight: .semibold))
                    .kerning(-0.5)
                    .foregroundStyle(Color.zhInk)
                Text("本机保存的识别记录。")
                    .font(.system(size: 13))
                    .foregroundStyle(Color.zhInk3)
                    .lineSpacing(3)
                    .padding(.top, 8)
            }
            Spacer()
            HStack(spacing: 8) {
                OlGhostBtn("刷新") { model.refresh() }
                OlGhostBtn("清空") { confirmClear() }
            }
            .padding(.top, 2)
        }
        .padding(.bottom, 24)
    }

    // MARK: - 左列：搜索 + 列表

    private var listColumn: some View {
        VStack(spacing: 0) {
            // 搜索 / 汇总 / chips 头部（原版 padding 12px 14px + borderBottom）。
            VStack(alignment: .leading, spacing: 0) {
                // 搜索框（原版：padding 6/10、12px、r8、line-strong + surface-2 底）。
                HStack(spacing: 6) {
                    Image(systemName: "magnifyingglass")
                        .font(.system(size: 12))
                        .foregroundStyle(Color.zhInk3)
                    TextField("", text: $query, prompt: Text("搜索转写内容…（⌘K）"))
                        .textFieldStyle(.plain)
                        .font(.system(size: 12))
                        .foregroundStyle(Color.zhInk)
                        .focused($focusSearch)
                        .onSubmit { startDebounce() }
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(
                    RoundedRectangle(cornerRadius: 8).fill(Color.zhSurface2)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 8)
                        .stroke(Color.zhLineStrong, lineWidth: 0.5)
                )
                .keyboardShortcut("k", modifiers: .command) // 原版 ⌘K 聚焦搜索

                Text("共 \(model.items.count) 条 · 显示 \(filtered.count)")
                    .font(.system(size: 11))
                    .foregroundStyle(Color.zhInk4)
                    .padding(.top, 6)

                // 筛选 chips（豆包版只有「全部」；原版 chipSelectedStyle）。
                HStack(spacing: 4) {
                    chip(label: "全部", id: "all")
                }
                .padding(.top, 10)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .overlay(alignment: .bottom) {
                Rectangle().fill(Color.zhLine).frame(height: 0.5)
            }

            // 列表区（原版 padding 6 + overflow auto）。
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    if let actionError = model.actionError {
                        Text(actionError)
                            .font(.system(size: 12))
                            .foregroundStyle(Color(nsColor: .zhErr))
                            .lineSpacing(1.5)
                            .padding(10)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(Color.red.opacity(0.08))
                            .clipShape(RoundedRectangle(cornerRadius: 8))
                            .padding(8)
                    }
                    if model.loading {
                        Text("加载中…")
                            .font(.system(size: 12))
                            .foregroundStyle(Color.zhInk4)
                            .padding(16)
                    } else if let loadError = model.loadError {
                        VStack(alignment: .leading, spacing: 10) {
                            Text("加载历史失败：\(loadError)")
                                .font(.system(size: 12))
                                .foregroundStyle(Color.zhInk4)
                            OlGhostBtn("重试") { model.refresh() }
                        }
                        .padding(16)
                    } else if filtered.isEmpty {
                        Text(emptyText)
                            .font(.system(size: 12))
                            .foregroundStyle(Color.zhInk4)
                            .padding(16)
                    } else {
                        ForEach(HistoryModel.groupByDay(filtered), id: \.label) { group in
                            Text(group.label)
                                .font(.system(size: 11, weight: .semibold))
                                .kerning(0.33)
                                .foregroundStyle(Color.zhInk4)
                                .padding(.horizontal, 12)
                                .padding(.top, 10)
                                .padding(.bottom, 4)
                            ForEach(group.items, id: \.id) { session in
                                listRow(session)
                            }
                        }
                    }
                }
            }
            .scrollIndicators(.hidden)
        }
        .background(
            RoundedRectangle(cornerRadius: 14)
                .fill(Color.zhSurface)
                .shadow(color: Color.black.opacity(0.03), radius: 1, y: 1)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 14).stroke(Color.zhLine, lineWidth: 0.5)
        )
        .clipShape(RoundedRectangle(cornerRadius: 14))
    }

    /// 原版 chip：padding 3px 9px、11px、r999、chipSelectedStyle（选中 = pill-selected
    /// 黑底白字 / 深色蓝底）。豆包版仅「全部」。
    private func chip(label: String, id: String) -> some View {
        let selected = filter == id
        return Button {
            filter = id
        } label: {
            Text(label)
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(selected ? Color.zhPillSelectedInk : Color.zhInk3)
                .padding(.horizontal, 9)
                .padding(.vertical, 3)
                .background(
                    Capsule().fill(selected ? Color.zhPillSelectedBG : Color.clear)
                )
                .overlay(
                    Capsule().strokeBorder(
                        selected ? Color.zhPillSelectedBorder : Color.zhLineStrong,
                        lineWidth: 0.5
                    )
                )
        }
        .buttonStyle(.plain)
    }

    /// 列表条目（原版：padding 10/12、r8、选中 rgba(37,99,235,.06) + inset 2px 蓝条）。
    private func listRow(_ s: HistoryModel.Session) -> some View {
        let selected = model.selectedId == s.id
        return Button {
            model.selectedId = s.id
        } label: {
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text(HistoryModel.formatTime(s.createdAt))
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(Color.zhInk3)
                    Spacer()
                    Text(HistoryModel.formatDuration(s.durationMs))
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(Color.zhInk4)
                }
                Text(s.finalText.split(separator: "\n").first.map(String.init) ?? "")
                    .font(.system(size: 12))
                    .foregroundStyle(Color.zhInk2)
                    .lineLimit(2)
                    .lineSpacing(1.5)
                    .multilineTextAlignment(.leading)
                    .frame(maxWidth: .infinity, alignment: .leading)
                OlPill(tone: s.mode == "raw" ? .outline : .default, size: .sm) {
                    Text(HistoryModel.modeLabel(s.mode))
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(selected ? Color(nsColor: NSColor(hex: 0x2563EB).withAlphaComponent(0.06)) : .clear)
        )
        .overlay(alignment: .leading) {
            if selected {
                Rectangle().fill(Color.zhBlue).frame(width: 2)
                    .clipShape(UnevenRoundedRectangle(topLeadingRadius: 8, bottomLeadingRadius: 8))
            }
        }
        .padding(.horizontal, 6)
        .padding(.bottom, 1)
        .animation(.easeOut(duration: 0.16), value: selected)
    }

    // MARK: - 右列：详情

    private var detailColumn: some View {
        ScrollView {
            Group {
                if let item = displayedItem {
                    detailContent(item)
                } else {
                    Text(model.loading ? "加载中…"
                         : (model.loadError != nil ? "加载历史失败：\(model.loadError ?? "")" : "左侧选一条查看详情。"))
                        .font(.system(size: 13))
                        .foregroundStyle(Color.zhInk4)
                        .padding(.top, 40)
                        .frame(maxWidth: .infinity, alignment: .center)
                }
            }
        }
        .scrollIndicators(.hidden)
        .padding(20)
        .background(
            RoundedRectangle(cornerRadius: 14)
                .fill(Color.zhSurface)
                .shadow(color: Color.black.opacity(0.03), radius: 1, y: 1)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 14).stroke(Color.zhLine, lineWidth: 0.5)
        )
        .id(model.selectedId) // 原版 key={item.id}：切条目重建 → 播放器复位
        .onDisappear { player.stop() }
    }

    /// 原版 item = filtered.find(selectedId) || filtered[0]（只影响展示，不改选中态）。
    private var displayedItem: HistoryModel.Session? {
        if let selectedId = model.selectedId,
           let s = filtered.first(where: { $0.id == selectedId }) {
            return s
        }
        return filtered.first
    }

    private var filtered: [HistoryModel.Session] {
        let byMode = filter == "all" ? model.items
            : model.items.filter { $0.mode == filter }
        let q = debouncedQuery.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return byMode }
        return byMode.filter {
            $0.rawTranscript.lowercased().contains(q) || $0.finalText.lowercased().contains(q)
        }
    }

    private var emptyText: String {
        if !debouncedQuery.trimmingCharacters(in: .whitespaces).isEmpty {
            return "没有匹配「\(debouncedQuery.trimmingCharacters(in: .whitespaces))」的记录。"
        }
        let trigger = ShortcutRecorderView.comboParts(settings.dictationHotkey).joined()
        return "还没有历史记录。按 \(trigger) 录一段试试。"
    }

    // MARK: - 详情内容

    private func detailContent(_ item: HistoryModel.Session) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            // 头部行：时间 + 模式 + 录音时长 | 导出 / 重转录 / 删除
            HStack(spacing: 10) {
                Text(HistoryModel.formatTime(item.createdAt))
                    .font(.system(size: 13, design: .monospaced))
                    .foregroundStyle(Color.zhInk3)
                OlPill(tone: .default, size: .sm) {
                    Text(HistoryModel.modeLabel(item.mode))
                }
                Text("录音 \(HistoryModel.formatDuration(item.durationMs))")
                    .font(.system(size: 11))
                    .foregroundStyle(Color.zhInk4)
                Spacer()
                if item.hasAudioRecording == true, !model.audioMissingIds.contains(item.id) {
                    OlGhostBtn("导出录音") { exportAudio(item) }
                    if item.errorCode == "transcribeFailed" || item.errorCode == "emptyTranscript" {
                        OlGhostBtn(model.retranscribing ? "转录中…" : "重新转录") {
                            model.retranscribe(item.id)
                        }
                        .disabled(model.retranscribing)
                    }
                }
                OlGhostBtn("删除") { model.delete(item.id) }
            }
            .padding(.bottom, 14)

            // 录音播放器（原版 AudioRecordingPlayer）。
            if item.hasAudioRecording == true, !model.audioMissingIds.contains(item.id) {
                recordingPlayer(item)
                    .padding(.bottom, 14)
            }

            // 双框 vs 单框（原版：finalText 非空且 != rawTranscript → 双框）。
            if !item.finalText.isEmpty, item.finalText != item.rawTranscript {
                HStack(alignment: .top, spacing: 12) {
                    textBox(
                        title: "原文",
                        tone: .outline,
                        text: item.rawTranscript.isEmpty ? "（空）" : item.rawTranscript,
                        mono: true,
                        copyAction: copyRaw(item),
                        justCopied: justCopiedRaw
                    )
                    textBox(
                        title: HistoryModel.modeLabel(item.mode),
                        tone: .blue,
                        text: item.finalText,
                        mono: false,
                        copyAction: copyFinal(item),
                        justCopied: justCopied
                    )
                }
            } else {
                textBox(
                    title: HistoryModel.modeLabel(item.mode),
                    tone: .blue,
                    text: item.finalText.isEmpty ? item.rawTranscript : item.finalText,
                    mono: false,
                    copyAction: copyFinal(item),
                    justCopied: justCopied
                )
            }

            // 流水线明细（原版 grid auto 1fr auto：识别 / 润色 / 插入）。
            VStack(alignment: .leading, spacing: 7) {
                if item.asrProvider != nil || item.asrMs != nil {
                    pipelineRow(
                        step: "识别",
                        hint: true,
                        value: [item.asrProvider, item.asrModel].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "),
                        trailing: item.asrMs.map(HistoryModel.formatStepDuration) ?? ""
                    )
                }
                if item.llmProvider != nil || item.llmModel != nil || item.polishMs != nil {
                    pipelineRow(
                        step: "润色",
                        hint: false,
                        value: [item.llmProvider, item.llmModel].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "),
                        trailing: item.polishMs.map(HistoryModel.formatStepDuration) ?? ""
                    )
                }
                pipelineRow(
                    step: "插入",
                    hint: false,
                    value: insertSummary(item),
                    trailing: insertStatusText(item)
                )
            }
            .padding(.top, 14)
            .overlay(alignment: .top) {
                Rectangle().fill(Color.zhLineSoft).frame(height: 0.5)
            }
            .padding(.top, 18)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// 原文/结果文本框（原版：padding 14、r10、原文 border line + surface-2 底 /
    /// 结果 border blue + blue-soft 底；Pill + 复制按钮在头部）。
    private func textBox(
        title: String,
        tone: HistoryBoxTone,
        text: String,
        mono: Bool,
        copyAction: @escaping () -> Void,
        justCopied: Bool
    ) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                OlPill(tone: tone == .blue ? .blue : .outline, size: .sm) {
                    Text(title)
                }
                Spacer()
                OlGhostBtn(justCopied ? "已复制" : "复制") { copyAction() }
            }
            .padding(.bottom, 10)
            Text(text)
                .font(.system(size: 13))
                .foregroundStyle(tone == .blue ? Color.zhInk : Color.zhInk2)
                .lineSpacing(6)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 10)
                .fill(tone == .blue ? Color.zhBlueSoft : Color.zhSurface2)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .stroke(tone == .blue ? Color.zhBlue : Color.zhLine, lineWidth: 0.5)
        )
    }

    private enum HistoryBoxTone { case blue, outline }

    /// 流水线行：左步骤名（识别带 help 虚线）、中 provider · model、右耗时（mono）。
    private func pipelineRow(step: String, hint: Bool, value: String, trailing: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 14) {
            Group {
                if hint {
                    Text("识别")
                        .underline(pattern: .dot)
                        .help("松键后等待识别结果的耗时。流式识别边录边转，此值通常远小于录音时长。")
                } else {
                    Text(step)
                }
            }
            .font(.system(size: 11))
            .foregroundStyle(Color.zhInk4)
            .frame(width: 36, alignment: .leading)
            Text(value)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(Color.zhInk2)
                .frame(maxWidth: .infinity, alignment: .leading)
            Text(trailing)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(Color.zhInk4)
                .frame(alignment: .trailing)
        }
    }

    /// 插入行中列（原版：appName 加粗 + · N 字 + · N 个热词）。
    private func insertSummary(_ item: HistoryModel.Session) -> String {
        var parts: [String] = []
        if let appName = item.appName, !appName.isEmpty {
            parts.append(appName)
        }
        parts.append("\(item.finalText.count) 字")
        if let count = item.dictionaryEntryCount, count > 0 {
            parts.append("\(count) 个热词")
        }
        return parts.joined(separator: " · ")
    }

    private func insertStatusText(_ item: HistoryModel.Session) -> String {
        switch item.insertStatus {
        case "inserted": "已插入"
        case "pasteSent": "已尝试粘贴"
        case "copiedFallback": "已复制(需 ⌘V)"
        default: "插入失败"
        }
    }

    // MARK: - 录音播放器（原版 AudioRecordingPlayer：按钮加载 → 原生 audio controls）

    private func recordingPlayer(_ item: HistoryModel.Session) -> some View {
        let id = item.id
        return Group {
            switch player.status {
            case .idle:
                OlGhostBtn("播放录音") {
                    player.load(sessionId: id, model: model)
                }
            case .loading:
                OlGhostBtn("加载中…") {}
                    .disabled(true)
            case .ready:
                HStack(spacing: 10) {
                    Button {
                        player.togglePlayPause()
                    } label: {
                        Image(systemName: player.isPlaying ? "pause.fill" : "play.fill")
                            .font(.system(size: 11))
                            .frame(width: 14)
                    }
                    .buttonStyle(OlGhostButtonStyle(hPadding: 8, vPadding: 4))
                    Slider(
                        value: Binding(
                            get: { player.currentTime },
                            set: { player.seek(to: $0) }
                        ),
                        in: 0...max(player.duration, 0.01)
                    )
                    .controlSize(.small)
                    .frame(maxWidth: .infinity)
                    Text("\(timeString(player.currentTime)) / \(timeString(player.duration))")
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(Color.zhInk4)
                }
            case .error(let message):
                Text(message)
                    .font(.system(size: 11))
                    .foregroundStyle(Color.zhErr)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func timeString(_ t: TimeInterval) -> String {
        let total = Int(t)
        return "\(total / 60):\(String(format: "%02d", total % 60))"
    }

    // MARK: - 动作（原版 onCopy / onCopyRaw / onExportAudio / onClear）

    private func copyFinal(_ item: HistoryModel.Session) -> () -> Void {
        { [self] in
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(
                item.finalText.trimmingCharacters(in: .whitespaces).isEmpty
                    ? item.rawTranscript : item.finalText,
                forType: .string
            )
            justCopied = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { justCopied = false }
        }
    }

    private func copyRaw(_ item: HistoryModel.Session) -> () -> Void {
        { [self] in
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(item.rawTranscript, forType: .string)
            justCopiedRaw = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { justCopiedRaw = false }
        }
    }

    private func exportAudio(_ item: HistoryModel.Session) {
        var error: String?
        let ok = model.exportAudio(item.id, error: &error)
        if let error, !error.contains("not found") {
            model.actionError = error
        }
    }

    private func confirmClear() {
        guard !model.items.isEmpty else { return }
        let alert = NSAlert()
        alert.messageText = "确定清空全部 \(model.items.count) 条记录？此操作不可恢复。"
        alert.alertStyle = .warning
        alert.addButton(withTitle: "清空")
        alert.addButton(withTitle: "取消")
        if alert.runModal() == .alertFirstButtonReturn {
            model.clear()
        }
    }

    // MARK: - 搜索防抖（原版 300ms debounce）

    private func startDebounce() {
        let text = query
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
            if text == query {
                debouncedQuery = text
            }
        }
    }
}
