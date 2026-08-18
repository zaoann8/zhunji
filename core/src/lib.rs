//! Zhunji 核心 — 从 zhunlu/src-tauri 剥离 tauri 依赖的纯逻辑层。
//!
//! 对外唯一接口是 ffi.rs（C ABI），SwiftUI app 经它调用本库；
//! 事件经 event_bus.rs 以 `extern "C"` 回调推给 Swift。

mod asr;
mod audio_mute;
mod combo_hotkey;
mod coordinator;
mod coordinator_state;
mod device_watch;
mod dictionary;
mod event_bus;
mod ffi;
mod global_hotkey_runtime;
mod hotkey;
mod insertion;
mod logging;
mod memory;
mod net;
mod permissions;
mod persistence;
mod providers;
mod recorder;
mod shortcut_binding;
mod side_aware_combo;
mod types;
mod unicode_keystroke;

/// 全局 tokio runtime——替代 tauri::async_runtime 的全局 runtime。任意线程调用
/// `block_on()` 都可用（tauri 原版对非 runtime 线程 fallback 到全局 runtime，这里
/// 直接暴露全局 runtime 实现等价语义）。录音电平等 CPU 密集短任务不占用 runtime
/// 线程：`block_on` 只在 hotkey bridge 线程内执行有网络/等待的 async 调用。
pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("zhunji-core")
            .build()
            .expect("failed to build zhunji-core tokio runtime")
    })
}

pub(crate) fn block_on<F: std::future::Future>(f: F) -> F::Output {
    runtime().block_on(f)
}
