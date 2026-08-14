// 词典页 — 1:1 复刻原版 DictionaryPage.tsx：
// - 热词管理，识别时自动增强匹配（keyterm 透传）
// - 添加卡（input ≤50 字符 + Enter 添加 + 错误红字）+ 列表卡（chips + × 删除）
// - 状态仅 terms/input/error 三个（原版同款），FFI 直调，无需 model 层

import SwiftUI

struct DictionaryView: View {
    @State private var terms: [String] = []
    @State private var input = ""
    @State private var error = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                pageHeader
                addCard
                listCard
            }
            .padding(.horizontal, 28)
            .padding(.vertical, 24)
            .padding(.bottom, 8)
        }
        .onAppear { refresh() }
    }

    // MARK: 页头（原版 PageHeader：desc 动态计数 terms.length / 100）

    private var pageHeader: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("词典")
                .font(.system(size: 26, weight: .semibold))
                .kerning(-0.5) // 原版 letterSpacing -0.02em
                .foregroundStyle(Color.zhInk)
            Text("热词偏置 · 识别时自动增强匹配 · \(terms.count) / 100")
                .font(.system(size: 13))
                .foregroundStyle(Color.zhInk3)
                .lineSpacing(1.55)
                .padding(.top, 8)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.bottom, 10) // 原版 PageHeader marginBottom 24（VStack spacing 14 已承担）
    }

    // MARK: 添加卡（原版 Card padding 16 marginBottom 14）

    private var addCard: some View {
        OlCard(padding: 16) {
            HStack(spacing: 8) {
                TextField("添加热词（≤ 50 字符）", text: $input)
                    .textFieldStyle(DictionaryInputStyle())
                    .onSubmit { add() }
                    .onChange(of: input) { newValue in
                        // 原版 input maxLength={50}。
                        if newValue.count > 50 {
                            input = String(newValue.prefix(50))
                        }
                        error = ""
                    }
                Button("添加") { add() }
                    .buttonStyle(OlGhostButtonStyle())
                    .disabled(input.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            if !error.isEmpty {
                Text(error)
                    .font(.system(size: 11.5))
                    .foregroundStyle(Color.zhErr)
                    .padding(.top, 6)
            }
        }
    }

    // MARK: 列表卡（原版 Card padding 0）

    private var listCard: some View {
        OlCard(padding: 0) {
            if terms.isEmpty {
                Text("暂无热词，添加后识别时自动增强匹配")
                    .font(.system(size: 13))
                    .foregroundStyle(Color.zhInk4)
                    .frame(maxWidth: .infinity)
                    .padding(32)
            } else {
                // 原版 flexWrap gap 6 padding 14。
                LazyVGrid(
                    columns: [GridItem(.adaptive(minimum: 60), spacing: 6)],
                    alignment: .leading,
                    spacing: 6
                ) {
                    ForEach(terms, id: \.self) { term in
                        termChip(term)
                    }
                }
                .padding(14)
            }
        }
    }

    /// 热词 chip（原版：padding 4/12、r14、blue-soft 底、blue 字 13/500 + × 删除）。
    private func termChip(_ term: String) -> some View {
        HStack(spacing: 6) {
            Text(term)
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(Color.zhBlue)
                .lineLimit(1)
            Button {
                remove(term)
            } label: {
                Text("×")
                    .font(.system(size: 14))
                    .foregroundStyle(Color.zhBlue)
                    .opacity(0.5)
                    .frame(width: 12, height: 12)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 4)
        .background(
            Capsule().fill(Color.zhBlueSoft)
        )
        .fixedSize()
    }

    // MARK: 操作（原版 add / remove / refresh）

    private func add() {
        let t = input.trimmingCharacters(in: .whitespaces)
        guard !t.isEmpty else { return }
        guard let out = t.withCString({ zhunji_add_term($0) }) else { return }
        defer { zhunji_free_string(out) }
        let json = String(cString: out)
        guard let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }
        if let err = obj["error"] as? String {
            error = err
        } else {
            input = ""
            error = ""
            refresh()
        }
    }

    private func remove(_ term: String) {
        let code = term.withCString { zhunji_remove_term($0) }
        if code == 0 { refresh() }
    }

    private func refresh() {
        guard let json = coreJsonString(zhunji_list_terms),
              let data = json.data(using: .utf8),
              let list = try? JSONDecoder().decode([String].self, from: data)
        else { return }
        terms = list
    }
}

// MARK: - 输入框（原版 DictionaryPage inputStyle：padding 7/12 + 13px + r6 + line-strong + surface-2）

private struct DictionaryInputStyle: TextFieldStyle {
    func _body(configuration: TextField<Self._Label>) -> some View {
        configuration
            .font(.system(size: 13))
            .foregroundStyle(Color.zhInk)
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .background(RoundedRectangle(cornerRadius: 6).fill(Color.zhSurface2))
            .overlay(
                RoundedRectangle(cornerRadius: 6)
                    .stroke(Color.zhLineStrong, lineWidth: 0.5)
            )
    }
}
