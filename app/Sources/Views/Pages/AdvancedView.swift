// 高级页 — 1:1 复刻原版 DebugToolsSection.tsx（tabs.tsx 的 AdvancedTab 仅此一节，无页头）：
// - 调试工具 Card：保留原始录音 Toggle + 保留条数 number input + 导出错误日志
// - 保留条数：留空 → null（不限制，core 走 200 默认）；非法输入不保存（受控回写原值）；
//   合法值 clamp(1,200) 回写
// - 导出：NSSavePanel（默认名 zhunji-{UTC ts}.log，Log/TXT 过滤器）→ core 复制
//   zhunji.log → 成功显示「已保存」（悬停看路径）4s 后回 idle；失败红字「导出失败：…」

import SwiftUI
import AppKit

struct AdvancedView: View {
    @ObservedObject var model = SettingsModel.shared

    /// 导出状态机（原版 exportStatus idle/busy/ok/err + 4s 自动回 idle）。
    private enum ExportStatus { case idle, busy, ok, err }
    @State private var exportStatus: ExportStatus = .idle
    @State private var exportMessage = ""

    /// 保留条数输入（受控：合法值 clamp 回写，非法输入恢复 prefs 原值）。
    @State private var maxEntriesText = ""

    var body: some View {
        ScrollView {
            VStack(spacing: 16) {
                debugCard
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 24)
            .padding(.bottom, 8)
        }
        .onAppear { syncMaxEntriesText() }
        // prefs 变化（本页保存 / 其他入口）→ 输入框同步受控值。
        .onChange(of: model.audioRecordingMaxEntries) { _ in syncMaxEntriesText() }
    }

    // MARK: 调试工具（原版 DebugToolsSection：Card + SectionTitle）

    private var debugCard: some View {
        OlCard {
            SectionTitle { Text("调试工具") }

            SettingRow(label: "保留原始录音（调试）") {
                OlToggle(on: model.recordAudioForDebug) { on in
                    model.recordAudioForDebug = on
                    model.save(["recordAudioForDebug": on])
                }
            }

            // 原版 number input：min 1 / max 200 / placeholder 200 / 宽 80 右对齐 /
            // disabled=!recordAudioForDebug；留空落 null 走 core 200 默认。
            SettingRow(label: "原始录音保留条数") {
                TextField("200", text: $maxEntriesText)
                    .textFieldStyle(OlInputStyle())
                    .frame(width: 80)
                    .multilineTextAlignment(.trailing)
                    .disabled(!model.recordAudioForDebug)
                    .onChange(of: maxEntriesText) { newValue in
                        onMaxEntriesChange(newValue)
                    }
            }

            exportRow
        }
    }

    // MARK: 导出错误日志（原版 exportErrorLog：save dialog + fs::copy）

    private var exportRow: some View {
        SettingRow(label: "导出错误日志") {
            HStack(spacing: 8) {
                Button {
                    exportLog()
                } label: {
                    Text(exportStatus == .busy ? "导出中…" : "导出")
                }
                .buttonStyle(OlGhostButtonStyle())
                .disabled(exportStatus == .busy)

                if exportStatus == .ok {
                    // 原版 desktop 只显示「已保存」，路径放 title 悬停。
                    Text("已保存")
                        .font(.system(size: 11))
                        .foregroundStyle(Color.zhOK)
                        .lineLimit(1)
                        .help(exportMessage)
                } else if exportStatus == .err {
                    Text(exportMessage.isEmpty ? "导出失败" : "导出失败：\(exportMessage)")
                        .font(.system(size: 11))
                        .foregroundStyle(Color.zhErr)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .help(exportMessage)
                }
            }
        }
    }

    // MARK: 行为

    /// 保留条数输入（原版 onAudioRecordingMaxEntriesChange：空 → null，
    /// 解析失败 → 忽略，合法 → clamp(1,200) 保存）。
    private func onMaxEntriesChange(_ raw: String) {
        let trimmed = raw.trimmingCharacters(in: .whitespaces)
        if trimmed.isEmpty {
            model.audioRecordingMaxEntries = nil
            model.save(["audioRecordingMaxEntries": NSNull()])
            return
        }
        // 原版 parseInt 截断小数 → 用 Double 解析 + Int 截断对齐。
        guard let parsed = Double(trimmed), parsed.isFinite else {
            // 解析失败：原版 onChange 直接 return，受控 input 回显示原值。
            syncMaxEntriesText()
            return
        }
        let clamped = min(200, max(1, Int(parsed)))
        model.audioRecordingMaxEntries = clamped
        model.save(["audioRecordingMaxEntries": clamped])
    }

    private func syncMaxEntriesText() {
        maxEntriesText = model.audioRecordingMaxEntries.map(String.init) ?? ""
    }

    private func exportLog() {
        exportStatus = .busy
        exportMessage = ""
        let panel = NSSavePanel()
        panel.nameFieldStringValue = defaultLogName()
        panel.allowedContentTypes = [.log, .plainText] // 原版 filters Log: log/txt
        panel.canCreateDirectories = true
        guard panel.runModal() == .OK, let url = panel.url else {
            exportStatus = .idle // 取消 → 原版返回 null → idle
            return
        }
        let path = url.path
        let json = path.withCString { cpath in
            coreJsonString { zhunji_export_error_log(cpath) }
        }
        guard let json,
              let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            exportStatus = .err
            exportMessage = "core 返回异常"
            return
        }
        if let error = obj["error"] as? String {
            exportStatus = .err
            exportMessage = error
            return
        }
        exportStatus = .ok
        exportMessage = path
        // 4s 后回 idle（原版 setTimeout 4000，仅 ok 态才重置）。
        DispatchQueue.main.asyncAfter(deadline: .now() + 4) {
            if exportStatus == .ok { exportStatus = .idle }
        }
    }

    /// 默认文件名（原版 ts = ISO 去 : . 截 19）→ zhunji-2026-08-14T12-34-56.log
    private func defaultLogName() -> String {
        let iso = ISO8601DateFormatter().string(from: Date())
        let clean = iso.replacingOccurrences(of: ":", with: "-")
            .replacingOccurrences(of: ".", with: "-")
        return "zhunji-\(clean.prefix(19)).log"
    }
}
