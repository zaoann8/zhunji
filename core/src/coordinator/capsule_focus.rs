//! Focus-target capture and capsule-window presentation extracted from
//! `coordinator.rs` (behavior-preserving move).
//!
//! 从 zhunlu 迁移（2026-08-13 重写）：窗口 show/hide/position 不再操作
//! tauri WebviewWindow —— 全部事件化，由 Swift 宿主用 NSPanel 执行。
//! AX 焦点捕获（capture_focus_target / capture_frontmost_app /
//! restore_focus_target_if_possible）与 emit_capsule 状态机逻辑保留在 core。
//!
//! macOS NSPanel 移植参考（原 show_capsule_window_no_activate 的实现要点，P1 Swift 侧用）：
//! - 非激活展示：orderFrontRegardless（不能 show()/makeKey，否则 AeroSpace 切 workspace）
//! - setLevel(25)：抬到菜单栏(24)之上，叠在全屏 app 之上
//! - collectionBehavior = CAN_JOIN_ALL_SPACES(1<<0) | STATIONARY(1<<4) |
//!   FULL_SCREEN_AUXILIARY(1<<8) = 273；入场帧先以低值(STATIONARY|FULL_SCREEN_AUXILIARY)
//!   上屏，orderFront 之后的下一个 runloop tick 再写 273（0→1 转变才触发 WindowServer
//!   重新注册贴附；同 tick 连写会被合并成 no-op）
//! - 鼠标穿透：可交互状态（classic 样式 Recording/Transcribing/Polishing）关穿透，
//!   其余保持穿透

use super::*;

#[cfg(target_os = "windows")]
pub(super) fn capture_focus_target() -> Option<usize> {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    let foreground = unsafe { GetForegroundWindow() };
    if foreground.0.is_null() {
        None
    } else {
        Some(foreground.0 as usize)
    }
}

#[cfg(not(target_os = "windows"))]
pub(super) fn capture_focus_target() -> Option<usize> {
    None
}

/// 捕获用户开始 dictation 时的前台 app 标签（"localizedName (bundle.id)"），用作 LLM
/// polish/translate 的上下文前提，让模型按 app 调风格。详见 issue #116。
///
/// macOS 走 NSWorkspace.frontmostApplication（公开 API，无需额外权限）；
/// Windows 复用前台 HWND 拿窗口标题；Linux/其他平台返回 None。
#[cfg(target_os = "macos")]
pub(super) fn capture_frontmost_app() -> Option<String> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    unsafe {
        let cls = AnyClass::get("NSWorkspace")?;
        let workspace: *mut AnyObject = msg_send![cls, sharedWorkspace];
        if workspace.is_null() {
            return None;
        }
        let app: *mut AnyObject = msg_send![workspace, frontmostApplication];
        if app.is_null() {
            return None;
        }
        let name_obj: *mut AnyObject = msg_send![app, localizedName];
        let bundle_obj: *mut AnyObject = msg_send![app, bundleIdentifier];
        let name = nsstring_to_string(name_obj);
        let bundle = nsstring_to_string(bundle_obj);
        match (name, bundle) {
            (Some(n), Some(b)) => Some(format!("{n} ({b})")),
            (Some(n), None) => Some(n),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }
}

#[cfg(target_os = "macos")]
unsafe fn nsstring_to_string(ns_string: *mut objc2::runtime::AnyObject) -> Option<String> {
    use objc2::msg_send;
    if ns_string.is_null() {
        return None;
    }
    let utf8: *const std::os::raw::c_char = unsafe { msg_send![ns_string, UTF8String] };
    if utf8.is_null() {
        return None;
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(utf8) };
    let s = cstr.to_string_lossy().into_owned();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(target_os = "windows")]
pub(super) fn capture_frontmost_app() -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    };

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return None;
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let copied = GetWindowTextW(hwnd, &mut buf);
        if copied <= 0 {
            return None;
        }
        let title = String::from_utf16_lossy(&buf[..copied as usize]);
        if title.is_empty() {
            None
        } else {
            Some(title)
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(super) fn capture_frontmost_app() -> Option<String> {
    None
}

#[cfg(target_os = "windows")]
pub(super) fn restore_focus_target_if_possible(target: Option<usize>) -> bool {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, IsIconic, IsWindow, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    let Some(raw_target) = target else {
        log::warn!("[coord] no original Windows insertion target captured");
        return false;
    };
    let hwnd = HWND(raw_target as *mut c_void);
    if hwnd.0.is_null() {
        return false;
    }
    if !unsafe { IsWindow(hwnd).as_bool() } {
        log::warn!("[coord] original Windows insertion target is no longer a valid window");
        return false;
    }

    let foreground = unsafe { GetForegroundWindow() };
    if foreground == hwnd {
        return true;
    }

    if unsafe { IsIconic(hwnd).as_bool() } {
        let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) };
    }
    let _ = unsafe { SetForegroundWindow(hwnd) };
    std::thread::sleep(std::time::Duration::from_millis(60));

    let foreground = unsafe { GetForegroundWindow() };
    if foreground != hwnd {
        log::warn!("[coord] failed to restore original Windows insertion target before paste");
        return false;
    }
    true
}

#[cfg(not(target_os = "windows"))]
pub(super) fn restore_focus_target_if_possible(_target: Option<usize>) -> bool {
    true
}

/// Esc 独占判定：胶囊显示「进行中」（录音/转写/润色）且确为 dictation 会话（phase 非
/// Idle）时为 true——tap/hook 吞掉 Esc 不透传宿主应用。phase 条件专门排除 QA：QA 也走
/// 胶囊，但它的 Esc 由聚焦浮窗处理（#161），全局吞键反而会把它挡掉。纯函数便于表格测试。
fn esc_exclusive_for_capsule(state: CapsuleState, phase: SessionPhase) -> bool {
    matches!(
        state,
        CapsuleState::Recording | CapsuleState::Transcribing | CapsuleState::Polishing
    ) && !matches!(phase, SessionPhase::Idle)
}

/// capsule:show 事件载荷。`is_entry_frame` 为 true 表示「隐藏→可见」入场帧：
/// Swift 应先显示 NSPanel 再渲染入场动画（原版 defer_capsule_emit 语义）。
#[derive(Clone, Debug, serde::Serialize)]
pub(super) struct CapsuleShowEvent<'a> {
    pub(super) is_entry_frame: bool,
    pub(super) payload: &'a CapsulePayload,
}

pub(super) fn emit_capsule(
    inner: &Arc<Inner>,
    state: CapsuleState,
    level: f32,
    elapsed_ms: u64,
    message: Option<String>,
    inserted_chars: Option<u32>,
) -> u64 {
    let _event_guard = inner.capsule_event_lock.lock();
    emit_capsule_with_context_locked(inner, state, level, elapsed_ms, message, inserted_chars)
}

/// `capsule_event_lock` 已由调用方持有的内部实现。自动隐藏路径必须能在验证 epoch
/// 后、发出 Idle 前一直持锁，才能保证旧 timer 不会盖掉刚到的新 payload。
fn emit_capsule_with_context_locked(
    inner: &Arc<Inner>,
    state: CapsuleState,
    level: f32,
    elapsed_ms: u64,
    message: Option<String>,
    inserted_chars: Option<u32>,
) -> u64 {
    // 每次 payload 都推进代数。这样一个终态的旧 timer 在之后出现任何新的
    // session 状态时都失效，不会把新的可见状态强行收回 Idle。
    let event_epoch = inner
        .capsule_event_epoch
        .fetch_add(1, Ordering::SeqCst)
        .wrapping_add(1);
    // 记录上一帧 state，用于判断本次是不是「入场帧」（见下方 defer_capsule_emit）。
    let prev_state = inner.last_capsule_state.lock().replace(state);
    // Esc 独占窗口：胶囊显示进行中（录音/转写/润色）且确为 dictation 会话（phase 非
    // Idle）时，tap/hook 吞掉 Esc 不透传宿主应用——此刻 Esc 的语义是「取消这个会话」，
    // 双重派发会顺带触发宿主应用的 Esc。终止帧（Done/Cancelled/Error/Idle）自然清除。
    // emit_capsule 是所有会话状态变化的单一出口，在此维护不会漏路径。
    let esc_exclusive = esc_exclusive_for_capsule(state, inner.state.lock().phase);
    crate::hotkey::set_esc_exclusive(esc_exclusive);
    let translation = inner.translation_modifier_seen.load(Ordering::SeqCst);
    // 预备态只对 Recording 有意义：麦克风还没吐第一帧 PCM 时（capsule_warming=true）把
    // warming 打成 true，前端渲染「待命」光效；level_handler 首触发后翻 false → 光条点亮。
    let warming =
        matches!(state, CapsuleState::Recording) && inner.capsule_warming.load(Ordering::SeqCst);
    let payload = CapsulePayload {
        state,
        level,
        elapsed_ms,
        message,
        inserted_chars,
        translation,
        warming,
        // 用户选择的胶囊样式。原版由主线程闭包每帧从 prefs 同步到原子缓存（避免音频
        // 线程碰偏好锁）；P0 直接读 prefs（锁开销极小），P1 若需优化再恢复原子缓存。
        capsule_style: inner.prefs.get().capsule_style,
    };

    let visible = !matches!(state, CapsuleState::Idle);
    let show_capsule = inner.prefs.get().show_capsule;
    // 入场帧：胶囊从不可见第一次变可见。事件里带 is_entry_frame，Swift 侧先 show 再
    // 起播入场动画（原版 defer_capsule_emit 语义：窗口 show 之后再 emit）。
    let was_visible = matches!(prev_state, Some(s) if !matches!(s, CapsuleState::Idle));
    let is_entry_frame = visible && !was_visible;

    if show_capsule && visible {
        if is_entry_frame {
            log::info!(
                "[capsule] first show this session: state={}",
                capsule_state_log_name(state)
            );
        }
        crate::event_bus::emit(
            "capsule:show",
            &CapsuleShowEvent {
                is_entry_frame,
                payload: &payload,
            },
        );
    } else if visible {
        // show_capsule 开关被用户关掉但本次确实想显示（visible=true）：一次性 info log。
        log::info!(
            "[capsule] suppressed by user toggle: show_capsule=false visible=true state={}",
            capsule_state_log_name(state)
        );
        crate::event_bus::emit_unit("capsule:hide");
    }
    // 状态事件：Swift 的 CapsulePanel 渲染 + AudioCue 提示音共用。
    crate::event_bus::emit("capsule:state", &payload);
    event_epoch
}

/// 旧 dictation timer 的收起路径。它与所有 emit 共享一把短锁：如果新语音先一步
/// 发了状态，也会在锁序上排在 Idle 前。
pub(super) fn hide_capsule_if_all_sessions_idle(inner: &Arc<Inner>) {
    // 先读 session lock，再进 capsule lock。event epoch 负责在两次读取之间
    // 有任何新 payload 时取消本次 Idle。
    let dictation_idle = inner.state.lock().phase == SessionPhase::Idle;
    let observed_epoch = inner.capsule_event_epoch.load(Ordering::SeqCst);
    if !dictation_idle {
        return;
    }

    let _event_guard = inner.capsule_event_lock.lock();
    if inner.capsule_event_epoch.load(Ordering::SeqCst) == observed_epoch {
        emit_capsule_with_context_locked(inner, CapsuleState::Idle, 0.0, 0, None, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CapsuleState;

    #[test]
    fn esc_exclusive_flag_matches_capsule_and_phase() {
        // 进行中胶囊 + dictation phase 非 Idle → 独占 Esc（不透传宿主应用）。
        for (state, phase) in [
            (CapsuleState::Recording, SessionPhase::Listening),
            (CapsuleState::Transcribing, SessionPhase::Processing),
            (CapsuleState::Polishing, SessionPhase::Processing),
            (CapsuleState::Recording, SessionPhase::Inserting),
        ] {
            assert!(
                esc_exclusive_for_capsule(state, phase),
                "{state:?} @ {phase:?} 应独占 Esc"
            );
        }

        // 终止帧（Done/Cancelled/Error/Idle）→ 清除独占。
        for (state, phase) in [
            (CapsuleState::Done, SessionPhase::Idle),
            (CapsuleState::Cancelled, SessionPhase::Idle),
            (CapsuleState::Error, SessionPhase::Idle),
            (CapsuleState::Idle, SessionPhase::Idle),
        ] {
            assert!(
                !esc_exclusive_for_capsule(state, phase),
                "{state:?} @ {phase:?} 不应独占 Esc"
            );
        }

        // QA 场景：胶囊显示进行中但 dictation phase=Idle → 不独占（Esc 归浮窗，#161）。
        for (state, phase) in [
            (CapsuleState::Recording, SessionPhase::Idle),
            (CapsuleState::Transcribing, SessionPhase::Idle),
            (CapsuleState::Polishing, SessionPhase::Idle),
        ] {
            assert!(
                !esc_exclusive_for_capsule(state, phase),
                "{state:?} @ {phase:?}（QA）不应独占 Esc"
            );
        }
    }
}
