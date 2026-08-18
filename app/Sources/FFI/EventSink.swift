// 事件回调 → @MainActor 分发。
//
// core 在任意线程以 NUL 结尾 JSON 推事件；本类持有 C 函数指针（强引用，
// 防提前释放），解析出事件名后路由到主线程。P1 起：
// - capsule:* → 胶囊状态机
// - device:changed → 麦克风设备列表刷新
// - app:* → 应用级状态（core-ready 等）

import Foundation

final class EventSink {
    static let shared = EventSink()

    /// 事件 → @MainActor 分发后的处理器列表（P1 各模块注册：胶囊、设置页等）。
    /// 只在主线程读写（addHandler 与 dispatch 的 main block 都在主线程）。
    private var handlers: [@MainActor (String, String) -> Void] = []

    func addHandler(_ handler: @escaping @MainActor (String, String) -> Void) {
        handlers.append(handler)
    }

    private let callback: EventCallback

    private init() {
        callback = { ptr in
            guard let ptr else { return }
            // core 在 Rust tokio 线程上回调：这里解析 JSON / NSLog 产生的 ObjC
            // 对象会进该线程的 autorelease pool——tokio 线程没有 run loop，
            // pool 永远不 drain（实测每事件滞留 1 个 NSConcreteData + 若干
            // NSDictionary/CFString，听写驱动，永久累积）。显式包 pool，
            // 每次回调的临时对象立即释放。
            autoreleasepool {
                let json = String(cString: ptr)
                EventSink.shared.dispatch(json)
            }
        }
    }

    /// 注册到 core。必须在 zhunji_init 之前调用。
    func register() {
        let result = zhunji_set_event_callback(callback)
        log("事件回调注册: \(result)")
    }

    private func dispatch(_ json: String) {
        // 解析 {event, payload}；解析失败只记日志不崩溃。
        guard let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let name = obj["event"] as? String
        else { return }
        let event = name
        var payload: String
        // payload 是任意 JSON 值（dict/string/number）——序列化回 JSON 字符串
        // 传给 handler（capsule:state 是对象，partial-text 是 {"text": ...}）。
        // 注意：JSON null 是 NSNull，不能喂给 JSONSerialization.data（会抛
        // ObjC exception，try? 捕不住，穿过 extern "C" 帧直接崩进程）。
        if let payloadObj = obj["payload"], !(payloadObj is NSNull),
           let data = try? JSONSerialization.data(withJSONObject: payloadObj),
           let s = String(data: data, encoding: .utf8)
        {
            payload = s
        } else {
            payload = "null"
        }
        log("事件: \(event)  payload=\(payload)")
        DispatchQueue.main.async {
            for handler in self.handlers {
                handler(event, payload)
            }
        }
    }

    private func log(_ message: String) {
        NSLog("[EventSink] %@", message)
    }
}
