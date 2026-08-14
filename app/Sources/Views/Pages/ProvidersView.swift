// 供应商管理页 — 1:1 复刻原版 ProvidersPage.tsx：
// - 通用 OpenAI 兼容 ASR 引擎管理（任意 name/url/apiKey/notes，无豆包/grok 专属分支）
// - 页头「添加」→ 表单卡（2 列 grid：名称*/URL*/API Key/备注）→ 列表卡
// - 测试连通性走异步事件（provider:test-result），结果全局显示（原版同款全局 state）
// - 删除无确认（原版直接 remove）；内置豆包不可删（core 层拦截）

import SwiftUI

struct ProvidersView: View {
    @ObservedObject var model = ProvidersModel.shared
    @State private var showAdd = false
    @State private var editingId: String?
    @State private var name = ""
    @State private var url = ""
    @State private var apiKey = ""
    @State private var notes = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                pageHeader
                if showAdd { formCard }
                listCard
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 24)
            .padding(.bottom, 8)
        }
        .onAppear { model.refresh() }
    }

    // MARK: 页头（原版 PageHeader：title 26/600 + desc 13 ink-3 + 右侧「添加」ghost sm）

    private var pageHeader: some View {
        HStack(alignment: .top, spacing: 24) {
            VStack(alignment: .leading, spacing: 0) {
                Text("供应商")
                    .font(.system(size: 26, weight: .semibold))
                    .kerning(-0.5) // 原版 letterSpacing -0.02em
                    .foregroundStyle(Color.zhInk)
                Text("管理第三方 ASR 引擎，OpenAI 兼容端点")
                    .font(.system(size: 13))
                    .foregroundStyle(Color.zhInk3)
                    .lineSpacing(1.55)
                    .padding(.top, 8)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Spacer()
            Button {
                startAdd()
            } label: {
                HStack(spacing: 6) {
                    // 原版 Btn icon="plus" size 13。
                    Image(systemName: "plus")
                        .font(.system(size: 13, weight: .medium))
                    Text("添加")
                }
            }
            .buttonStyle(OlGhostButtonStyle())
        }
        .padding(.bottom, 10) // 原版 PageHeader marginBottom 24（VStack spacing 14 已承担）
    }

    // MARK: 添加 / 编辑表单（原版 showAdd 区块：Card padding 20）

    private var formCard: some View {
        OlCard(padding: 20) {
            Text(editingId == nil ? "添加供应商" : "编辑供应商")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Color.zhInk)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.bottom, 14)

            // 原版 grid：2 列，gap "10px 16px"（行 10 / 列 16）。
            LazyVGrid(
                columns: [
                    GridItem(.flexible(), spacing: 16),
                    GridItem(.flexible()),
                ],
                spacing: 10
            ) {
                providerField(label: "名称 *", text: $name, placeholder: "我的供应商")
                providerField(label: "URL *", text: $url, placeholder: "https://your-worker.example.com/v1")
                providerSecureField(label: "API Key", text: $apiKey, placeholder: "sk-...")
                providerField(label: "备注", text: $notes, placeholder: "")
            }

            if let saveError = model.saveError {
                Text(saveError)
                    .font(.system(size: 11))
                    .foregroundStyle(Color.zhErr)
                    .padding(.top, 8)
            }

            // 原版：flex-end gap 8 marginTop 16；取消 ghost / 保存 primary（name/url 空禁用）。
            HStack(spacing: 8) {
                Spacer()
                Button("取消") { cancel() }
                    .buttonStyle(OlGhostButtonStyle())
                Button("保存") { save() }
                    .buttonStyle(OlGhostButtonStyle())
                    .disabled(
                        name.trimmingCharacters(in: .whitespaces).isEmpty
                        || url.trimmingCharacters(in: .whitespaces).isEmpty
                    )
            }
            .padding(.top, 16)
        }
    }

    private func providerField(label: String, text: Binding<String>, placeholder: String) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(label)
                .font(.system(size: 11.5, weight: .medium))
                .foregroundStyle(Color.zhInk4)
                .padding(.bottom, 4)
            TextField(placeholder, text: text)
                .textFieldStyle(ProviderInputStyle())
        }
    }

    private func providerSecureField(label: String, text: Binding<String>, placeholder: String) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(label)
                .font(.system(size: 11.5, weight: .medium))
                .foregroundStyle(Color.zhInk4)
                .padding(.bottom, 4)
            SecureField(placeholder, text: text)
                .font(.system(size: 13))
                .foregroundStyle(Color.zhInk)
                .padding(.horizontal, 10)
                .padding(.vertical, 7)
                .background(RoundedRectangle(cornerRadius: 6).fill(Color.zhSurface2))
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(Color.zhLineStrong, lineWidth: 0.5)
                )
        }
    }

    // MARK: 列表（原版 Card padding 0）

    private var listCard: some View {
        OlCard(padding: 0) {
            if model.providers.isEmpty {
                Text("暂无供应商，点击右上角「添加」")
                    .font(.system(size: 13))
                    .foregroundStyle(Color.zhInk4)
                    .frame(maxWidth: .infinity)
                    .padding(32)
            }
            ForEach(model.providers) { provider in
                providerRow(provider)
                    .overlay(alignment: .bottom) {
                        // 原版行 borderBottom 0.5px line-soft（最后一行也画，原版同）。
                        Rectangle()
                            .fill(Color.zhLineSoft)
                            .frame(height: 0.5)
                    }
            }
        }
    }

    private func providerRow(_ provider: ProvidersModel.Provider) -> some View {
        HStack(spacing: 14) {
            // 默认圆点（原版 20×20 圆钮：默认 = 6px 蓝环 + 白底，否则 1.5px ink-4 描边）。
            Button {
                model.setDefault(provider.id)
            } label: {
                Circle()
                    .fill(provider.isDefault ? Color.white : Color.clear)
                    .frame(width: 20, height: 20)
                    .overlay(
                        Circle()
                            .stroke(
                                provider.isDefault ? Color.zhBlue : Color.zhInk4,
                                lineWidth: provider.isDefault ? 6 : 1.5
                            )
                    )
                    // 非默认时 fill 透明：不加 contentShape 则透明区不参与命中测试，
                    // 只有 1.5px 描边环可点（SwiftUI 透明内容穿透行为）。
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("设为默认")

            // 信息：name 13/600 + 第二行 url — notes 11.5 ink-4 单行省略。
            VStack(alignment: .leading, spacing: 0) {
                Text(provider.name)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Color.zhInk)
                    .lineLimit(1)
                Text(subtitle(for: provider))
                    .font(.system(size: 11.5))
                    .foregroundStyle(Color.zhInk4)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .padding(.top, 2)
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            OlPill(tone: provider.isDefault ? .blue : .default, size: .sm) {
                Text(provider.isDefault ? "默认" : "备用")
            }

            // 测试结果全局显示（原版 testResult && testing === null 每行都渲染；
            // ok → --ol-ok #16a34a，err → --ol-err #dc2626）。
            if let result = model.testResult, model.testingId == nil {
                Text(result.msg)
                    .font(.system(size: 11))
                    .foregroundStyle(result.ok ? Color.zhOK : Color.zhErr)
            }

            HStack(spacing: 4) {
                Button(model.testingId == provider.id ? "测试中…" : "测试") {
                    model.test(provider.id)
                }
                .buttonStyle(OlGhostButtonStyle())
                .disabled(model.testingId == provider.id)
                Button("编辑") { startEdit(provider) }
                    .buttonStyle(OlGhostButtonStyle())
                Button("删除") { model.remove(provider.id) }
                    .buttonStyle(OlGhostButtonStyle())
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
    }

    private func subtitle(for provider: ProvidersModel.Provider) -> String {
        if let notes = provider.notes, !notes.isEmpty {
            return "\(provider.url) — \(notes)"
        }
        return provider.url
    }

    // MARK: 表单状态（原版 startAdd / startEdit / cancel / save）

    private func startAdd() {
        editingId = nil
        name = ""
        url = ""
        apiKey = ""
        notes = ""
        showAdd = true
        model.testResult = nil
    }

    private func startEdit(_ provider: ProvidersModel.Provider) {
        editingId = provider.id
        name = provider.name
        url = provider.url
        apiKey = provider.apiKey ?? ""
        notes = provider.notes ?? ""
        showAdd = true
        model.testResult = nil
    }

    private func cancel() {
        showAdd = false
        editingId = nil
        model.testResult = nil
    }

    private func save() {
        guard !name.trimmingCharacters(in: .whitespaces).isEmpty,
              !url.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        if model.save(
            name: name, url: url, apiKey: apiKey, notes: notes, editingId: editingId
        ) {
            cancel()
        }
    }
}

// MARK: - 输入框（原版 inputStyle：padding 7/10 + 13px + r6 + line-strong + surface-2）

private struct ProviderInputStyle: TextFieldStyle {
    func _body(configuration: TextField<Self._Label>) -> some View {
        configuration
            .font(.system(size: 13))
            .foregroundStyle(Color.zhInk)
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(RoundedRectangle(cornerRadius: 6).fill(Color.zhSurface2))
            .overlay(
                RoundedRectangle(cornerRadius: 6)
                    .stroke(Color.zhLineStrong, lineWidth: 0.5)
            )
    }
}
