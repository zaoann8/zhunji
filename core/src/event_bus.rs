//! 事件总线 — 原 tauri `app.emit/emit_to` 的替代。
//!
//! core 内部任意线程调用 `emit*`，经全局注册的 `extern "C"` 回调把
//! `{"event":"<name>","payload":<json>}` 推给 Swift 宿主；Swift 在
//! EventSink 里解析并派发到主线程。回调要求极短（只做分发），
//! 序列化在这里完成，宿主只做字符串拷贝。

use std::ffi::{CStr, CString};
use std::sync::OnceLock;

/// Swift 注册的回调：入参是 NUL 结尾的 JSON 字节串（`{"event":...,"payload":...}`）。
pub type EventCallback = unsafe extern "C" fn(*const std::os::raw::c_char);

static CALLBACK: OnceLock<EventCallback> = OnceLock::new();

/// ffi::register_events 调用，仅一次；重复注册返回 false。
pub fn set_callback(cb: EventCallback) -> bool {
    CALLBACK.set(cb).is_ok()
}

/// 发送事件（payload 为已序列化的 JSON 文本；不会失败，未注册回调时静默丢弃）。
pub fn emit_raw(event: &str, payload_json: &str) {
    let Some(cb) = CALLBACK.get() else {
        return;
    };
    let Ok(msg) = CString::new(format!("{{\"event\":\"{event}\",\"payload\":{payload_json}}}"))
    else {
        log::warn!("[event_bus] event {event} 含 NUL 字节，丢弃");
        return;
    };
    unsafe { cb(msg.as_ptr()) };
}

/// 发送带结构体 payload 的事件（序列化失败时 payload 为 null）。
pub fn emit<T: serde::Serialize>(event: &str, payload: &T) {
    let json = serde_json::to_string(payload).unwrap_or_else(|e| {
        log::warn!("[event_bus] event {event} 序列化失败: {e}");
        "null".into()
    });
    emit_raw(event, &json);
}

/// 发送无 payload 的事件。
pub fn emit_unit(event: &str) {
    emit_raw(event, "null");
}

/// 供宿主（Swift）反向查询回调是否就绪（调试用）。
#[allow(dead_code)] // P0 冒烟调试：检查回调是否已注册
pub fn callback_registered() -> bool {
    CALLBACK.get().is_some()
}

/// 调试工具：把宿主回调收到的原始 JSON 打回日志（FFI 冒烟测试用）。
#[allow(dead_code)] // P0 冒烟调试：打印原始事件
pub fn log_raw_incoming(json: &CStr) {
    log::info!("[event_bus] incoming: {}", json.to_string_lossy());
}
