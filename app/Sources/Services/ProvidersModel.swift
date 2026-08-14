// 供应商管理数据模型（原版 ProvidersPage.tsx）：
// - 通用 OpenAI 兼容 ASR 引擎：任意 name/url/apiKey/notes（无豆包/grok 专属逻辑）
// - 内置豆包常驻列表首位（core providers.rs read() 保证），不可删除
// - list/add/update/remove/set_default 同步；test_provider 异步事件回调
// - 设默认 = providers.json default + prefs.activeAsrProvider 同步（core 内完成）

import Foundation

@MainActor
final class ProvidersModel: ObservableObject {
    static let shared = ProvidersModel()

    /// 原版 Provider（camelCase；`default` 为保留字用 CodingKeys）。
    struct Provider: Decodable, Identifiable {
        let id: String
        let name: String
        let url: String
        let apiKey: String?
        let notes: String?
        let isDefault: Bool

        enum CodingKeys: String, CodingKey {
            case id, name, url, apiKey, notes
            case isDefault = "default"
        }
    }

    @Published var providers: [Provider] = []
    @Published var testingId: String?
    @Published var testResult: (ok: Bool, msg: String)?
    /// 表单提交失败的错误提示（原版 console.error；Swift 侧展示在表单卡内）。
    @Published var saveError: String?

    private init() {}

    func refresh() {
        guard let json = coreJsonString(zhunji_list_providers),
              let data = json.data(using: .utf8),
              let arr = try? JSONDecoder().decode([Provider].self, from: data)
        else { return }
        providers = arr
    }

    // MARK: - CRUD（原版 save / remove / setDefault）

    /// 新增或更新（editingId nil = 新增）。返回是否成功。
    @discardableResult
    func save(name: String, url: String, apiKey: String, notes: String, editingId: String?) -> Bool {
        saveError = nil
        let payload: [String: Any] = [
            "id": editingId ?? "",
            "name": name.trimmingCharacters(in: .whitespaces),
            "url": url.trimmingCharacters(in: .whitespaces),
            "apiKey": apiKey.trimmingCharacters(in: .whitespaces),
            "notes": notes.trimmingCharacters(in: .whitespaces),
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8)
        else { return false }
        if editingId != nil {
            let result = json.withCString { zhunji_update_provider($0) }
            if result != 0 {
                saveError = "保存失败"
                return false
            }
        } else {
            // add_provider 返回新 Provider JSON 或 {"error":"..."}（指针）。
            guard let out = json.withCString({ zhunji_add_provider($0) }) else {
                saveError = "保存失败"
                return false
            }
            defer { zhunji_free_string(out) }
            let resultJson = String(cString: out)
            if let data = resultJson.data(using: .utf8),
               let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let err = obj["error"] as? String {
                saveError = err
                return false
            }
        }
        refresh()
        return true
    }

    func remove(_ id: String) {
        let result = id.withCString { zhunji_remove_provider($0) }
        if result == 0 { refresh() }
    }

    /// 设为默认（core 同步 prefs.activeAsrProvider + 发 prefs:changed）。
    func setDefault(_ id: String) {
        let result = id.withCString { zhunji_set_default_provider($0) }
        if result == 0 { refresh() }
    }

    /// 测试连通性（原版 testProvider：GET {url}/v1/models + Bearer）。
    func test(_ id: String) {
        guard testingId == nil else { return }
        testingId = id
        testResult = nil
        id.withCString { zhunji_test_provider($0) }
    }

    /// provider:test-result 事件回调。
    func applyTestResult(_ payload: [String: Any]) {
        guard let id = payload["id"] as? String, id == testingId else { return }
        testingId = nil
        if payload["ok"] as? Bool == true {
            testResult = (true, "连接成功")
        } else {
            testResult = (false, payload["error"] as? String ?? "测试失败")
        }
    }
}
