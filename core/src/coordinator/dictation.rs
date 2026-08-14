use std::sync::Arc;

use crate::coordinator_state::{
    finish_cancelled_processing_state, request_stop_during_starting_state,
};
use crate::types::HotkeyMode;

use super::resources::*;
use super::*;

/// 同一个 hotkey 边沿之间的最小间隔。低于此阈值的连按整体作为误触丢弃 ——
/// 避免微动开关回弹 / 用户手抖双击造成的空转写报错和 ASR session 抢资源。
const HOTKEY_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);
const MAX_PENDING_COMBO_PRESSES: usize = 64;
/// Auto 模式下区分「短按 = 切换式」与「长按 = 按住说话」的按住时长阈值。
/// 松手时若按住 < 此值判为短按（锁存，保持录音），>= 此值判为长按（松手即停）。
/// 时长以热键事件产生时携带的时间戳计算，避免串行 bridge 的排队延迟改变用户的物理按住时长。
/// 350ms 是「点一下 vs 明显按住」的自然分界。
const AUTO_HOLD_THRESHOLD: std::time::Duration = std::time::Duration::from_millis(350);
/// modifier-only 触发键（Option / 右 Ctrl…）按下后的「组合键仲裁窗口」。
///
/// 按下这一刻还分不清用户是想说话，还是要打 Option+任意字母/数字键：修饰键的按下边沿两者完全一样。
/// 所以先等这么久再开会话——期间监听器若报告叠加了普通键，这次按下整条作废，麦克风
/// 不开、胶囊不闪、也不烧一次 ASR 建连。代价是听写起录晚这么多，取 150ms：足以覆盖
/// 绝大多数组合键的「修饰键→普通键」间隔，又低于人从按键到开口的反应时间（>250ms），
/// 不会吃掉首字。窗口没盖住的慢速组合键（按住 Option 半秒再按 Tab）由组合键撤销
/// 事后撤销兜底，见 handle_trigger_combined。
pub(super) const COMBO_ARBITRATION_GRACE: std::time::Duration =
    std::time::Duration::from_millis(150);
#[cfg(target_os = "windows")]
pub(super) fn windows_sendinput_options_from_prefs(
    prefs: &crate::types::UserPreferences,
) -> crate::unicode_keystroke::WindowsSendInputOptions {
    crate::unicode_keystroke::WindowsSendInputOptions {
        newline_mode: prefs.windows_sendinput_newline_mode,
    }
}

#[cfg(target_os = "windows")]
fn drain_streaming_insert_deltas_with_sendinput_options(
    rx: std::sync::mpsc::Receiver<String>,
    flush_interval: std::time::Duration,
    options: crate::unicode_keystroke::WindowsSendInputOptions,
) -> (String, Option<String>) {
    drain_streaming_insert_deltas_with(rx, flush_interval, move |pending, typed| {
        flush_streaming_insert_buffer_with_options(pending, typed, options)
    })
}

#[cfg(any(target_os = "windows", test))]
fn drain_streaming_insert_deltas_with<F>(
    rx: std::sync::mpsc::Receiver<String>,
    flush_interval: std::time::Duration,
    mut flush_pending: F,
) -> (String, Option<String>)
where
    F: FnMut(&mut String, &mut String) -> Option<String>,
{
    let mut typed_text = String::new();
    let mut first_failure: Option<String> = None;
    let mut pending = String::new();
    while let Ok(delta) = rx.recv() {
        pending.push_str(&delta);
        let flush_at = std::time::Instant::now() + flush_interval;
        loop {
            let now = std::time::Instant::now();
            if now >= flush_at {
                break;
            }
            match rx.recv_timeout(flush_at.duration_since(now)) {
                Ok(delta) => pending.push_str(&delta),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    first_failure = flush_pending(&mut pending, &mut typed_text);
                    return (typed_text, first_failure);
                }
            }
        }
        first_failure = flush_pending(&mut pending, &mut typed_text);
        if first_failure.is_some() {
            // 一旦类型链路出错（如 Secure Input 启用），后续 delta 全部丢弃，但仍
            // 把 mpsc drain 完，避免发送端阻塞。
            while rx.recv().is_ok() {}
            break;
        }
    }
    if first_failure.is_none() {
        first_failure = flush_pending(&mut pending, &mut typed_text);
    }
    (typed_text, first_failure)
}

#[cfg(target_os = "windows")]
fn flush_streaming_insert_buffer_with_options(
    pending: &mut String,
    typed_text: &mut String,
    options: crate::unicode_keystroke::WindowsSendInputOptions,
) -> Option<String> {
    flush_streaming_insert_buffer_with(pending, typed_text, move |text| {
        crate::unicode_keystroke::type_unicode_chunk_with_options(text, options)
    })
}

#[cfg(any(target_os = "windows", test))]
fn flush_streaming_insert_buffer_with<F>(
    pending: &mut String,
    typed_text: &mut String,
    mut type_chunk: F,
) -> Option<String>
where
    F: FnMut(&str) -> Result<usize, crate::unicode_keystroke::TypeError>,
{
    if pending.is_empty() {
        return None;
    }
    let delta = std::mem::take(pending);
    let delta_chars = delta.chars().count();
    match type_chunk(&delta) {
        Ok(typed_chars) => {
            let appended = append_typed_prefix(typed_text, &delta, typed_chars);
            if appended < delta_chars {
                let reason = format!(
                    "type_unicode_chunk typed only {appended}/{delta_chars} chars without error"
                );
                log::error!(
                    "[coord] streaming_insert: {reason} at typed={} chars; \
                     dropping remaining deltas",
                    typed_text.chars().count()
                );
                Some(reason)
            } else {
                None
            }
        }
        Err(e) => {
            append_typed_prefix(typed_text, &delta, e.typed_chars());
            log::error!(
                "[coord] streaming_insert: type_unicode_chunk failed at typed={} chars: {e}; \
                 dropping remaining deltas",
                typed_text.chars().count()
            );
            Some(e.to_string())
        }
    }
}

fn finalize_polished_text(
    polished: String,
    translation_active: bool,
    _raw_uses_llm: bool,
    _mode: PolishMode,
    polish_error: &Option<String>,
    chinese_script_preference: crate::types::ChineseScriptPreference,
    already_streamed: bool,
) -> String {
    if already_streamed {
        return polished;
    }
    let should_force_script = if translation_active {
        // 翻译路径目标可能是非中文（英/日/韩），OpenCC 会破坏它，故只在 polish 失败、
        // 回退到中文原文时才做字形转换。
        polish_error.is_some()
    } else {
        // 普通听写：始终按用户所选字形（简/繁）做确定性 OpenCC 转换。Auto 时
        // apply_chinese_script_preference 内部是 no-op，对默认用户零影响。
        // 不再只在 Raw / polish 失败时转——polish 模式靠 LLM 提示输出繁体并不可靠
        // （模型默认简体），导致繁中用户每次都拿到简体输出（issue #643）。
        true
    };
    if should_force_script {
        apply_chinese_script_preference(&polished, chinese_script_preference)
    } else {
        polished
    }
}

fn default_done_message(status: InsertStatus, polish_failed: bool) -> Option<String> {
    if polish_failed {
        // polish 失败优先告知用户，即使 insert 成功也要让用户知道这版是原文
        Some("润色失败，已插入原文".to_string())
    } else {
        match status {
            InsertStatus::Inserted => None,
            InsertStatus::PasteSent => Some("已尝试粘贴".to_string()),
            InsertStatus::CopiedFallback => Some(if cfg!(target_os = "windows") {
                "已复制，请 Ctrl+V".to_string()
            } else {
                "已复制，请粘贴".to_string()
            }),
            InsertStatus::Failed => Some("插入失败".to_string()),
        }
    }
}

pub(super) async fn handle_pressed_edge(
    inner: &Arc<Inner>,
    pressed_at: std::time::Instant,
    press_id: u64,
) {
    let was_held = inner.hotkey_trigger_held.swap(true, Ordering::SeqCst);
    if !was_held {
        // 先切换代次并清掉上一轮的会话标记，再做防抖。被防抖丢弃的按下也必须
        // 让后续组合键撤销事件归属于自己，不能继承上一轮的 true。
        inner
            .hotkey_press_generation
            .store(press_id, Ordering::SeqCst);
        inner.hotkey_press_began_session.store(0, Ordering::SeqCst);

        // 防抖：相邻 < HOTKEY_DEBOUNCE 的边沿直接丢弃，记到 log 方便排查。
        // 与 `hotkey_trigger_held` 互补：held 防 press-without-release，本检查防
        // press-release-press 三连过快。每个有效边沿都会更新时间戳。
        let now = std::time::Instant::now();
        let too_soon = {
            let mut last = inner.last_hotkey_dispatch_at.lock();
            let drop = matches!(*last, Some(t) if now.duration_since(t) < HOTKEY_DEBOUNCE);
            if !drop {
                *last = Some(now);
            }
            drop
        };
        if too_soon {
            log::info!(
                "[coord] hotkey pressed edge debounced (< {} ms since last dispatch)",
                HOTKEY_DEBOUNCE.as_millis()
            );
            return;
        }

        handle_pressed(inner, pressed_at, press_id).await;
    }
}

pub(super) async fn handle_pressed(
    inner: &Arc<Inner>,
    pressed_at: std::time::Instant,
    press_id: u64,
) {
    let mode = inner.prefs.get().hotkey.mode;
    let phase = inner.state.lock().phase;
    log::info!("[coord] hotkey pressed (mode={mode:?}, phase={phase:?})");
    match (mode, phase) {
        (HotkeyMode::Toggle, SessionPhase::Idle) => {
            // 冷却检查：end_session / 取消收尾后禁止短时间内再次激活，避免三连按第 3 次误触
            // （此时胶囊仍在离场动画周期内，issue #545）。识别中按下想录下一条的 Pressed 会被
            // 缓在 hotkey channel 里、会话收尾后（距 Idle 落在冷却期内）才取出 —— 一律静默
            // 丢弃，不再放行开录（issue #856：无反馈排队 + 延迟开录的惊吓成本大于收益）。
            let now = std::time::Instant::now();
            let on_cooldown = inner
                .session_cooldown_until
                .lock()
                .map(|deadline| now < deadline)
                .unwrap_or(false);
            if on_cooldown {
                log::info!(
                    "[coord] toggle activation blocked by cooldown (session still winding down)"
                );
                return;
            }
            begin_session_from_press(inner, press_id).await;
        }
        (HotkeyMode::Toggle, SessionPhase::Listening) => {
            let _ = end_session(inner).await;
        }
        (HotkeyMode::Hold, SessionPhase::Idle) => {
            begin_session_from_press(inner, press_id).await;
        }
        // Toggle 模式 Starting 阶段第二次按 → 用户想停。
        // 不能直接 end_session（ASR session 还没建好），存边沿，握手完成后立即触发。
        (HotkeyMode::Toggle, SessionPhase::Starting) => {
            request_stop_during_starting(inner, "toggle stop edge");
        }
        // Auto 模式：按下即开录（与 Hold 一样不丢首字）。是短按还是长按要到松手时才知道，
        // 所以这里只负责「开始」并记下按下时刻，语义交给 handle_released 判定。
        (HotkeyMode::Auto, SessionPhase::Idle) => {
            // 复用 Toggle 的冷却检查：#545 离场动画期间误触保护；识别中排队的按下同样丢弃（#856）。
            let now = std::time::Instant::now();
            let on_cooldown = inner
                .session_cooldown_until
                .lock()
                .map(|deadline| now < deadline)
                .unwrap_or(false);
            if on_cooldown {
                log::info!(
                    "[coord] auto activation blocked by cooldown (session still winding down)"
                );
                return;
            }
            *inner.hotkey_press_at.lock() = Some(pressed_at);
            begin_session_from_press(inner, press_id).await;
        }
        // Auto 模式已因上一次「短按」锁存为切换态，再次按下 → 用户想停。
        (HotkeyMode::Auto, SessionPhase::Listening) => {
            let _ = end_session(inner).await;
        }
        // Auto 模式锁存后仍在 Starting 时第二次按 → 想停，同 Toggle 存边沿。
        (HotkeyMode::Auto, SessionPhase::Starting) => {
            request_stop_during_starting(inner, "auto stop edge");
        }
        _ => {}
    }
}

/// 由「这一次热键按下」开一条会话，并记下这个事实。组合键撤销只撤销带着这个
/// 标记的会话（见 handle_trigger_combined）。
///
/// 开录之前先过一遍组合键仲裁窗口：命中就当这次按下没发生过——不开麦、不弹胶囊。
async fn begin_session_from_press(inner: &Arc<Inner>, press_id: u64) {
    if press_resolves_to_combo(inner, press_id).await {
        // 按住态一并清掉：随后必然到来的 Released 会被 handle_released_edge 的
        // was_held 检查吞掉，不会走 Auto 短按锁存。
        inner.hotkey_trigger_held.store(false, Ordering::SeqCst);
        *inner.hotkey_press_at.lock() = None;
        *inner.last_hotkey_dispatch_at.lock() = None;
        return;
    }
    inner
        .hotkey_press_began_session
        .store(press_id, Ordering::SeqCst);
    // 组合键事件可能刚好在仲裁窗口结束、但在上面的标记写入前抵达；再检查一次，
    // 避免这种窄竞态把已判定为组合键的按下开成会话。
    if combo_seen_for_press(inner, press_id) {
        inner
            .hotkey_press_began_session
            .compare_exchange(press_id, 0, Ordering::SeqCst, Ordering::SeqCst)
            .ok();
        inner.hotkey_trigger_held.store(false, Ordering::SeqCst);
        *inner.hotkey_press_at.lock() = None;
        *inner.last_hotkey_dispatch_at.lock() = None;
        return;
    }
    let _ = begin_session(inner).await;
    // 组合键撤销走独立通道，可能恰好在上面的仲裁检查之后、会话启动之前抵达。
    // 这种情况下撤销线程会留下 pending 标记，但在 phase=Idle 时无法取消；启动完成后
    // 必须再消费一次，否则这次组合键会把会话误启动出来。
    if inner.hotkey_press_generation.load(Ordering::SeqCst) == press_id
        && combo_seen_for_press(inner, press_id)
    {
        inner.hotkey_trigger_held.store(false, Ordering::SeqCst);
        *inner.hotkey_press_at.lock() = None;
        *inner.last_hotkey_dispatch_at.lock() = None;
        inner
            .hotkey_press_began_session
            .compare_exchange(press_id, 0, Ordering::SeqCst, Ordering::SeqCst)
            .ok();
        cancel_combined_session_if_active(inner);
        return;
    }
    if inner.hotkey_press_generation.load(Ordering::SeqCst) == press_id
        && inner.state.lock().phase == SessionPhase::Idle
    {
        inner
            .hotkey_press_began_session
            .compare_exchange(press_id, 0, Ordering::SeqCst, Ordering::SeqCst)
            .ok();
    }
}

/// 组合键仲裁：等 COMBO_ARBITRATION_GRACE，再问监听器这次按住有没有叠加普通键。
///
/// 只对 modifier-only 触发键等待 —— 自定义组合键（Cmd+Shift+D 之类）本身就没有歧义，
/// 让它白等这一下纯粹是掉延迟。等待放在防抖 / 冷却判定之后，那些判定用的仍是未被本
/// 窗口推迟的时刻。
async fn press_resolves_to_combo(inner: &Arc<Inner>, press_id: u64) -> bool {
    let binding = inner.prefs.get().dictation_hotkey;
    if crate::shortcut_binding::legacy_modifier_trigger(&binding).is_none() {
        return false;
    }
    tokio::time::sleep(COMBO_ARBITRATION_GRACE).await;
    let combined = combo_seen_for_press(inner, press_id);
    if combined {
        log::info!(
            "[coord] 触发键在 {}ms 仲裁窗口内叠加了其他键 —— 本次按下作废，不开录音",
            COMBO_ARBITRATION_GRACE.as_millis()
        );
    }
    combined
}

/// 触发键（modifier-only 热键）按住期间又按了普通键 —— 用户在打 Option+任意字母/数字键这类组合键，
/// 不是想说话。撤销这次按下：
///
/// 1. 清掉按住态。后面必然到来的 Released 会被 handle_released_edge 的 `was_held`
///    检查吞掉，不会再走 Hold 松手结束 / Auto 短按锁存那套判定 —— 否则 Auto 模式下
///    「Option+组合键快速松手」正是被判成短按锁存，录音一直开着停不下来。
/// 2. 只有这次按下真的开出了会话才取消它。按下时是 toggle 停止 / 被冷却拦下 /
///    路由给 QA 的，什么都不动（尤其不能取消正在转写的上一条）。
///
/// 组合键误触不算「刚用完一次听写」，所以顺带清掉冷却与防抖时间戳：否则紧接着那次
/// 真想说话的按下会被 #545 冷却 / 250ms 防抖静默吞掉，用户以为热键坏了。
///
/// 本函数跑在 `combo_abort_bridge_loop` 的独立线程上，与 Pressed/Released 那条串行
/// bridge 并发 —— 这正是它能在按下 Q 的那一帧就撤掉胶囊的原因，但也意味着不能再假定
/// 「Released 一定排在自己后面」。所以撤不撤销只看 `hotkey_press_began_session`
/// （每个 Pressed 边沿都会重置它，见 handle_pressed_edge），不看 `hotkey_trigger_held`：
/// 万一 Released 抢先跑完把按住态清了，撤销仍然认得出这条会话是自己那次按下开的。
/// 清 `hotkey_trigger_held` 只为吞掉后面的 Released，与撤销与否无关。
///
/// 另一个并发面是撤销落在 `begin_session` 还在 await 的中途 —— 由 begin_session 里
/// 既有的 `startup_race_status_for_starting` / `CancelRaced` 检查点接住（audit HIGH #1），
/// 与 Esc 取消同一条路径。
fn combo_seen_for_press(inner: &Arc<Inner>, press_id: u64) -> bool {
    // 自定义组合键和窗口回退路径没有 modifier-only 监听器，使用 0 表示没有代次。
    // pending 的初始值也是 0，不能让 compare_exchange(0, 0) 把每次自定义组合键误判为
    // 已发生组合撤销。
    if press_id == 0 {
        return false;
    }
    let pending = {
        let mut pending_presses = inner.hotkey_combo_pending_presses.lock();
        pending_presses
            .iter()
            .position(|pending_press| *pending_press == press_id)
            .and_then(|index| pending_presses.remove(index))
            .is_some()
    };
    let monitor_seen = inner
        .hotkey
        .lock()
        .as_ref()
        .is_some_and(|monitor| monitor.trigger_combined_since_press(press_id));
    pending || monitor_seen
}

pub(super) fn handle_trigger_combined(inner: &Arc<Inner>, press_id: u64) {
    if press_id == 0 {
        return;
    }
    // 先记下代次：combo 事件可能早于 Pressed 事件被协调器线程取出，仲裁窗口会
    // 在稍后消费这个待处理标记。若当前已进入下一代，则只记录旧事件，不能清掉
    // 新按下的 held 状态。
    {
        let mut pending_presses = inner.hotkey_combo_pending_presses.lock();
        if !pending_presses.contains(&press_id) {
            pending_presses.push_back(press_id);
            if pending_presses.len() > MAX_PENDING_COMBO_PRESSES {
                pending_presses.pop_front();
            }
        }
    }
    if inner.hotkey_press_generation.load(Ordering::SeqCst) != press_id {
        log::debug!("[coord] ignore stale combined hotkey press_id={press_id}");
        return;
    }
    // 长时说话保护：录音中忽略组合键（不重置 held，松手仍能正常结束会话）。
    if inner.state.lock().phase == SessionPhase::Listening {
        log::info!("[coord] 录音中忽略组合键取消（长时说话保护）");
        return;
    }
    inner.hotkey_trigger_held.store(false, Ordering::SeqCst);
    *inner.hotkey_press_at.lock() = None;
    let began_session = inner
        .hotkey_press_began_session
        .compare_exchange(press_id, 0, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok();
    if !began_session {
        log::info!("[coord] hotkey combined with another key (本次按下没开出会话，无需撤销)");
        return;
    }
    log::info!("[coord] hotkey combined with another key —— 取消本次按下开出的会话");
    cancel_combined_session_if_active(inner);
}

/// 只取消仍处于可取消阶段的本次会话。
///
/// 组合键通道独立于 Pressed/Released，事件可能在正常松手收尾、phase 已回到 Idle 后才被
/// 消费。此时不能清掉正常会话留下的冷却和防抖时间戳，否则会重新打开 #545 的三连按窗口。
/// 若会话尚未进入可取消阶段，pending 标记由 `begin_session_from_press` 的收尾检查消费，
/// 防止「撤销先到、开录后到」的竞态。
fn cancel_combined_session_if_active(inner: &Arc<Inner>) {
    if !cancel_session(inner) {
        return;
    }
    *inner.session_cooldown_until.lock() = None;
    *inner.last_hotkey_dispatch_at.lock() = None;
}

pub(super) async fn handle_released_edge(inner: &Arc<Inner>, released_at: std::time::Instant) {
    let was_held = inner.hotkey_trigger_held.swap(false, Ordering::SeqCst);
    if was_held {
        handle_released(inner, released_at).await;
    }
}

pub(super) async fn handle_released(inner: &Arc<Inner>, released_at: std::time::Instant) {
    let mode = inner.prefs.get().hotkey.mode;
    let phase = inner.state.lock().phase;
    log::info!("[coord] hotkey released (mode={mode:?}, phase={phase:?})");
    if mode == HotkeyMode::Toggle {
        // Toggle 听写松手不做事（点一下停）。Less Computer 走独立专用键监听器。
        return;
    }
    if mode == HotkeyMode::Hold {
        match phase {
            SessionPhase::Listening => {
                let _ = end_session(inner).await;
            }
            // Hold 模式 Starting 阶段松开 → 用户想停。同上：握手完成后再 end。
            SessionPhase::Starting => {
                request_stop_during_starting(inner, "hold release edge");
            }
            _ => {}
        }
    }
    if mode == HotkeyMode::Auto {
        // 使用物理按下/松开的事件时刻，避免 bridge 排队时把处理延迟误算为按住时长。
        let held_long = inner
            .hotkey_press_at
            .lock()
            .take()
            .map(|pressed_at| {
                released_at.saturating_duration_since(pressed_at) >= AUTO_HOLD_THRESHOLD
            })
            .unwrap_or(false);
        match phase {
            // 长按松手 = 按住说话，松手即停；短按 = 切换式，锁存保持录音，下次按下再停。
            SessionPhase::Listening if held_long => {
                let _ = end_session(inner).await;
            }
            // 仍在握手就松手，且判为长按 → 用户按住说话想停，存边沿握手完成后再 end。
            SessionPhase::Starting if held_long => {
                request_stop_during_starting(inner, "auto hold release edge");
            }
            SessionPhase::Listening | SessionPhase::Starting => {
                log::info!("[coord] auto short-tap latched (toggle semantics); next press stops");
            }
            _ => {}
        }
    }
}

pub(super) fn request_stop_during_starting(inner: &Arc<Inner>, reason: &str) {
    {
        let mut state = inner.state.lock();
        if !request_stop_during_starting_state(&mut state) {
            return;
        }
    }
    log::info!("[coord] {reason} during Starting — queued");
    stop_recorder_if_pending_start_stop(inner);
}

pub(super) async fn begin_session(inner: &Arc<Inner>) -> Result<(), String> {
    let current_session_id = {
        let mut state = inner.state.lock();
        let Some(session_id) =
            begin_session_state(&mut state, capture_focus_target(), capture_frontmost_app())
        else {
            return Ok(());
        };
        if let Some(label) = state.front_app.as_deref() {
            log::info!("[coord] front_app captured: {label}");
        }
        session_id
    };
    #[cfg(target_os = "windows")]
    {
        if inner.prefs.get().windows_insertion_mode == crate::types::WindowsInsertionMode::Tsf {
            let prepared = inner.windows_ime.prepare_session();
            let mut slots = inner.prepared_windows_ime_session.lock();
            store_prepared_windows_ime_session(&mut slots, current_session_id, prepared);
        }
    }
    // 翻译模式标志重置；hotkey 监听器在 Shift down 时再 set true。
    inner
        .translation_modifier_seen
        .store(false, Ordering::SeqCst);

    #[cfg(any(debug_assertions, test))]
    if hotkey_injection_dry_run_enabled() {
        emit_capsule(inner, CapsuleState::Recording, 0.0, 0, None, None);
        inner.state.lock().phase = SessionPhase::Listening;
        log::info!("[coord] session started (hotkey-injection dry-run)");
        return Ok(());
    }

    // 乐观显示：按下热键即弹出胶囊并播入场动画，不等麦克风/ASR。此刻麦克风还在 cpal
    // init 窗口内、没有第一帧 PCM，先进「预备态」（warming=true → 前端渲染待命光效，引导
    // 用户稍候再开口）；level_handler 首次触发（PCM 真的流入）后翻成正式录音态、光条点亮。
    // 这样把「视觉反馈」与「麦克风就绪」解耦：即时反馈 + 完整入场动画，同时用预备→点亮的
    // 过渡守住「不漏首字」。若随后凭证/权限校验失败，下面分支会用 Error 覆盖这一帧。
    inner.capsule_warming.store(true, Ordering::SeqCst);
    emit_capsule(inner, CapsuleState::Recording, 0.0, 0, None, None);

    let active_asr = inner.prefs.get().active_asr_provider.clone();

    if let Err(message) = ensure_asr_credentials(&active_asr) {
        log::warn!("[coord] ASR credential gate failed: {message}");
        emit_capsule(
            inner,
            CapsuleState::Error,
            0.0,
            0,
            Some(message.clone()),
            None,
        );
        restore_prepared_windows_ime_session(inner, current_session_id);
        inner.state.lock().phase = SessionPhase::Idle;
        return Err(message);
    }

    if let Err(message) = ensure_microphone_permission(inner) {
        log::warn!("[coord] microphone permission gate failed: {message}");
        emit_capsule(
            inner,
            CapsuleState::Error,
            0.0,
            0,
            Some(message.clone()),
            None,
        );
        restore_prepared_windows_ime_session(inner, current_session_id);
        inner.state.lock().phase = SessionPhase::Idle;
        schedule_capsule_idle(inner, CAPSULE_AUTO_HIDE_DELAY_MS);
        return Err(message);
    }

    // 不在这里 emit Recording capsule —— 让 start_recorder_for_starting 在
    // Recorder::start 成功后再发，确保「用户看到录音条」时 mic 已经在 capture。
    // 之前在这一行就 emit 会让用户看到录音条后立刻开口，但 mic 还在 cpal init
    // 窗口（50-200ms）内 → 开头几个字物理上录不到。详见 issue 备注。
    // 统一百炼:按所选模型把 build 分发重定向到具体协议 id（凭据仍读真实 active
    // `bailian` 的那把 key；endpoint 由前端按模型同步）。别名 id 原样返回,走旧路径。
    // 编译期护栏（exhaustiveness tripwire）：下面这条云端构建 if-else 链最后是
    // `else` 静默落到火山。这个穷尽的空 match 本身不做事，但新增
    // ActiveAsrProviderKind 时会在此编译失败，逼作者回来给新 kind 补一条构建分支
    // ——把「装完才发现漏了」的运行期坑变成编译期错误。QA 侧的 build_qa_asr_start
    // 已是穷尽 match，两条构建路径都受编译器保护。
    // 豆包 IME 免费通道：无凭据、自动注册。复用常驻实例 + DeferredAsrBridge。
    if active_asr == "builtin-doubao" || crate::asr::doubao::is_doubao(&active_asr) {
        let name = find_provider("builtin-doubao")
            .map(|p| p.name)
            .unwrap_or_else(|| "豆包 IME".into());
        let asr = Arc::clone(&inner.doubao);
        let bridge = Arc::new(DeferredAsrBridge::new());
        let consumer: Arc<dyn crate::recorder::AudioConsumer> = bridge.clone();
        store_asr_for_session(
            inner,
            current_session_id,
            ActiveAsr::Doubao(Arc::clone(&asr)),
            AsrCallLabel::new(name, None),
        );
        // 录音起点 = 用户按下热键的时刻（缓冲音频从这里开始）。
        let recorder_start_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        start_recorder_for_starting(inner, current_session_id, &active_asr, consumer).await?;

        // 实时上屏（受「流式输入」开关控制）：先切 ABC 输入源，避免中文 IME 拦截注入。
        if inner.prefs.get().streaming_insert {
            {
                match crate::unicode_keystroke::switch_to_ascii().await {
                    Ok(prev) => {
                        if matches!(
                            startup_race_status_for_starting(inner, current_session_id),
                            StartupRaceStatus::ActiveStarting
                        ) {
                            *inner.live_insert.prev_ime.lock() = prev;
                            inner.live_insert.enabled.store(true, Ordering::SeqCst);
                        } else {
                            let _ = crate::unicode_keystroke::restore_input_source(prev).await;
                        }
                    }
                    Err(e) => {
                        log::warn!("[coord] live insert: switch to ABC failed: {e}");
                        if matches!(
                            startup_race_status_for_starting(inner, current_session_id),
                            StartupRaceStatus::ActiveStarting
                        ) {
                            inner.live_insert.enabled.store(true, Ordering::SeqCst);
                        }
                    }
                }
            }
        }

        if let Err(e) = asr.open_session().await {
            log::error!("[coord] open Doubao ASR session failed: {e}");
            inner.last_engine_ok.store(false, Ordering::SeqCst);
            *inner.last_engine_error.lock() = Some(format!("引擎连接失败: {e}"));
            match startup_race_status_for_starting(inner, current_session_id) {
                StartupRaceStatus::ActiveStarting => {}
                _ => {
                    asr.cancel();
                    discard_startup_resources_for_session(inner, current_session_id);
                    restore_prepared_windows_ime_session(inner, current_session_id);
                    set_phase_idle_if_session_matches(inner, current_session_id);
                    return Ok(());
                }
            }
            discard_startup_resources_for_session(inner, current_session_id);
            emit_capsule(
                inner,
                CapsuleState::Error,
                0.0,
                0,
                Some(format!("ASR 连接失败: {e}")),
                None,
            );
            restore_prepared_windows_ime_session(inner, current_session_id);
            set_phase_idle_if_session_matches(inner, current_session_id);
            schedule_capsule_idle(inner, CAPSULE_AUTO_HIDE_DELAY_MS);
            return Err(e.to_string());
        }
        // 帧时间戳基准校准到录音开始（缓冲音频录制于握手完成前）。
        asr.set_timestamp_base(recorder_start_ms);
        let target: Arc<dyn crate::asr::AudioConsumer> = asr;
        let flushed_bytes = bridge.attach(target);
        log::info!("[coord] Doubao ASR connected; flushed {flushed_bytes} deferred audio bytes");
        finish_starting_session(inner, current_session_id).await;
        return Ok(());
    }

    // 第三方供应商（非豆包）：从 providers.json 取凭据，经 Grok STT 引擎 batch POST。
    if let Some(p) = find_provider(&active_asr) {
        let provider_name = p.name.clone();
        let _ = crate::asr::grok_stt::save_credentials_file(
            &p.url,
            p.api_key.as_deref().unwrap_or(""),
        );
        let asr = Arc::clone(&inner.grok_stt);
        let bridge = Arc::new(DeferredAsrBridge::new());
        let consumer: Arc<dyn crate::recorder::AudioConsumer> = bridge.clone();
        store_asr_for_session(
            inner,
            current_session_id,
            ActiveAsr::GrokStt(Arc::clone(&asr)),
            AsrCallLabel::new(provider_name, None),
        );
        // 录音起点 = 用户按下热键的时刻（缓冲音频从这里开始）。
        let recorder_start_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        start_recorder_for_starting(inner, current_session_id, &active_asr, consumer).await?;

        if let Err(e) = asr.open_session().await {
            log::error!("[coord] open Grok STT session failed: {e}");
            inner.last_engine_ok.store(false, Ordering::SeqCst);
            *inner.last_engine_error.lock() = Some(format!("引擎连接失败: {e}"));
            match startup_race_status_for_starting(inner, current_session_id) {
                StartupRaceStatus::ActiveStarting => {}
                _ => {
                    asr.cancel();
                    discard_startup_resources_for_session(inner, current_session_id);
                    restore_prepared_windows_ime_session(inner, current_session_id);
                    set_phase_idle_if_session_matches(inner, current_session_id);
                    return Ok(());
                }
            }
            discard_startup_resources_for_session(inner, current_session_id);
            emit_capsule(
                inner,
                CapsuleState::Error,
                0.0,
                0,
                Some(format!("ASR 连接失败: {e}")),
                None,
            );
            restore_prepared_windows_ime_session(inner, current_session_id);
            set_phase_idle_if_session_matches(inner, current_session_id);
            schedule_capsule_idle(inner, CAPSULE_AUTO_HIDE_DELAY_MS);
            return Err(e.to_string());
        }
        // 帧时间戳基准校准到录音开始（session_duration_ms 依赖）。
        asr.set_timestamp_base(recorder_start_ms);
        let target: Arc<dyn crate::asr::AudioConsumer> = asr;
        let flushed_bytes = bridge.attach(target);
        log::info!("[coord] Grok STT session ready; flushed {flushed_bytes} deferred audio bytes");
        finish_starting_session(inner, current_session_id).await;
        return Ok(());
    }

    Ok(())
}

pub(super) async fn start_recorder_for_starting(
    inner: &Arc<Inner>,
    session_id: SessionId,
    active_asr: &str,
    consumer: Arc<dyn crate::recorder::AudioConsumer>,
) -> Result<(), String> {
    let inner_for_level = Arc::clone(inner);
    // 节流：电平回调本身约 185 Hz（cpal 默认音频块），全部转发到前端会让 CSS
    // transition 互相覆盖、视觉上"被平均"成静止。限制为 ~30 Hz（33ms 最少间隔），
    // 配合 CSS 短 transition 让每次 emit 完整可见。
    let last_emit_at = Arc::new(Mutex::new(None::<Instant>));
    const LEVEL_EMIT_MIN_INTERVAL_MS: u64 = 33;
    let level_handler: Arc<dyn Fn(f32) + Send + Sync> = Arc::new(move |level| {
        let phase = inner_for_level.state.lock().phase;
        if phase != SessionPhase::Listening && phase != SessionPhase::Starting {
            return;
        }
        let now = Instant::now();
        {
            let mut last = last_emit_at.lock();
            if let Some(prev) = *last {
                if now.duration_since(prev).as_millis() < LEVEL_EMIT_MIN_INTERVAL_MS as u128 {
                    return;
                }
            }
            *last = Some(now);
        }
        let elapsed = inner_for_level
            .state
            .lock()
            .started_at
            .elapsed()
            .as_millis() as u64;
        // 第一帧 PCM 真的流到 consumer 了（recorder.rs::process_callback 的顺序保证
        // consume_pcm_chunk 先于 level_handler）——关掉预备态，让这一帧起 payload.warming
        // 翻 false，前端把「待命」光条点亮成正式录音态。之后每帧都是 false（幂等）。
        inner_for_level
            .capsule_warming
            .store(false, Ordering::SeqCst);
        emit_capsule(
            &inner_for_level,
            CapsuleState::Recording,
            level,
            elapsed,
            None,
            None,
        );
    });

    let microphone_device_name = selected_microphone_device_name(inner);
    stop_microphone_preview_monitor(inner, "dictation recorder");
    acquire_recording_mute(inner, "dictation").await;
    // 总是把这次口述归档成 `recordings/<session_id>.wav`，不再只在 record_audio_for_debug
    // 下归档。原因：失败保留 + 自动重试需要原始音频，而该开关默认 false——之前转录失败时音频
    // 直接丢失（用户反馈「识别失败，之前的语音也都丢失了」）。归档是临时的：拿到非空转写后，
    // 若用户没开 record_audio_for_debug 就立刻删掉（隐私——成功的口述不留痕），只有「转录失败」
    // 的录音会留下，供历史里手动「重新转录」或自动静默重试复用。prune_recordings 兜底总量。
    // 文件名用 coordinator 的 SessionId，跟 history 那条记录 id 对齐（见下游 polish 收尾
    // `history_session_id = current_session_id.to_string()`），前端凭 id 就能找到录音。
    let audio_archive_path = {
        let prefs = inner.prefs.get();
        let _ = crate::persistence::prune_recordings(
            prefs.history_retention_days,
            prefs.audio_recording_max_entries,
        );
        crate::persistence::recording_path_for_session(&session_id.to_string()).ok()
    };
    match Recorder::start(
        microphone_device_name,
        consumer,
        level_handler,
        audio_archive_path,
    ) {
        Ok((rec, runtime_errors, archive_active)) => {
            // 把 archive 实际创建状态存到 Inner，让 history 写入路径（含 empty-transcript
            // 失败分支）读真实情况，而不是 prefs 开关。修 pr_agent "Wrong Flag" 反馈。
            inner
                .audio_archive_active
                .store(archive_active, std::sync::atomic::Ordering::Relaxed);
            store_recorder_for_session(inner, session_id, rec);
            spawn_recorder_error_monitor(inner, runtime_errors);
            // 不在这里 emit Recording capsule。
            // Recorder::start Ok 仅代表 cpal Stream::play 完成，不代表 audio
            // 线程已经在向 consumer 推 PCM —— macOS CoreAudio AudioUnit 启动到
            // 第一帧 process_callback 中间有 50–200 ms 间隙（Windows 类似）。
            // 之前在这里立即 emit Recording 会让用户「看到录音条」就开口，但前几个
            // 字落在 cpal init 窗口里被吞，反映为短录音漏首字（用户报告）。
            //
            // 现改为：level_handler 第一次被触发时才 emit Recording capsule。
            // recorder.rs::process_callback 的顺序是 consume_pcm_chunk → level_handler，
            // 所以 level_handler 第一次执行 == PCM 已经真实流到 consumer。从这一刻
            // 起用户说什么都被录到。capsule 自然就晚 50–200 ms 出现，但出现 ==
            // mic 真的在录，匹配「麦先录、UI 再弹」的预期。
            //
            // 原本的竞态保护交还给两条已有路径：
            //   - stop_recorder_if_pending_start_stop：短按时把 capsule 切到
            //     Transcribing；recorder 已 stop，level_handler 不会再发火。
            //   - level_handler 内部 phase 检查：cancel / 错误使 phase 不在
            //     {Starting, Listening} 时直接 return，不会在错误状态上盖
            //     Recording。
            stop_recorder_if_pending_start_stop(inner);
            log::info!("[coord] recorder started (asr={active_asr}, phase=Starting)");
        }
        Err(e) => {
            log::error!("[coord] recorder start failed: {e}");
            let message = e.user_message();
            cancel_asr_for_session(inner, session_id);
            emit_capsule(
                inner,
                CapsuleState::Error,
                0.0,
                0,
                Some(message.clone()),
                None,
            );
            restore_prepared_windows_ime_session(inner, session_id);
            release_recording_mute(inner, "dictation");
            inner.state.lock().phase = SessionPhase::Idle;
            schedule_capsule_idle(inner, CAPSULE_AUTO_HIDE_DELAY_MS);
            return Err(message);
        }
    }

    Ok(())
}

pub(super) fn spawn_recorder_error_monitor(inner: &Arc<Inner>, rx: mpsc::Receiver<RecorderError>) {
    // 捕获当前 session_id：err 来时若 id 已经不一致说明是上一 session 的迟到事件，
    // 不能去 abort 当前 active 的新 session（它录得好好的）。
    let captured_session_id = inner.state.lock().session_id;
    let inner = Arc::clone(inner);
    std::thread::Builder::new()
        .name("openless-recorder-error-monitor".into())
        .spawn(move || {
            if let Ok(err) = rx.recv() {
                let current_session_id = inner.state.lock().session_id;
                if captured_session_id != current_session_id {
                    log::warn!(
                        "[coord] recorder error from stale session {} dropped (current={}, err={})",
                        captured_session_id,
                        current_session_id,
                        err
                    );
                    return;
                }
                log::error!("[coord] recorder runtime error: {err}");
                abort_recording_with_error(&inner, format!("录音中断: {err}"));
            }
        })
        .ok();
}

pub(super) fn abort_recording_with_error(inner: &Arc<Inner>, message: String) {
    let Some(abort) = ({
        let mut state = inner.state.lock();
        begin_recording_abort_before_restore(&mut state)
    }) else {
        return;
    };

    discard_startup_resources_for_session(inner, abort.session_id);
    restore_prepared_windows_ime_session(inner, abort.session_id);
    {
        let mut state = inner.state.lock();
        publish_abort_idle_after_restore(&mut state, abort.session_id);
    }

    emit_capsule(
        inner,
        CapsuleState::Error,
        0.0,
        abort.elapsed,
        Some(message),
        None,
    );
    schedule_capsule_idle(inner, CAPSULE_AUTO_HIDE_DELAY_MS);
}

pub(super) async fn finish_starting_session(inner: &Arc<Inner>, session_id: SessionId) {
    // audit HIGH #1：转 Listening 之前在同一 lock 内检查 cancel race。
    // 之前是无条件 phase=Listening，会把 cancel_session 在 await 期间设的 Idle
    // 反向覆盖回 Listening → 用户的 cancel 边沿被吞掉。
    let outcome = {
        let mut state = inner.state.lock();
        finish_starting_session_state(&mut state, session_id)
    };
    match outcome {
        BeginOutcome::StaleContinuation => {
            log::info!(
                "[coord] stale recorder/ASR startup continuation from session {session_id} — ignoring"
            );
            discard_startup_resources_for_session(inner, session_id);
            restore_prepared_windows_ime_session(inner, session_id);
        }
        BeginOutcome::CancelRaced => {
            log::info!("[coord] cancel raced during recorder/ASR startup — aborting begin");
            discard_startup_resources_for_session(inner, session_id);
            restore_prepared_windows_ime_session(inner, session_id);
            set_phase_idle_if_session_matches(inner, session_id);
        }
        BeginOutcome::Started | BeginOutcome::PendingStop => {
            log::info!("[coord] session started");
            if matches!(outcome, BeginOutcome::PendingStop) {
                log::info!("[coord] applying pending_stop edge → end_session immediately");
                let _ = end_session(inner).await;
            }
        }
    }
}

/// 转录失败时落一条「转录失败」历史，并保留这次的原始录音，让用户能在历史里看到失败、
/// 手动「重新转录」。复活并修好 issue #613：之前失败的录音被孤立——历史里看不到这条、
/// 音频也找不回（孤儿 wav 最终被 prune 清掉，语音彻底丢失）。
///
/// session_id 与归档 wav 同名（`recordings/<session_id>.wav`），保证 read_audio_recording /
/// retranscribe_recording 凭 id 能定位文件。has_audio_recording 读 Recorder::start 的实际
/// 写盘状态（不是 prefs 开关）：开关想录但路径创建失败时为 false，避免前端渲染播放/重转
/// 按钮而后端 404。
fn build_transcribe_failed_session(
    session_id: SessionId,
    duration_ms: u64,
    asr_ms: u64,
    mode: PolishMode,
    has_audio_recording: bool,
) -> DictationSession {
    DictationSession {
        id: session_id.to_string(),
        created_at: Utc::now().to_rfc3339(),
        source: crate::types::HistorySource::Voice,
        raw_transcript: String::new(),
        final_text: String::new(),
        mode,
        style_pack_id: None,
        translation_active: false,
        polish_source: None,
        app_bundle_id: None,
        app_name: None,
        insert_status: InsertStatus::Failed,
        error_code: Some("transcribeFailed".to_string()),
        duration_ms: Some(duration_ms),
        dictionary_entry_count: None,
        has_audio_recording: Some(has_audio_recording),
        asr_provider: None,
        asr_model: None,
        llm_provider: None,
        llm_model: None,
        asr_ms: Some(asr_ms),
        polish_ms: None,
    }
}

fn write_transcribe_failed_history(
    inner: &Arc<Inner>,
    session_id: SessionId,
    duration_ms: u64,
    asr_ms: u64,
    asr_call_label: Option<&AsrCallLabel>,
) {
    let prefs = inner.prefs.get();
    let mut session = build_transcribe_failed_session(
        session_id,
        duration_ms,
        asr_ms,
        prefs.default_mode,
        inner.audio_archive_active.load(Ordering::Relaxed),
    );
    // 失败条目也记下是哪个 ASR 出的错——「哪个模型转不出来」正是模型对比要看的信息。
    // 用 begin_session 的构建时快照，而不是此刻重读设置（PR #826 review）。
    if let Some(label) = asr_call_label {
        session.asr_provider = Some(label.provider.clone());
        session.asr_model = label.model.clone();
    }
    if let Err(e) = inner.history.append_with_retention(
        session,
        prefs.history_retention_days,
        prefs.history_max_entries,
    ) {
        log::error!("[coord] transcribeFailed history append failed: {e}");
    }
    // 失败条目也进历史，通知前端刷新概览/历史。
    crate::event_bus::emit_unit("history:changed");
}

/// ASR 转录失败 / 超时的统一收尾，替代之前散落在每个引擎分支里重复 5 行的失败尾巴：
/// 保留录音 + 落失败历史 → 错误胶囊 → 恢复窗口/IME → 回 Idle → 定时隐藏胶囊。
/// 永远返回 `Err(err)`，调用方写 `return fail_dictation(...)`。集中一处既保证没有任何引擎
/// 分支漏掉「失败保留」，也是自动静默重试彻底失败后的唯一收尾点。
fn fail_dictation(
    inner: &Arc<Inner>,
    session_id: SessionId,
    elapsed: u64,
    asr_ms: u64,
    user_msg: String,
    err: String,
    asr_call_label: Option<&AsrCallLabel>,
) -> Result<(), String> {
    // 转写失败：清掉实时上屏的临时文本（没有最终结果可替换）。
    inner.last_engine_ok.store(false, Ordering::SeqCst);
    *inner.last_engine_error.lock() = Some(err.clone());
    let temp_len = inner.live_insert.prev_len.swap(0, Ordering::SeqCst);
    if temp_len > 0 {
        let _ = crate::insertion::macos::delete_chars(temp_len);
    }
    write_transcribe_failed_history(inner, session_id, elapsed, asr_ms, asr_call_label);
    emit_capsule(
        inner,
        CapsuleState::Error,
        0.0,
        elapsed,
        Some(user_msg),
        None,
    );
    restore_prepared_windows_ime_session(inner, session_id);
    inner.state.lock().phase = SessionPhase::Idle;
    // 与成功 / 取消收尾一致：回 Idle 即设冷却，把识别中缓存在 hotkey channel 里的 Pressed
    // 一并静默丢弃（issue #856）——否则失败收尾后那条排队按下会立刻开出一条新录音，用户以为
    // 「全部停下了」却再次弹出胶囊；同时覆盖错误胶囊离场动画期间的误触（issue #545）。
    {
        let now = std::time::Instant::now();
        *inner.session_cooldown_until.lock() =
            Some(now + std::time::Duration::from_millis(POST_SESSION_COOLDOWN_MS));
    }
    schedule_capsule_idle(inner, CAPSULE_AUTO_HIDE_DELAY_MS);
    Err(err)
}

/// ASR 失败/超时分支从引擎 match 里产出的「失败」值：带用户提示文案 + 内部错误串，交给
/// match 之后的统一处理（先自动重试，彻底失败再 fail_dictation 收尾）。
struct TranscribeFail {
    user_msg: String,
    err: String,
}

impl TranscribeFail {
    fn new(user_msg: String, err: String) -> Self {
        Self { user_msg, err }
    }
}

/// 自动静默重试的最大次数（不含首次转写）。失败/超时多为网络或服务端瞬时抖动，重试几次
/// 往往就能拿回这段语音；上限避免在永久性故障（如鉴权失败）上空耗太久。
const SILENT_RETRY_MAX: u32 = 2;
/// 每次重试前的线性退避基数：第 N 次重试前等 `SILENT_RETRY_BACKOFF_MS * N` 毫秒，给抖动的
/// 网络/服务端一点缓冲再打。
const SILENT_RETRY_BACKOFF_MS: u64 = 500;

enum SilentRetryOutcome {
    Transcript {
        raw: RawTranscript,
        asr_call_label: AsrCallLabel,
    },
    Exhausted(Option<AsrCallLabel>),
    Cancelled,
}

fn accept_silent_retry_transcript(
    raw: RawTranscript,
    retry_label: AsrCallLabel,
    asr_call_label: &mut Option<AsrCallLabel>,
) -> RawTranscript {
    *asr_call_label = Some(retry_label);
    raw
}

/// 归档 wav 是 16k/mono/16-bit、固定 44 字节标准头；取出 PCM 负载。
/// 长度 <= 44（空/损坏）返回 None。
fn pcm_from_wav_bytes(wav: &[u8]) -> Option<Vec<u8>> {
    if wav.len() <= 44 {
        return None;
    }
    Some(wav[44..].to_vec())
}

/// 16k/mono/16-bit PCM：每毫秒 32 字节（16000 * 2 / 1000）。用 PCM 长度反推时长，给重试成功
/// 后的 RawTranscript.duration_ms（写历史 / 胶囊用）。
fn pcm_duration_ms(pcm_len: usize) -> u64 {
    (pcm_len as u64) / 32
}

/// 用「当前」provider 把一段 PCM 重新转录（建一条全新 ASR 会话——原会话失败/断开后不可
/// 复用）。复用 Coordinator::retranscribe_pcm（历史「重新转录」同款逻辑）；Coordinator 只持有
/// `inner`，这里用 inner 重建一个轻量句柄，零副作用。
async fn retranscribe_pcm_via_inner(
    inner: &Arc<Inner>,
    pcm: Vec<u8>,
) -> (Result<String, String>, Option<AsrCallLabel>) {
    Coordinator {
        inner: Arc::clone(inner),
    }
    .retranscribe_pcm_until_cancelled(pcm)
    .await
}

/// 自动静默重试：从刚归档的 wav 读 PCM，用当前 provider 重转最多 SILENT_RETRY_MAX 次（线性
/// 退避）。任一次拿到非空文本立即返回 Transcript（当作正常转写继续走润色/插入）；没有归档
/// 音频、读不到或全部失败返回 Exhausted（交回 fail_dictation 做「失败保留 + 报错」）。如果
/// 用户在退避或重试请求期间按 Esc，则返回 Cancelled，直接完成取消收尾。全程不改胶囊文案——
/// 对用户静默，只是「转写中」多停留一会儿。
async fn try_silent_retranscribe(inner: &Arc<Inner>, session_id: SessionId) -> SilentRetryOutcome {
    if inner.state.lock().cancelled {
        return SilentRetryOutcome::Cancelled;
    }
    if !inner.audio_archive_active.load(Ordering::Relaxed) {
        return SilentRetryOutcome::Exhausted(None); // 没归档音频，无从重试
    }
    let Some(path) = crate::persistence::recording_path_for_session(&session_id.to_string()).ok()
    else {
        return SilentRetryOutcome::Exhausted(None);
    };
    let wav = tokio::select! {
        biased;
        _ = wait_for_processing_cancel(inner) => return SilentRetryOutcome::Cancelled,
        result = tokio::fs::read(&path) => match result {
            Ok(wav) => wav,
            Err(_) => return SilentRetryOutcome::Exhausted(None),
        },
    };
    let Some(pcm) = pcm_from_wav_bytes(&wav) else {
        return SilentRetryOutcome::Exhausted(None);
    };
    let duration_ms = pcm_duration_ms(pcm.len());
    let mut last_attempted_label = None;
    for attempt in 1..=SILENT_RETRY_MAX {
        tokio::select! {
            biased;
            _ = wait_for_processing_cancel(inner) => return SilentRetryOutcome::Cancelled,
            _ = tokio::time::sleep(std::time::Duration::from_millis(
                SILENT_RETRY_BACKOFF_MS * attempt as u64,
            )) => {}
        }
        let (result, attempted_label) = tokio::select! {
            biased;
            _ = wait_for_processing_cancel(inner) => return SilentRetryOutcome::Cancelled,
            result = retranscribe_pcm_via_inner(inner, pcm.clone()) => result,
        };
        if attempted_label.is_some() {
            last_attempted_label = attempted_label.clone();
        }
        match result {
            Ok(text) if !text.trim().is_empty() => {
                log::info!(
                    "[coord] 自动静默重试第 {attempt}/{SILENT_RETRY_MAX} 次成功（{} 字）",
                    text.chars().count()
                );
                return SilentRetryOutcome::Transcript {
                    raw: RawTranscript { text, duration_ms },
                    asr_call_label: attempted_label
                        .expect("successful retranscription must have a build-time ASR label"),
                };
            }
            Ok(_) => {
                // 重试得到空转写——多半真没说话，再重试无意义，省流量直接放弃。
                log::info!("[coord] 自动静默重试得到空转写，停止重试");
                return SilentRetryOutcome::Exhausted(last_attempted_label);
            }
            Err(e) => {
                log::warn!("[coord] 自动静默重试第 {attempt}/{SILENT_RETRY_MAX} 次失败: {e}");
                if e.contains("40200011") {
                    // 并发配额错误：上限要等已开会话关闭才回落，不在退避窗口内，
                    // 继续重试只会再失败几次、白等退避。直接放弃，录音保留可手动重转。
                    log::warn!("[coord] 并发配额错误，停止自动静默重试");
                    return SilentRetryOutcome::Exhausted(last_attempted_label);
                }
            }
        }
    }
    SilentRetryOutcome::Exhausted(last_attempted_label)
}

fn finish_cancelled_processing(inner: &Arc<Inner>, session_id: SessionId) -> bool {
    let finished = {
        let mut state = inner.state.lock();
        finish_cancelled_processing_state(&mut state, session_id)
    };
    if finished {
        schedule_capsule_idle(inner, CAPSULE_CANCEL_HIDE_DELAY_MS);
    }
    finished
}

pub(super) fn schedule_cancelled_asr_release(
    inner: &Arc<Inner>,
    asr: &ActiveAsr,
    session_id: SessionId,
) {
    let _ = (inner, asr, session_id);
}

/// end_session 转写阶段与「用户取消」赛跑的结果。
enum TranscribeRace {
    Done(Result<RawTranscript, TranscribeFail>),
    /// 用户在 Processing（转写）阶段按 Esc / 取消：drop 掉在途 transcribe future。
    Cancelled,
}

/// 轮询 Processing 阶段的取消标志。用户在转写阶段按 Esc 时，cancel_session 只把
/// `state.cancelled` 置 true —— 此刻 ASR 句柄已被 end_session 从 `inner.asr` 槽 take 走，
/// cancel_session 走的 `cancel_asr_for_session` 是 no-op，够不到在途请求。end_session 用
/// 本函数与在途 transcribe future 赛跑：命中即 drop future，从而中断 reqwest HTTP /
/// 停止等待流式最终结果 / 停止本地转写。
///
/// 用 75ms 轮询而非 notify：转写通常 0.2–3s，几次定时器唤醒的开销可忽略，用户也感知不到
/// 这点延迟；换来的是不依赖任何唤醒信号、没有「取消边沿在注册 waiter 之前触发就丢失」的
/// 竞态，逻辑上更稳。
async fn wait_for_processing_cancel(inner: &Arc<Inner>) {
    loop {
        if inner.state.lock().cancelled {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(75)).await;
    }
}

pub(super) async fn end_session(inner: &Arc<Inner>) -> Result<(), String> {
    let current_session_id = {
        let mut state = inner.state.lock();
        let Some(session_id) = start_processing_if_listening(&mut state) else {
            return Ok(());
        };
        session_id
    };

    let elapsed = inner.state.lock().started_at.elapsed().as_millis() as u64;
    emit_capsule(inner, CapsuleState::Transcribing, 0.0, elapsed, None, None);

    // 松开即停实时上屏（尾音缓冲期间不再注入新 partial 到光标）。
    inner.live_insert.enabled.store(false, Ordering::SeqCst);

    // 尾音缓冲：松开热键后继续录 500ms，避免最后一个字/词被截断。
    // （用户松开时往往还在收尾音节，立即停录会让尾字残缺。）
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    if let Some(rec) = take_recorder_for_session(inner, current_session_id) {
        rec.stop();
        release_recording_mute(inner, "dictation");
    }
    // 恢复用户原输入法（实时上屏切过 ABC）。
    let prev_ime = inner.live_insert.prev_ime.lock().take();
    if let Some(prev) = prev_ime {
        let _ = crate::unicode_keystroke::restore_input_source(Some(prev)).await;
    }

    let asr_opt = take_asr_for_session(inner, current_session_id);
    // 构建时快照（begin_session 存入）。会话中途改设置不影响这份归因。
    let mut asr_call_label = take_asr_label_for_session(inner, current_session_id);
    let asr = match asr_opt {
        Some(a) => a,
        None => {
            restore_prepared_windows_ime_session(inner, current_session_id);
            if !finish_cancelled_processing(inner, current_session_id) {
                set_phase_idle_if_session_matches(inner, current_session_id);
            }
            return Ok(());
        }
    };

    let uses_global_timeout = asr_transcribe_uses_global_timeout(&asr);
    // ASR 句柄内部是 Arc，clone 只是 +1 引用。留一份给取消路径：transcribe future 会把
    // `asr` move 进去，命中取消时那个 future 会被 drop（连同它持有的 Arc），我们再用这份
    // clone 显式 cancel，促使流式 WebSocket 立刻关闭、不残留后台 worker。
    let asr_for_cancel = asr.clone();
    // 「等待转写结果」实测起点：流式 ASR 量的是收尾延迟，批式量完整转写。写进
    // history.asr_ms 供历史详情页展示（含下方的自动静默重试时间——那也是用户等的时间）。
    let transcribe_started = std::time::Instant::now();
    // 每个引擎分支产出 Ok(RawTranscript) 或 Err(TranscribeFail)；失败/超时不再就地 return，
    // 而是把失败值交给 match 之后统一处理：先自动静默重试（从归档音频重转，应对网络/服务端
    // 瞬时抖动），重试拿回文本就当正常转写继续；彻底失败才 fail_dictation 保留录音 + 报错。
    //
    // 整段转写与「用户在 Processing 阶段取消」赛跑：命中取消就直接 drop 掉 transcribe future
    // 中断在途请求，不再傻等它跑完（见 issue「转写中按 Esc 停不下来」）。
    let raced: TranscribeRace = {
        let transcribe_fut = async move {
            let transcribe_outcome: Result<RawTranscript, TranscribeFail> = match asr {
                ActiveAsr::Doubao(asr) => {
                    debug_assert!(uses_global_timeout);
                    if let Err(e) = asr.send_last_frame().await {
                        log::error!("[coord] Doubao send last frame failed: {e}");
                    }
                    let timeout_duration =
                        std::time::Duration::from_secs(COORDINATOR_GLOBAL_TIMEOUT_SECS);
                    match tokio::time::timeout(timeout_duration, asr.await_final_result()).await {
                        Ok(Ok(r)) => Ok(RawTranscript {
                            text: r,
                            duration_ms: asr.session_duration_ms(),
                        }),
                        Ok(Err(e)) => {
                            inner.last_engine_ok.store(false, Ordering::SeqCst);
                            *inner.last_engine_error.lock() = Some(format!("识别失败: {e}"));
                            log::error!("[coord] Doubao await final failed: {e}");
                            asr.cancel();
                            Err(TranscribeFail::new(format!("识别失败: {e}"), e.to_string()))
                        }
                        Err(_) => {
                            inner.last_engine_ok.store(false, Ordering::SeqCst);
                            *inner.last_engine_error.lock() = Some("识别超时".to_string());
                            log::error!(
                                "[coord] Doubao 全局超时 {} 秒",
                                COORDINATOR_GLOBAL_TIMEOUT_SECS
                            );
                            asr.cancel();
                            Err(TranscribeFail::new(
                                "识别超时".to_string(),
                                "doubao global timeout".to_string(),
                            ))
                        }
                    }
                }
                ActiveAsr::GrokStt(asr) => {
                    debug_assert!(uses_global_timeout);
                    if let Err(e) = asr.send_last_frame().await {
                        log::error!("[coord] Grok STT send last frame failed: {e}");
                    }
                    let timeout_duration =
                        std::time::Duration::from_secs(COORDINATOR_GLOBAL_TIMEOUT_SECS);
                    match tokio::time::timeout(timeout_duration, asr.await_final_result()).await {
                        Ok(Ok(r)) => Ok(RawTranscript {
                            text: r,
                            duration_ms: asr.session_duration_ms(),
                        }),
                        Ok(Err(e)) => {
                            inner.last_engine_ok.store(false, Ordering::SeqCst);
                            *inner.last_engine_error.lock() = Some(format!("识别失败: {e}"));
                            log::error!("[coord] Grok STT await final failed: {e}");
                            asr.cancel();
                            Err(TranscribeFail::new(format!("识别失败: {e}"), e.to_string()))
                        }
                        Err(_) => {
                            inner.last_engine_ok.store(false, Ordering::SeqCst);
                            *inner.last_engine_error.lock() = Some("识别超时".to_string());
                            log::error!(
                                "[coord] Grok STT 全局超时 {} 秒",
                                COORDINATOR_GLOBAL_TIMEOUT_SECS
                            );
                            asr.cancel();
                            Err(TranscribeFail::new(
                                "识别超时".to_string(),
                                "grok_stt global timeout".to_string(),
                            ))
                        }
                    }
                }
            };
            transcribe_outcome
        };
        tokio::select! {
            // biased：每次先查取消标志，取消优先于「转写恰好同时完成」。
            biased;
            _ = wait_for_processing_cancel(inner) => TranscribeRace::Cancelled,
            outcome = transcribe_fut => TranscribeRace::Done(outcome),
        }
    };

    let transcribe_outcome: Result<RawTranscript, TranscribeFail> = match raced {
        TranscribeRace::Cancelled => {
            log::info!("[coord] cancel during transcribe — 中断在途 ASR 请求，丢弃转写");
            // 上面 select! 已把 transcribe_fut drop 掉（中断 reqwest / 停止等待流式结果 /
            // 停止本地转写）；这里再显式 cancel 一次，促使流式 WebSocket 立即关闭、不残留
            // 后台 worker。asr_for_cancel 与被 drop 的 future 共享同一 Arc 底层。
            let asr_for_release = asr_for_cancel.clone();
            cancel_active_asr(asr_for_cancel);
            // end_session 已经把 ASR 从 inner.asr 取走，cancel_session 无法再触发
            // provider 的释放调度；取消路径必须自己补上，否则本地模型会一直占用缓存。
            schedule_cancelled_asr_release(inner, &asr_for_release, current_session_id);
            restore_prepared_windows_ime_session(inner, current_session_id);
            // 与下方「ASR 完成后 cancel 检查」同款收尾（finish_cancelled_processing 负责
            // 把 phase 收回 Idle、清 focus_target）。
            finish_cancelled_processing(inner, current_session_id);
            return Ok(());
        }
        TranscribeRace::Done(outcome) => outcome,
    };

    // ASR 完成后 cancel 检查：转写恰好跑完、用户几乎同时按 Esc（select! 走了 Done 分支）时
    // 这里兜底命中。上面赛跑分支处理的是「转写还在途中」的取消。
    // 优先级高于 empty 检查 — 用户取消 → 静默丢弃，不写失败历史也不弹错误胶囊。
    if inner.state.lock().cancelled {
        log::info!("[coord] cancel detected after ASR — discarding transcript");
        restore_prepared_windows_ime_session(inner, current_session_id);
        // PR #387 的「cancel 后清 focus_target」契约要在 Processing 路径上也成立。
        // cancel_session 在 Processing 阶段故意跳过 finish_cancel_session_state（让
        // 这里收尾），但此前的 end_session 没把 focus_target 清掉。logic-review
        // 2026-05-10 P3 (🚩) 把这条补完。
        finish_cancelled_processing(inner, current_session_id);
        return Ok(());
    }

    // ASR 失败/超时：先自动静默重试（从刚归档的音频重转，应对网络/服务端瞬时抖动）。上面的
    // cancel 检查已先行——用户主动取消的会话不会走到这里触发重试。重试拿回文本就当作正常转写
    // 继续走润色/插入；彻底失败才 fail_dictation 保留录音 + 报错（音频仍在，可去历史手动重转）。
    let raw = match transcribe_outcome {
        Ok(raw) => {
            // 会话结束即断开 WS（与已验证的 Swift Demo 行为一致）。服务端把「连接未关
            // 的会话」计入免费通道并发配额（5 路）：连接复用 + 从不主动关，连续几次
            // 听写就会占满配额，下一次按下直接 40200011 并发超限。成功路径主动 close
            // 让服务端立刻释放会话；失败路径上面的 asr.cancel() 已经断开了。
            close_active_asr(asr_for_cancel);
            raw
        }
        Err(fail) => match try_silent_retranscribe(inner, current_session_id).await {
            SilentRetryOutcome::Transcript {
                raw,
                asr_call_label: retry_label,
            } => accept_silent_retry_transcript(raw, retry_label, &mut asr_call_label),
            SilentRetryOutcome::Cancelled => {
                log::info!("[coord] cancel during silent ASR retry — discarding transcript");
                restore_prepared_windows_ime_session(inner, current_session_id);
                finish_cancelled_processing(inner, current_session_id);
                return Ok(());
            }
            SilentRetryOutcome::Exhausted(retry_label) => {
                if retry_label.is_some() {
                    asr_call_label = retry_label;
                }
                // 处理最后一次重试结果时也复查一次取消标志，覆盖「重试刚返回
                // Exhausted 与用户同时按 Esc」的窄竞态，避免误走失败提示。
                if inner.state.lock().cancelled {
                    log::info!("[coord] cancel after silent ASR retry — discarding transcript");
                    restore_prepared_windows_ime_session(inner, current_session_id);
                    finish_cancelled_processing(inner, current_session_id);
                    return Ok(());
                }
                return fail_dictation(
                    inner,
                    current_session_id,
                    elapsed,
                    transcribe_started.elapsed().as_millis() as u64,
                    fail.user_msg,
                    fail.err,
                    asr_call_label.as_ref(),
                );
            }
        },
    };
    let asr_ms = transcribe_started.elapsed().as_millis() as u64;
    let (asr_provider, asr_model) = match &asr_call_label {
        Some(label) => (Some(label.provider.clone()), label.model.clone()),
        None => (None, None),
    };

    // ASR 返回空转写护栏（来自 PR #66）：写一条 emptyTranscript 失败历史 + 错误胶囊，
    // 与 main 上其它 error 路径保持一致（带 schedule_capsule_idle 让胶囊自动消失）。
    let mut raw = raw;

    #[cfg(any(debug_assertions, test))]
    if raw.text.trim().is_empty() {
        if let Some(debug_text) = debug_transcript_override_text() {
            log::info!(
                "[coord] using debug transcript override (chars={})",
                debug_text.chars().count()
            );
            raw.text = debug_text;
        }
    }

    if raw.text.trim().is_empty() {
        let session = DictationSession {
            // session_id 与归档 wav 同名，empty 录音才能被 read_audio_recording /
            // retranscribe_recording 凭 id 找回（之前用 Uuid::new_v4，与 `<session_id>.wav`
            // 对不上，has_audio_recording 标了 true 但前端永远 404）。
            id: current_session_id.to_string(),
            created_at: Utc::now().to_rfc3339(),
            source: crate::types::HistorySource::Voice,
            raw_transcript: raw.text.clone(),
            final_text: String::new(),
            mode: inner.prefs.get().default_mode,
            style_pack_id: None,
            translation_active: false,
            polish_source: None,
            app_bundle_id: None,
            app_name: None,
            insert_status: InsertStatus::Failed,
            error_code: Some("emptyTranscript".to_string()),
            duration_ms: Some(raw.duration_ms),
            dictionary_entry_count: None,
            // empty-transcript（ASR 没识别到任何文字）也保留 wav 标记——这是用户最想
            // 通过原始录音定位"是不是麦克风太小声 / ASR 模型问题"的场景。修 pr_agent
            // "Missing Audio" 反馈。
            has_audio_recording: Some(inner.audio_archive_active.load(Ordering::Relaxed)),
            // 空转写也记下是哪个 ASR 模型给出的空结果 + 等了多久，供模型对比排查。
            asr_provider: asr_provider.clone(),
            asr_model: asr_model.clone(),
            llm_provider: None,
            llm_model: None,
            asr_ms: Some(asr_ms),
            polish_ms: None,
        };
        let prefs_snapshot = inner.prefs.get();
        if let Err(e) = inner.history.append_with_retention(
            session,
            prefs_snapshot.history_retention_days,
            prefs_snapshot.history_max_entries,
        ) {
            log::error!("[coord] history append failed: {e}");
        }
        // 通知前端刷新概览/历史（成功/失败/空转写路径统一）。
        crate::event_bus::emit_unit("history:changed");
        emit_capsule(
            inner,
            CapsuleState::Error,
            0.0,
            elapsed,
            Some("没有识别到语音".to_string()),
            None,
        );
        restore_prepared_windows_ime_session(inner, current_session_id);
        inner.state.lock().phase = SessionPhase::Idle;
        // 与成功 / 取消 / 失败收尾一致：回 Idle 即设冷却，识别中排队的热键按下同样丢弃（#856）。
        {
            let now = std::time::Instant::now();
            *inner.session_cooldown_until.lock() =
                Some(now + std::time::Duration::from_millis(POST_SESSION_COOLDOWN_MS));
        }
        schedule_capsule_idle(inner, CAPSULE_AUTO_HIDE_DELAY_MS);
        // 空转写：清掉实时上屏的临时文本。
        let temp_len = inner.live_insert.prev_len.swap(0, Ordering::SeqCst);
        if temp_len > 0 {
            let _ = crate::insertion::macos::delete_chars(temp_len);
        }
        return Err("ASR returned empty transcript".to_string());
    }

    // 拿到非空转写 → 原始音频对「ASR 重试」已无价值。非 debug 用户：删掉刚归档的 wav
    // （隐私——成功的口述不留痕，只保留失败录音供手动重转 / 自动重试），并把
    // audio_archive_active 翻成 false，让下游 history 的 has_audio_recording 读到真实状态
    // （成功条目不会渲染播放/重转按钮再 404）。debug 用户：保留全部录音（原调试行为）。
    // 失败/超时路径在上面的 match 内就产出 Err 并走 fail_dictation，不会走到这里，失败录音始终留存。
    if !inner.prefs.get().record_audio_for_debug
        && inner.audio_archive_active.swap(false, Ordering::Relaxed)
    {
        if let Ok(path) =
            crate::persistence::recording_path_for_session(&current_session_id.to_string())
        {
            if let Err(e) = tokio::fs::remove_file(&path).await {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("[coord] 清理成功口述的归档录音失败: {e}");
                }
            }
        }
    }

    emit_capsule(inner, CapsuleState::Polishing, 0.0, elapsed, None, None);

    let prefs = inner.prefs.get();
    let working_languages = prefs.working_languages.clone();
    let chinese_script_preference = prefs.chinese_script_preference;
    // doudou 无 LLM / 风格包：翻译与 LLM 润色恒关闭，文本直通
    // （引擎自动校正已在 doubao 侧完成）。
    let raw_uses_llm = false;
    let translation_active = false;
    let mode = PolishMode::Raw;
    log::info!(
        "[coord] polish dispatch: translation={translation_active} mode={mode:?} raw_chars={} working_languages={:?}",
        raw.text.chars().count(),
        working_languages
    );

    // Linux: emit_capsule(Polishing) 已通过 fcitx5 auxDown 显示 "✨ 润色中..."，
    // 无需在此重复调用。

    // doudou 无 LLM：最终文本 = 引擎输出原样直通（实时上屏的临时文本由
    // doubao partial 驱动，此处是定稿替换）。
    let (polished, polish_error, already_streamed) = (raw.text.clone(), None, false);
    let polish_ms: Option<u64> = None;
    let (llm_provider, llm_model) = (None, None);

    let polished = finalize_polished_text(
        polished,
        translation_active,
        raw_uses_llm,
        mode,
        &polish_error,
        chinese_script_preference,
        already_streamed,
    );
    // 原子化最后一次 cancel 检查 + 转 Inserting：
    // 在同一 lock 内决定「丢弃」还是「进入 Inserting」。一旦设到 Inserting，
    // cancel_session 就拒绝介入（Cmd+V 已发出，撤销不掉）。这是 audit HIGH #2 的修复，
    // 之前 check 与 inserter.insert 之间有窗口期。
    //
    // 流式路径例外：`already_streamed = true` 表示字符已经一边流一边落到光标了，
    // 撤销不掉。即使 cancel 旗在中途被立起来，也只能尊重「已经发生」的事实，进入
    // Inserting 状态完成 history / vocab 等收尾工作。
    let proceed_to_insert = {
        let mut state = inner.state.lock();
        if state.cancelled && !already_streamed {
            false
        } else {
            state.phase = SessionPhase::Inserting;
            true
        }
    };
    if !proceed_to_insert {
        log::info!(
            "[coord] cancel detected before insert — discarding output (chars={})",
            polished.chars().count()
        );
        restore_prepared_windows_ime_session(inner, current_session_id);
        finish_cancelled_processing(inner, current_session_id);
        return Ok(());
    }

    // 替换实时上屏的临时文本：删除 → 立即插入最终（连续操作，无空白闪烁）。
    let temp_len = inner.live_insert.prev_len.swap(0, Ordering::SeqCst);
    if temp_len > 0 {
        let _ = crate::insertion::macos::delete_chars(temp_len);
    }

    let focus_target = inner.state.lock().focus_target;
    let focus_ready_for_paste = restore_focus_target_if_possible(focus_target);
    let prefs = inner.prefs.get();
    let restore_clipboard = prefs.restore_clipboard_after_paste;
    let allow_non_tsf_insertion_fallback = prefs.allow_non_tsf_insertion_fallback;
    let windows_insertion_mode = prefs.windows_insertion_mode;
    let paste_shortcut = prefs.paste_shortcut;
    // 流式路径下，字符已经通过 Unicode keystroke 落到光标处，跳过 inserter.insert。
    let status = if already_streamed {
        log::info!(
            "[coord] insertion skipped: {} chars already streamed via unicode_keystroke (polish_error={:?})",
            polished.chars().count(),
            polish_error
        );
        InsertStatus::Inserted
    } else {
        if focus_ready_for_paste {
            #[cfg(target_os = "windows")]
            {
                match windows_insertion_mode {
                    crate::types::WindowsInsertionMode::SendInput => {
                        let sendinput_options = windows_sendinput_options_from_prefs(&prefs);
                        if allow_non_tsf_insertion_fallback {
                            insert_via_non_tsf_fallback(
                                inner,
                                &polished,
                                restore_clipboard,
                                paste_shortcut,
                            )
                        } else {
                            inner
                                .inserter
                                .insert_via_unicode_keystrokes(&polished, sendinput_options)
                        }
                    }
                    crate::types::WindowsInsertionMode::Paste => {
                        inner
                            .inserter
                            .insert(&polished, restore_clipboard, paste_shortcut)
                    }
                    crate::types::WindowsInsertionMode::Tsf => {
                        let ime_target = capture_ime_submit_target();
                        insert_with_windows_ime_first(
                            inner,
                            current_session_id,
                            &polished,
                            restore_clipboard,
                            allow_non_tsf_insertion_fallback,
                            paste_shortcut,
                            ime_target,
                        )
                        .await
                    }
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                inner
                    .inserter
                    .insert(&polished, restore_clipboard, paste_shortcut)
            }
        } else {
            #[cfg(target_os = "linux")]
            {
                // Linux: fcitx5 commitString 无需窗口焦点，始终尝试插入。
                inner
                    .inserter
                    .insert(&polished, restore_clipboard, paste_shortcut)
            }
            #[cfg(not(target_os = "linux"))]
            {
                log::warn!(
                    "[coord] original insertion target is not foreground; copied output without paste"
                );
                if allow_non_tsf_insertion_fallback {
                    inner.inserter.copy_fallback(&polished)
                } else {
                    InsertStatus::Failed
                }
            }
        }
    };
    restore_prepared_windows_ime_session(inner, current_session_id);
    let inserted_chars = polished.chars().count() as u32;

    // polish 失败时在 history 里标记 polishFailed，让用户能在历史详情看到为什么这次输出
    // 不是预期的 mode 风格。即使失败也不丢词 — final_text 仍是原文（保留"用户的话不丢"语义）。
    let error_code = dictation_error_code(
        status,
        polish_error.is_some(),
        focus_ready_for_paste,
        allow_non_tsf_insertion_fallback,
        windows_insertion_mode,
    )
    .map(str::to_string);
    let tsf_required_insert_failed = error_code.as_deref() == Some("windowsImeTsfRequired");

    // 与 coordinator 内部 SessionId 对齐：方便 recorder 旁路写盘的 `<session_id>.wav`
    // 跟 history 这条 DictationSession.id 同名，前端凭 id 就能找到对应录音文件。
    let history_session_id = current_session_id.to_string();
    let history_created_at = Utc::now().to_rfc3339();
    let prefs_snapshot = inner.prefs.get();
    let session = DictationSession {
        id: history_session_id.clone(),
        created_at: history_created_at.clone(),
        source: crate::types::HistorySource::Voice,
        raw_transcript: raw.text.clone(),
        final_text: polished.clone(),
        mode,
        style_pack_id: None,
        translation_active,
        polish_source: None,
        app_bundle_id: None,
        app_name: None,
        insert_status: status,
        error_code,
        duration_ms: Some(raw.duration_ms),
        dictionary_entry_count: None,
        // 用 begin_session 时 Recorder::start 返回的实际写盘状态，而不是 prefs 开关——
        // 开关打开但路径创建失败时这里是 false，避免前端渲染播放按钮后端 404。
        has_audio_recording: Some(inner.audio_archive_active.load(Ordering::Relaxed)),
        asr_provider,
        asr_model,
        llm_provider,
        llm_model,
        asr_ms: Some(asr_ms),
        polish_ms,
    };
    if let Err(e) = inner.history.append_with_retention(
        session,
        prefs_snapshot.history_retention_days,
        prefs_snapshot.history_max_entries,
    ) {
        log::error!("[coord] history append failed: {e}");
    }
    // 成功路径也要通知前端刷新概览/历史（空转写路径在别处已 emit，这里补成功路径）。
    crate::event_bus::emit_unit("history:changed");
    // 活动计数（概览页热力图数据源）：只有成功完成的听写才点亮格子——转录失败 /
    // 错误收尾的两处 append 不计。写失败不阻断主流程。
    if let Err(e) = inner
        .activity
        .bump(&chrono::Local::now().format("%Y-%m-%d").to_string())
    {
        log::warn!("[coord] activity bump failed: {e}");
    }

    let done_message = if tsf_required_insert_failed {
        Some("TSF 未上屏，已禁止非 TSF 兜底".to_string())
    } else {
        default_done_message(status, polish_error.is_some())
    };

    // 胶囊只在 error 态渲染 message —— done 态按设计是「冻结光效淡出、不带文字」
    // （见 Capsule.tsx 的 VoiceOrbStage：`state === 'error' && <span>{message}</span>`）。
    // 所以失败信息必须走 error 态才看得见，否则文案算出来就被前端丢掉。
    //
    // 最典型的受害者是润色失败：它会静默回退成未润色的原文，而胶囊照常显示成功态，
    // 用户界面上没有任何痕迹。实际后果是 LLM 凭证失效后，用户连着十几个小时每句话
    // 都在拿原文，只能靠「今天出来的字怎么变笨了」察觉，日志里其实每一句都报了错。
    let session_failed =
        tsf_required_insert_failed || polish_error.is_some() || status == InsertStatus::Failed;
    // 引擎状态卡（概览页 get_engine_status 数据源）：会话成功/失败同步记录。
    // 清理瘦身时丢失了成功路径的写入（失败路径在 open_session 处直接写字段），
    // 这里在 end_session 收尾统一补回。
    if session_failed {
        inner.last_engine_ok.store(false, Ordering::SeqCst);
        *inner.last_engine_error.lock() = Some("会话未完成".to_string());
    } else {
        inner.last_engine_ok.store(true, Ordering::SeqCst);
        *inner.last_engine_error.lock() = None;
    }
    let capsule_state = if session_failed {
        CapsuleState::Error
    } else {
        CapsuleState::Done
    };

    emit_capsule(
        inner,
        capsule_state,
        0.0,
        elapsed,
        done_message,
        Some(inserted_chars),
    );

    {
        let mut state = inner.state.lock();
        state.phase = SessionPhase::Idle;
        state.focus_target = None;
    }
    // Toggle 模式冷却：设冷却时间戳，POST_SESSION_COOLDOWN_MS 内禁止新的 activate。
    // 覆盖胶囊离场动画周期，避免三连按第 3 次误激活（issue #545）。
    {
        let now = std::time::Instant::now();
        *inner.session_cooldown_until.lock() =
            Some(now + std::time::Duration::from_millis(POST_SESSION_COOLDOWN_MS));
    }
    schedule_capsule_idle(inner, CAPSULE_AUTO_HIDE_DELAY_MS);

    Ok(())
}

pub(super) fn dictation_error_code(
    status: InsertStatus,
    polish_failed: bool,
    focus_ready_for_paste: bool,
    allow_non_tsf_insertion_fallback: bool,
    windows_insertion_mode: crate::types::WindowsInsertionMode,
) -> Option<&'static str> {
    if !focus_ready_for_paste && status == InsertStatus::Failed {
        Some("focusRestoreFailed")
    } else if cfg!(target_os = "windows")
        && focus_ready_for_paste
        && !allow_non_tsf_insertion_fallback
        && windows_insertion_mode == crate::types::WindowsInsertionMode::Tsf
        && status == InsertStatus::Failed
    {
        Some("windowsImeTsfRequired")
    } else if polish_failed {
        Some("polishFailed")
    } else {
        None
    }
}

pub(super) fn cancel_session(inner: &Arc<Inner>) -> bool {
    // 取消时清理实时上屏的临时文本并恢复输入法。
    inner.live_insert.enabled.store(false, Ordering::SeqCst);
    let temp_len = inner.live_insert.prev_len.swap(0, Ordering::SeqCst);
    if temp_len > 0 {
        let _ = crate::insertion::macos::delete_chars(temp_len);
    }
    let prev_ime = inner.live_insert.prev_ime.lock().take();
    if let Some(prev) = prev_ime {
        crate::runtime().spawn(async move {
            let _ = crate::unicode_keystroke::restore_input_source(Some(prev)).await;
        });
    }

    let Some(decision) = ({
        let mut state = inner.state.lock();
        let phase = state.phase;
        let decision = begin_cancel_session_state(&mut state);
        if phase == SessionPhase::Inserting {
            log::info!("[coord] cancel ignored — already in Inserting phase, can't undo paste");
        }
        decision
    }) else {
        return false;
    };

    // 顺序要紧：先把 UI 收干净，再去拆麦克风 / ASR。
    //
    // 反过来（原来的顺序）会让胶囊等在 `stop_recorder_for_session` 后面 ——
    // `Recorder::stop()` 要 join 音频线程，而音频线程退出前要 join liveness watchdog，
    // watchdog 又睡在自己的检查间隔里，实测撤销到胶囊消失能差 0.8~1 秒。用户按 Option+Q
    // 或按 Esc 的观感就是「明明已经取消了，胶囊还赖着」。拆资源不需要 UI 等它，反正
    // 这段时间录到的音频整条会话都要丢。
    //
    // 代价：胶囊消失后麦克风还会多开一小会儿（系统菜单栏的录音小圆点晚灭）。这段窗口
    // 必须足够短 —— 否则紧接着那次真想说话的按下会在旧 recorder 还占着麦克风时
    // build_input_stream，而 `Recorder` 没有 Drop 停采，recorder 槽被新会话覆盖后旧音频
    // 线程会继续跑、抓着麦克风不放。所以 watchdog 的检查间隔必须是碎的（见
    // recorder.rs 的 WATCHDOG_*），把这段窗口压到几十毫秒；两处改动是一对，不能只留一个。
    //
    // Processing 阶段保持 phase=Processing 让 end_session 自己走完检查 + 收尾；
    // 其他阶段直接转 Idle。
    if decision.phase != SessionPhase::Processing {
        let mut state = inner.state.lock();
        finish_cancel_session_state(&mut state, decision);
        // 只有真正把 phase 设为 Idle 时才设冷却（避免离场动画期间误激活）。
        let now = std::time::Instant::now();
        *inner.session_cooldown_until.lock() =
            Some(now + std::time::Duration::from_millis(POST_SESSION_COOLDOWN_MS));
    }
    // emit_capsule 仍然排在 finish_cancel_session_state 之后：它要读 phase 拼 payload，
    // 提到前面会发出「还在进行中」的那一帧。
    emit_capsule(inner, CapsuleState::Cancelled, 0.0, 0, None, None);
    log::info!("[coord] session cancelled (was {:?})", decision.phase);
    schedule_capsule_idle(inner, CAPSULE_CANCEL_HIDE_DELAY_MS);

    stop_recorder_for_session(inner, decision.session_id);
    cancel_asr_for_session(inner, decision.session_id);
    restore_prepared_windows_ime_session(inner, decision.session_id);
    true
}

#[cfg(any(target_os = "windows", test))]
fn append_typed_prefix(target: &mut String, delta: &str, typed_chars: usize) -> usize {
    let mut end = 0;
    let mut appended = 0;
    for (idx, ch) in delta.char_indices().take(typed_chars) {
        end = idx + ch.len_utf8();
        appended += 1;
    }
    target.push_str(&delta[..end]);
    appended
}

#[cfg(test)]
mod tests {
    use super::{
        accept_silent_retry_transcript, append_typed_prefix, build_transcribe_failed_session,
        default_done_message, drain_streaming_insert_deltas_with, finalize_polished_text,
        flush_streaming_insert_buffer_with, pcm_duration_ms, pcm_from_wav_bytes,
    };
    use crate::types::{ChineseScriptPreference, DictationSession, InsertStatus, PolishMode};
    use uuid::Uuid;

    fn coordinator_with_dictation_hotkey(
        binding: crate::types::ShortcutBinding,
    ) -> super::super::Coordinator {
        let coordinator = super::super::Coordinator::new();
        coordinator
            .inner
            .prefs
            .set(crate::types::UserPreferences {
                dictation_hotkey: binding,
                ..Default::default()
            })
            .unwrap();
        coordinator
    }

    // modifier-only 触发键：按下后必须先过仲裁窗口，才能知道这是说话还是
    // Option+任意字母/数字键。
    #[tokio::test]
    async fn modifier_only_press_waits_out_the_arbitration_window() {
        let coordinator = coordinator_with_dictation_hotkey(crate::types::ShortcutBinding {
            primary: "LeftOption".into(),
            modifiers: vec![],
        });

        let started = std::time::Instant::now();
        // 测试里没装监听器（inner.hotkey = None）→ 读不到叠加标志，按「不是组合键」放行。
        assert!(!super::press_resolves_to_combo(&coordinator.inner, 1).await);
        assert!(started.elapsed() >= super::COMBO_ARBITRATION_GRACE);
    }

    #[tokio::test]
    async fn arbitration_combo_does_not_consume_debounce_window() {
        let coordinator = coordinator_with_dictation_hotkey(crate::types::ShortcutBinding {
            primary: "LeftOption".into(),
            modifiers: vec![],
        });
        coordinator
            .inner
            .hotkey_press_generation
            .store(1, std::sync::atomic::Ordering::SeqCst);
        coordinator
            .inner
            .hotkey_combo_pending_presses
            .lock()
            .push_back(1);
        *coordinator.inner.last_hotkey_dispatch_at.lock() = Some(std::time::Instant::now());

        super::begin_session_from_press(&coordinator.inner, 1).await;

        assert!(coordinator.inner.last_hotkey_dispatch_at.lock().is_none());
        assert_eq!(
            coordinator.inner.state.lock().phase,
            crate::coordinator_state::SessionPhase::Idle
        );
    }

    // 自定义组合键（Cmd+Shift+D）没有歧义 —— 白等这一下就是纯掉延迟。
    #[tokio::test]
    async fn custom_combo_press_skips_the_arbitration_window() {
        let coordinator = coordinator_with_dictation_hotkey(crate::types::ShortcutBinding {
            primary: "D".into(),
            modifiers: vec!["cmd".into(), "shift".into()],
        });

        let started = std::time::Instant::now();
        assert!(!super::press_resolves_to_combo(&coordinator.inner, 1).await);
        assert!(!super::combo_seen_for_press(&coordinator.inner, 0));
        assert!(started.elapsed() < super::COMBO_ARBITRATION_GRACE);
    }

    #[test]
    fn pending_combo_queue_preserves_multiple_press_ids() {
        let coordinator = super::super::Coordinator::new();
        coordinator
            .inner
            .hotkey_combo_pending_presses
            .lock()
            .extend([11, 12]);

        assert!(super::combo_seen_for_press(&coordinator.inner, 11));
        assert!(super::combo_seen_for_press(&coordinator.inner, 12));
        assert!(!super::combo_seen_for_press(&coordinator.inner, 11));
    }

    #[test]
    fn silent_retry_replaces_initial_asr_attribution() {
        let mut label = Some(super::AsrCallLabel::new(
            "volcengine",
            Some("volc.seedasr.sauc.duration".into()),
        ));
        let retry_label = super::AsrCallLabel::new(
            "bailian-qwen3-realtime",
            Some("qwen3-asr-flash-realtime".into()),
        );
        let raw = super::RawTranscript {
            text: "重试成功".into(),
            duration_ms: 900,
        };

        let accepted = accept_silent_retry_transcript(raw, retry_label.clone(), &mut label);

        assert_eq!(accepted.text, "重试成功");
        assert_eq!(label, Some(retry_label));
    }

    #[allow(clippy::too_many_arguments)]
    fn history_session(
        id: &str,
        raw: &str,
        final_text: &str,
        style_pack_id: Option<&str>,
        translation_active: bool,
        polish_source: Option<&str>,
    ) -> DictationSession {
        DictationSession {
            id: id.into(),
            created_at: "2026-06-03T00:00:00Z".into(),
            source: crate::types::HistorySource::Voice,
            raw_transcript: raw.into(),
            final_text: final_text.into(),
            mode: PolishMode::Structured,
            app_bundle_id: None,
            app_name: None,
            insert_status: InsertStatus::Inserted,
            error_code: None,
            duration_ms: Some(1000),
            dictionary_entry_count: None,
            has_audio_recording: None,
            style_pack_id: style_pack_id.map(str::to_string),
            translation_active,
            polish_source: polish_source.map(str::to_string),
            asr_provider: None,
            asr_model: None,
            llm_provider: None,
            llm_model: None,
            asr_ms: None,
            polish_ms: None,
        }
    }

    #[test]
    fn transcribe_failed_history_keeps_session_id_for_recording_lookup() {
        // 修 #613：失败 / empty 历史条目的 id 必须 == coordinator SessionId，这样归档录音
        // `recordings/<session_id>.wav` 才能被 read_audio_recording / retranscribe_recording
        // 凭 id 找回。之前 empty 分支用 Uuid::new_v4()，与 wav 文件名对不上 → 前端永远 404、
        // 录音随 prune 丢失（用户报告「识别失败之前的语音也都丢失了」）。
        let sid = Uuid::new_v4();
        let session =
            build_transcribe_failed_session(sid, 4200, 17_250, PolishMode::Structured, true);
        assert_eq!(session.id, sid.to_string());
    }

    #[test]
    fn transcribe_failed_history_marks_failed_and_recoverable() {
        let sid = Uuid::new_v4();
        let session =
            build_transcribe_failed_session(sid, 1234, 17_250, PolishMode::Structured, true);
        assert!(matches!(session.insert_status, InsertStatus::Failed));
        assert_eq!(session.error_code.as_deref(), Some("transcribeFailed"));
        assert_eq!(session.duration_ms, Some(1234));
        assert_eq!(session.asr_ms, Some(17_250));
        // 归档成功 → 标 has_audio_recording=true，前端据此渲染「重新转录」入口。
        assert_eq!(session.has_audio_recording, Some(true));
    }

    #[test]
    fn transcribe_failed_history_flags_no_audio_when_archive_inactive() {
        // 录音归档失败（has_audio=false）→ 条目仍写（用户看得到这次失败），但不标可重转，
        // 避免前端渲染重转按钮而后端找不到 wav。
        let sid = Uuid::new_v4();
        let session = build_transcribe_failed_session(sid, 1, 250, PolishMode::Structured, false);
        assert_eq!(session.has_audio_recording, Some(false));
    }

    #[test]
    fn pcm_from_wav_strips_44_byte_header() {
        // 自动静默重试从归档 wav 取 PCM：标准 16k/mono/16-bit 头固定 44 字节，PCM = 头之后全部。
        let mut wav = vec![0u8; 44];
        wav.extend_from_slice(&[1, 2, 3, 4]);
        assert_eq!(pcm_from_wav_bytes(&wav), Some(vec![1, 2, 3, 4]));
    }

    #[test]
    fn pcm_from_wav_rejects_headeronly_or_truncated() {
        // <= 44 字节 = 没有音频负载（空录音 / 截断）→ None，不触发无意义的重试。
        assert_eq!(pcm_from_wav_bytes(&[0u8; 44]), None);
        assert_eq!(pcm_from_wav_bytes(&[0u8; 10]), None);
        assert_eq!(pcm_from_wav_bytes(&[]), None);
    }

    #[test]
    fn pcm_duration_ms_matches_16k_mono_16bit_rate() {
        // 16000 样本/秒 × 2 字节/样本 = 32000 字节/秒 = 32 字节/毫秒。
        assert_eq!(pcm_duration_ms(32_000), 1000); // 1s
        assert_eq!(pcm_duration_ms(16_000), 500); // 0.5s
        assert_eq!(pcm_duration_ms(32), 1); // 1ms
        assert_eq!(pcm_duration_ms(0), 0);
    }

    #[test]
    fn streamed_output_skips_postprocessing_mutations() {
        let result = finalize_polished_text(
            "Open AI".into(),
            false,
            false,
            PolishMode::Raw,
            &None,
            ChineseScriptPreference::Auto,
            true,
        );

        assert_eq!(result, "Open AI");
    }

    #[test]
    fn raw_llm_output_still_applies_script_preference() {
        let result = finalize_polished_text(
            "繁體".into(),
            false,
            true,
            PolishMode::Raw,
            &None,
            ChineseScriptPreference::Simplified,
            false,
        );

        assert_eq!(result, "繁体");
    }

    #[test]
    fn append_typed_prefix_keeps_unicode_char_boundaries() {
        let mut typed = String::from("前");

        let appended = append_typed_prefix(&mut typed, "a你🙂b", 3);

        assert_eq!(appended, 3);
        assert_eq!(typed, "前a你🙂");
    }

    #[test]
    fn append_typed_prefix_caps_at_delta_length() {
        let mut typed = String::new();

        let appended = append_typed_prefix(&mut typed, "好", 10);

        assert_eq!(appended, 1);
        assert_eq!(typed, "好");
    }

    #[test]
    fn polish_output_honors_chinese_script_preference() {
        // issue #643：polish 模式（非 Raw、polish 成功）的成品也按用户字形偏好确定性转换，
        // 不再依赖 LLM 提示——繁中用户因此每次都拿到繁体。
        let finalize = |pref| {
            finalize_polished_text(
                "学习".to_string(),
                false, // translation_active
                false, // raw_uses_llm
                PolishMode::Structured,
                &None, // polish 成功
                pref,
                false, // already_streamed
            )
        };
        // 繁体偏好：学习 → 學習（OpenCC S2t），至少不再含简体「学/习」。
        let trad = finalize(ChineseScriptPreference::Traditional);
        assert!(
            !trad.contains('学') && !trad.contains('习'),
            "traditional pref left simplified chars: {trad}"
        );
        // 简体偏好：保持简体（输入已是简体，T2s 无变化）。
        let simp = finalize(ChineseScriptPreference::Simplified);
        assert!(
            simp.contains('学') && simp.contains('习'),
            "simplified pref: {simp}"
        );
        // Auto：不转换，对默认用户零影响。
        assert_eq!(finalize(ChineseScriptPreference::Auto), "学习");
    }

    #[test]
    fn default_done_message_works_correctly() {
        assert_eq!(
            default_done_message(InsertStatus::PasteSent, false),
            Some("已尝试粘贴".to_string())
        );
        assert_eq!(
            default_done_message(InsertStatus::Inserted, true),
            Some("润色失败，已插入原文".to_string())
        );
    }

    #[test]
    fn streaming_insert_batches_queued_deltas_before_flush() {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send("你".to_string()).unwrap();
        tx.send("好".to_string()).unwrap();
        tx.send("🙂".to_string()).unwrap();
        drop(tx);

        let mut flushed = Vec::new();
        let (typed, failure) = drain_streaming_insert_deltas_with(
            rx,
            std::time::Duration::from_millis(50),
            |pending, typed_text| {
                flushed.push(pending.clone());
                typed_text.push_str(pending);
                pending.clear();
                None
            },
        );

        assert_eq!(flushed, vec!["你好🙂".to_string()]);
        assert_eq!(typed, "你好🙂");
        assert_eq!(failure, None);
    }

    #[test]
    fn flush_streaming_insert_buffer_keeps_partial_unicode_prefix() {
        let mut pending = "a你🙂b".to_string();
        let mut typed = String::new();

        let failure = flush_streaming_insert_buffer_with(&mut pending, &mut typed, |_| {
            Err(crate::unicode_keystroke::TypeError::Partial {
                typed_chars: 3,
                source: Box::new(platform_type_error()),
            })
        });

        assert_eq!(typed, "a你🙂");
        assert!(pending.is_empty());
        assert!(failure.is_some());
    }

    #[cfg(target_os = "macos")]
    fn platform_type_error() -> crate::unicode_keystroke::TypeError {
        crate::unicode_keystroke::TypeError::EventAllocFailed
    }

    #[cfg(target_os = "windows")]
    fn platform_type_error() -> crate::unicode_keystroke::TypeError {
        crate::unicode_keystroke::TypeError::SendInputFailed("fail".into())
    }

    #[cfg(target_os = "linux")]
    fn platform_type_error() -> crate::unicode_keystroke::TypeError {
        crate::unicode_keystroke::TypeError::EnigoText("fail".into())
    }
}
