// P0.4 FFI 冒烟：验证 C ABI 链路 zhunji_set_event_callback → zhunji_init → 事件回调 → zhunji_request_shutdown。
// 编译运行见 run_smoke.sh。

import Foundation

typealias EventCallback = @convention(c) (UnsafePointer<CChar>?) -> Void

@_silgen_name("zhunji_init")
func zhunji_init() -> Int32

@_silgen_name("zhunji_set_event_callback")
func zhunji_set_event_callback(_ callback: @escaping EventCallback) -> Int32

@_silgen_name("zhunji_request_shutdown")
func zhunji_request_shutdown()

var received: [String] = []

// 回调在任意 core 线程触发；@escaping + 顶层全局持有保证不被释放。
let callback: EventCallback = { ptr in
    guard let ptr else { return }
    let json = String(cString: ptr)
    print("[swift] event: \(json)")
    received.append(json)
}

// 调用约定：先注册回调，再 init（core-ready 事件才能收到）。
let reg = zhunji_set_event_callback(callback)
print("[swift] zhunji_set_event_callback -> \(reg)")
guard reg == 0 else {
    fatalError("回调注册失败（重复注册？）")
}

// zhunji_init 必须从主线程调用（global-hotkey manager 构造约束）。
let code = zhunji_init()
print("[swift] zhunji_init -> \(code)")
guard code == 0 else {
    fatalError("zhunji_init 失败")
}

// 幂等：重复 init 应返回 0 且不重复初始化。
let again = zhunji_init()
print("[swift] zhunji_init (repeat) -> \(again)")

// 等 core 初始化线程跑完（设备 watcher 注册、引擎预热）。
Thread.sleep(forTimeInterval: 1.5)

// SMOKE_HOLD_SECONDS：shutdown 前额外驻留，供外部 ps 测 RSS（P0.5 决策门）。
let hold = Double(ProcessInfo.processInfo.environment["SMOKE_HOLD_SECONDS"] ?? "0") ?? 0
if hold > 0 {
    print("[swift] holding \(hold)s for RSS measurement…")
    Thread.sleep(forTimeInterval: hold)
}

print("[swift] zhunji_request_shutdown")
zhunji_request_shutdown()
Thread.sleep(forTimeInterval: 0.3)

let readyEvents = received.filter { $0.contains("app:core-ready") }
guard !readyEvents.isEmpty else {
    fatalError("未收到 app:core-ready 事件，事件回调链路不通")
}
print("[swift] PASS: 收到 \(received.count) 条事件，包含 app:core-ready")
