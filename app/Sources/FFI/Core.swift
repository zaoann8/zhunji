// C ABI 声明——Swift 侧唯一碰 Rust 核心的地方。
// 对应 core/src/ffi.rs 的导出符号。

import Foundation

/// Rust 侧事件回调类型：`extern "C" fn(*const c_char)`，载荷是 NUL 结尾的
/// JSON 字符串 `{"event":"<name>","payload":<json>}`，任意 core 线程触发。
typealias EventCallback = @convention(c) (UnsafePointer<CChar>?) -> Void

/// 初始化 core（幂等）。必须从主线程调用——global-hotkey 的 manager
/// 要求主线程构造（core 的 warmup 依赖此约定）。
/// 返回 0 成功（或已初始化），1 失败。
@_silgen_name("zhunji_init")
func zhunji_init() -> Int32

/// 注册事件回调。已注册过则保留第一个（返回 1），首次注册成功返回 0。
/// 必须在 zhunji_init 之前调用（否则 app:core-ready 事件丢失）。
/// core 只存裸函数指针，Swift 侧必须持强引用防止提前释放。
@_silgen_name("zhunji_set_event_callback")
func zhunji_set_event_callback(_ callback: @escaping EventCallback) -> Int32

/// 请求 core 退出：supervisor 线程 + 设备监听线程停止。
@_silgen_name("zhunji_request_shutdown")
func zhunji_request_shutdown()

/// 胶囊「取消」按钮：取消当前会话（同步）。
@_silgen_name("zhunji_capsule_cancel")
func zhunji_capsule_cancel()

/// 胶囊「确认」按钮：停止听写并提交插入（core 内部异步执行）。
@_silgen_name("zhunji_capsule_confirm")
func zhunji_capsule_confirm()

// MARK: - P1.4 偏好 / 凭据 / 设备

/// 读取偏好全量 JSON（camelCase）。返回 core 堆分配字符串，用完必须
/// zhunji_free_string 释放。
@_silgen_name("zhunji_get_prefs")
func zhunji_get_prefs() -> UnsafeMutablePointer<CChar>?

/// 保存偏好（JSON 字符串；缺失字段走 serde default）。
/// 返回 0 成功；热键字段变化 core 侧自动重注册。
@_silgen_name("zhunji_set_prefs")
func zhunji_set_prefs(_ json: UnsafePointer<CChar>) -> Int32

/// 麦克风设备列表 JSON `[{"name":..,"isDefault":..}]`（同上需释放）。
@_silgen_name("zhunji_list_microphone_devices")
func zhunji_list_microphone_devices() -> UnsafeMutablePointer<CChar>?

/// ASR 供应商注册表 JSON `[{"id":..,"name":..,"default":..}]`
/// （引擎下拉数据源；内置豆包常驻，同上需释放）。
@_silgen_name("zhunji_list_providers")
func zhunji_list_providers() -> UnsafeMutablePointer<CChar>?

/// 释放 core 分配的字符串。
@_silgen_name("zhunji_free_string")
func zhunji_free_string(_ ptr: UnsafeMutablePointer<CChar>?)

// MARK: - P1.4 权限页（热键状态 / 网络检查 / 电平监听）

/// 热键监听状态 JSON `{"adapter","state","message"}`（同上需释放）。
@_silgen_name("zhunji_get_hotkey_status")
func zhunji_get_hotkey_status() -> UnsafeMutablePointer<CChar>?

/// 发起网络连通性检查（异步，结果经 `network:result` 事件回调）。
/// 返回 0 已发起，1 core 未初始化。
@_silgen_name("zhunji_check_network")
func zhunji_check_network() -> Int32

/// 开始监听麦克风电平（deviceName 空 = 系统默认；已有监听先停）。
/// 电平 0..1 经 `microphone:level` 事件回调。返回 0 成功。
/// 注意：内部同步构造音频流（数十 ms），调用方应放后台队列。
@_silgen_name("zhunji_start_microphone_level_monitor")
func zhunji_start_microphone_level_monitor(_ deviceName: UnsafePointer<CChar>) -> Int32

/// 停止电平监听（幂等）。
@_silgen_name("zhunji_stop_microphone_level_monitor")
func zhunji_stop_microphone_level_monitor()

// MARK: - P2a 概览页（历史 / 活动 / 凭据 / 引擎状态）

/// 历史列表 JSON（camelCase，同 history.json；同上需释放）。
@_silgen_name("zhunji_list_history")
func zhunji_list_history() -> UnsafeMutablePointer<CChar>?

/// 年度活动计数 JSON `[{"date":"YYYY-MM-DD","count":n}...]`（同上需释放）。
@_silgen_name("zhunji_get_activity_stats")
func zhunji_get_activity_stats() -> UnsafeMutablePointer<CChar>?

/// 凭据状态 JSON `{"activeAsrProvider","asrConfigured"}`（同上需释放）。
@_silgen_name("zhunji_get_credentials")
func zhunji_get_credentials() -> UnsafeMutablePointer<CChar>?

/// 引擎上次会话状态 JSON `{"ok":bool,"error":string|null}`（同上需释放）。
@_silgen_name("zhunji_get_engine_status")
func zhunji_get_engine_status() -> UnsafeMutablePointer<CChar>?

/// 发起引擎连通性测试（异步，结果经 `engine:test-result` 事件回调）。
/// 返回 0 已发起，1 core 未初始化。
@_silgen_name("zhunji_test_engine")
func zhunji_test_engine() -> Int32

// MARK: - 历史页（P2）

/// 删除一条历史（原版 delete_history_entry），删除后发 `history:changed` 事件。
/// 返回 0 成功，1 未初始化，2 删除失败。
@_silgen_name("zhunji_delete_history_entry")
func zhunji_delete_history_entry(_ id: UnsafePointer<CChar>) -> Int32

/// 清空历史（原版 clear_history），完成后发 `history:changed` 事件。
/// 返回 0 成功，1 未初始化，2 清空失败。
@_silgen_name("zhunji_clear_history")
func zhunji_clear_history() -> Int32

/// 读取录音 wav → data URL JSON：`{"data":"data:audio/wav;base64,..."}` 或 `{"error":"..."}`。
@_silgen_name("zhunji_read_audio_recording")
func zhunji_read_audio_recording(_ sessionId: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

/// 异步重转录（原版 retranscribe_recording），完成发 `history:retranscribed` 事件。
/// 返回 0 已发起，1 未初始化，2 id 非法。
@_silgen_name("zhunji_retranscribe_recording")
func zhunji_retranscribe_recording(_ sessionId: UnsafePointer<CChar>) -> Int32

/// 导出录音 wav 到目标路径（对话框由 Swift 侧 NSSavePanel 负责）。
/// 返回 0 成功，1 未初始化，2 id 非法，3 录音不存在，4 复制失败。
@_silgen_name("zhunji_export_audio_recording")
func zhunji_export_audio_recording(
    _ sessionId: UnsafePointer<CChar>,
    _ destPath: UnsafePointer<CChar>
) -> Int32

// MARK: - 供应商管理页（P2）

/// 新增供应商（通用 OpenAI 兼容 ASR 端点，无内置专属分支）。
/// 入参 JSON `{"name","url","apiKey","notes"}`；返回新 Provider JSON，
/// 失败 `{"error":"..."}`（同上需释放）。新增永不自动设为默认（原版 add_provider）。
@_silgen_name("zhunji_add_provider")
func zhunji_add_provider(_ json: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

/// 更新供应商。入参 JSON 含 `"id"`；返回 0 成功，1 未初始化，2 参数缺失，
/// 3 未找到，4 内置供应商不可编辑。
@_silgen_name("zhunji_update_provider")
func zhunji_update_provider(_ json: UnsafePointer<CChar>) -> Int32

/// 删除供应商。返回 0 成功，1 未初始化，2 未找到，3 内置供应商不可删除。
@_silgen_name("zhunji_remove_provider")
func zhunji_remove_provider(_ id: UnsafePointer<CChar>) -> Int32

/// 设为默认（同步 prefs.activeAsrProvider + 发 `prefs:changed`）。
/// 返回 0 成功，1 未初始化，2 未找到。
@_silgen_name("zhunji_set_default_provider")
func zhunji_set_default_provider(_ id: UnsafePointer<CChar>) -> Int32

/// 测试供应商连通性（异步 GET `{url}/v1/models` + Bearer，10s 超时），
/// 结果经 `provider:test-result` 事件回调。返回 0 已发起，1 未初始化，2 未找到。
@_silgen_name("zhunji_test_provider")
func zhunji_test_provider(_ id: UnsafePointer<CChar>) -> Int32

// MARK: - 词典页（P2）

/// 热词列表 JSON 数组 `["词1","词2"]`（同上需释放）。
@_silgen_name("zhunji_list_terms")
func zhunji_list_terms() -> UnsafeMutablePointer<CChar>?

/// 新增热词。返回 `{"ok":true}` 或 `{"error":"..."}`（同上需释放）。
@_silgen_name("zhunji_add_term")
func zhunji_add_term(_ term: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

/// 删除热词。返回 0 成功，1 失败。
@_silgen_name("zhunji_remove_term")
func zhunji_remove_term(_ term: UnsafePointer<CChar>) -> Int32

// MARK: - 高级页（P2：调试工具 / 导出错误日志）

/// 导出错误日志：复制 `~/Library/Logs/Zhunji/zhunji.log` 到目标路径
/// （NSSavePanel 由 Swift 侧负责）。返回 `{"ok":true}` 或 `{"error":"..."}`（同上需释放）。
@_silgen_name("zhunji_export_error_log")
func zhunji_export_error_log(_ targetPath: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

// MARK: - 辅助

/// 调用 core 取 JSON 字符串并自动释放（core 返回 null → nil）。
func coreJsonString(_ call: () -> UnsafeMutablePointer<CChar>?) -> String? {
    guard let ptr = call() else { return nil }
    defer { zhunji_free_string(ptr) }
    return String(cString: ptr)
}
