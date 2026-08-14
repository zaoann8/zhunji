// 胶囊状态模型 — 解析 core 的 capsule:state / partial-text 事件。
// payload 形状对照 core/src/types.rs 的 CapsulePayload（serde camelCase）。

import Foundation

/// CapsulePayload 的 Swift 镜像（只取 P1.2 需要的字段）。
struct CapsuleStatePayload: Decodable {
    let state: String
    let level: Double
    let message: String?
    let insertedChars: Int?
    let translation: Bool
    let warming: Bool
    let capsuleStyle: String?
}

/// 显示阶段：hidden → entering（capsule-in 动画）→ visible → leaving（capsule-out）。
enum CapsulePhase: Equatable {
    case hidden, entering, visible, leaving
}

@MainActor
final class CapsuleModel: ObservableObject {
    /// 扫光慢速切换的延时任务（2 秒后 fast → slow）。
    private var slowDownTask: Task<Void, Never>?
    @Published var phase: CapsulePhase = .hidden
    @Published var state: String = "idle"
    @Published var level: Double = 0
    @Published var message: String?
    @Published var insertedChars: Int?
    @Published var translation: Bool = false
    @Published var warming: Bool = false
    @Published var capsuleStyle: String = "classic"
    @Published var partialText: String?
    /// "thinking" 扫光速度：进入 transcribing/polishing 的头 2 秒 fast（0.9s/cycle，
    /// 提示「流式刚开始」），之后切 slow（2.4s）稳态。原版 Capsule.tsx shineFast 逻辑。
    @Published var shineFast: Bool = true

    /// 可交互状态（关鼠标穿透）：recording / transcribing / polishing。
    var interactive: Bool {
        state == "recording" || state == "transcribing" || state == "polishing"
    }

    /// capsule:state 事件。
    func applyState(_ json: String) {
        guard let data = json.data(using: .utf8),
              let payload = try? JSONDecoder().decode(CapsuleStatePayload.self, from: data)
        else {
            NSLog("[Capsule] capsule:state 解析失败: %@", json)
            return
        }
        level = payload.level
        message = payload.message
        if let inserted = payload.insertedChars {
            insertedChars = inserted
        }
        translation = payload.translation
        warming = payload.warming
        if let style = payload.capsuleStyle {
            capsuleStyle = style
        }
        let next = payload.state
        state = next
        if next == "transcribing" || next == "polishing" {
            shineFast = true
            slowDownTask?.cancel()
            slowDownTask = Task { [weak self] in
                try? await Task.sleep(nanoseconds: 2_000_000_000)
                guard !Task.isCancelled else { return }
                self?.shineFast = false
            }
        } else {
            shineFast = true
            slowDownTask?.cancel()
            slowDownTask = nil
        }
        if next == "idle" {
            // 从可见态过渡 → leaving；从未可见过（idle→idle）保持 hidden。
            if phase != .hidden {
                phase = .leaving
            }
        } else {
            if phase == .leaving || phase == .hidden {
                phase = .entering
            }
        }
    }

    /// partial-text 事件（payload {"text": "..."}）。
    func applyPartial(_ json: String) {
        guard let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let text = obj["text"] as? String
        else { return }
        partialText = text
    }
}
