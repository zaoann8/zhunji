//! Hotkey supervisor / bridge loops and shortcut wiring extracted from
//! `coordinator.rs` (behavior-preserving move; see git history).
//!
//! Functions operate on the parent `Inner`/`Coordinator` and reference
//! parent-module items via `use super::*;`. Visibility is `pub(super)` so the
//! parent `coordinator` module can call them through `use hotkey_loops::*;`.

use super::*;

// ─────────────────────────── hotkey bridging ───────────────────────────

/// Esc 取消专用消费线程。为什么不并入 `hotkey_bridge_loop`：bridge 为修 #468/#475
/// 的 latch 竞态把 Pressed/Released 改成了串行 block_on —— Hold 松手后 `end_session`
/// 会在 bridge 线程上同步跑完整段转写 + 润色，期间 bridge 无法 recv。若 Esc 与其同
/// 队列，取消事件只能排队等流程跑完（此时 phase 已回 Idle，cancel 变 no-op），#798
/// 在 `end_session` 里的 select! 取消赛跑永远等不到 `cancelled` 旗标 ——「转写 / 润色
/// 中按 Esc 停不下来」。独立通道 + 本线程保证 `cancel_session` 随到随执行（它是纯同步
/// 快路径：置旗标 + 清资源，不 await）。
pub(super) fn esc_cancel_bridge_loop(inner: Arc<Inner>, rx: mpsc::Receiver<()>) {
    while rx.recv().is_ok() {
        if inner.shortcut_recording_active.load(Ordering::SeqCst) {
            continue;
        }
        cancel_session(&inner);
    }
}

/// 组合键撤销专用消费线程。撤销事件携带触发键按下代次，避免独立通道的迟到事件
/// 误取消下一次按下开启的会话。
pub(super) fn combo_abort_bridge_loop(
    inner: Arc<Inner>,
    rx: mpsc::Receiver<u64>,
    handler: fn(&Arc<Inner>, u64),
) {
    while let Ok(press_id) = rx.recv() {
        if inner.shortcut_recording_active.load(Ordering::SeqCst) {
            continue;
        }
        handler(&inner, press_id);
    }
}

pub(super) fn spawn_esc_cancel_bridge(inner: &Arc<Inner>) -> mpsc::Sender<()> {
    let (cancel_tx, cancel_rx) = mpsc::channel::<()>();
    let bridge_inner = Arc::clone(inner);
    if let Err(e) = std::thread::Builder::new()
        .name("openless-esc-cancel-bridge".into())
        .spawn(move || esc_cancel_bridge_loop(bridge_inner, cancel_rx))
    {
        // 线程建不起来 = 取消通道没有消费者，Esc 取消会静默失效——这正是本 PR 想修的
        // bug 以另一种方式回归，必须留 error 日志以便排查。
        log::error!("[hotkey] esc-cancel-bridge 线程启动失败，Esc 取消将不可用: {e}");
    }
    cancel_tx
}

pub(super) fn spawn_combo_abort_bridge(
    inner: &Arc<Inner>,
    handler: fn(&Arc<Inner>, u64),
) -> mpsc::Sender<u64> {
    let (combo_tx, combo_rx) = mpsc::channel::<u64>();
    let bridge_inner = Arc::clone(inner);
    std::thread::Builder::new()
        .name("openless-combo-abort-bridge".into())
        .spawn(move || combo_abort_bridge_loop(bridge_inner, combo_rx, handler))
        .ok();
    combo_tx
}

pub(super) fn hotkey_supervisor_loop(inner: Arc<Inner>) {
    let mut attempts: u32 = 0;
    let capability = HotkeyMonitor::capability();
    loop {
        if inner.shutdown.load(Ordering::SeqCst) {
            return;
        }
        let prefs = inner.prefs.get();

        if inner.hotkey.lock().is_some() {
            return;
        }
        // Linux: 启动前检查 fcitx5 插件是否可用
        #[cfg(target_os = "linux")]
        if !crate::linux_fcitx::available() {
            *inner.hotkey_status.lock() = HotkeyStatus {
                adapter: capability.adapter,
                state: HotkeyStatusState::Failed,
                message: Some("fcitx5 插件不可用 — 请确保 fcitx5 已安装且在运行".into()),
                last_error: Some(crate::types::HotkeyInstallError {
                    code: "fcitx5_unavailable".into(),
                    message: "fcitx5 插件 DBus 接口无响应".into(),
                }),
            };
            log::warn!("[hotkey-supervisor] fcitx5 plugin unavailable, retrying...");
            attempts += 1;
            std::thread::sleep(std::time::Duration::from_secs(3));
            continue;
        }
        *inner.hotkey_status.lock() = HotkeyStatus {
            adapter: capability.adapter,
            state: HotkeyStatusState::Starting,
            message: Some(format!("正在安装全局快捷键监听（第 {} 次）", attempts + 1)),
            last_error: None,
        };
        let trigger = crate::shortcut_binding::legacy_modifier_trigger(&prefs.dictation_hotkey)
            .unwrap_or(crate::types::HotkeyTrigger::Custom);
        let binding = crate::types::HotkeyBinding {
            trigger,
            mode: prefs.hotkey.mode,
            keys: None,
        };
        let (tx, rx) = mpsc::channel::<HotkeyEvent>();
        #[cfg(target_os = "linux")]
        let (fcitx_tx, fcitx_binding) = (tx.clone(), binding.clone());
        let cancel_tx = spawn_esc_cancel_bridge(&inner);
        let combo_tx = spawn_combo_abort_bridge(&inner, handle_trigger_combined);
        #[cfg(target_os = "linux")]
        let combo_tx_for_fcitx = combo_tx.clone();
        match HotkeyMonitor::start(binding, tx, cancel_tx, combo_tx) {
            Ok(monitor) => {
                let adapter = monitor.kind();
                *inner.hotkey.lock() = Some(monitor);
                if let Some(monitor) = inner.hotkey.lock().as_ref() {
                    let (qa_trigger, translation_trigger) = modifier_shortcut_triggers(&inner);
                    monitor.update_modifier_shortcuts(qa_trigger, translation_trigger);
                }
                *inner.hotkey_status.lock() = HotkeyStatus {
                    adapter,
                    state: HotkeyStatusState::Installed,
                    message: Some(format!("{} 已安装", adapter.display_name())),
                    last_error: None,
                };
                log::info!(
                    "[coord] hotkey listener installed (after {} attempt(s))",
                    attempts + 1
                );
                let inner_clone = Arc::clone(&inner);
                std::thread::Builder::new()
                    .name("openless-hotkey-bridge".into())
                    .spawn(move || hotkey_bridge_loop(inner_clone, rx))
                    .ok();
                // Linux: 启动 fcitx5 插件信号监听作为热键源。
                #[cfg(target_os = "linux")]
                {
                    let (qa_trigger, translation_trigger) = modifier_shortcut_triggers(&inner);
                    let custom_key = custom_dictation_key_string(&inner);
                    crate::linux_fcitx::start_dictation_signal_listener(
                        fcitx_tx,
                        combo_tx_for_fcitx,
                        fcitx_binding.clone(),
                        qa_trigger,
                        translation_trigger,
                        custom_key,
                    );
                    if fcitx_binding.trigger == crate::types::HotkeyTrigger::Custom {
                        sync_custom_dictation_to_plugin(&inner);
                    } else {
                        crate::linux_fcitx::sync_binding_to_plugin(&fcitx_binding);
                    }
                }
                return;
            }
            Err(e) => {
                attempts += 1;
                let error_message = e.message.clone();
                *inner.hotkey_status.lock() = HotkeyStatus {
                    adapter: capability.adapter,
                    state: HotkeyStatusState::Failed,
                    message: Some(error_message.clone()),
                    last_error: Some(e),
                };
                if attempts <= 3 || attempts % 10 == 0 {
                    log::warn!(
                        "[coord] hotkey listener attempt #{attempts} failed: {}; retrying in 3s",
                        error_message
                    );
                }
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        }
    }
}

// ─────────────────────────── combo hotkey supervisor ───────────────────────────

pub(super) fn combo_hotkey_supervisor_loop(inner: Arc<Inner>) {
    let mut attempts: u32 = 0;
    loop {
        if inner.shutdown.load(Ordering::SeqCst) {
            return;
        }
        // 读当前 prefs
        let prefs = inner.prefs.get();
        if crate::shortcut_binding::legacy_modifier_trigger(&prefs.dictation_hotkey).is_some() {
            take_combo_hotkey_on_main_thread(&inner);
            inner.side_aware_combo.lock().take();
            return;
        }

        let binding = prefs.dictation_hotkey.clone();
        if is_unconfigured_shortcut(&binding) {
            take_combo_hotkey_on_main_thread(&inner);
            inner.side_aware_combo.lock().take();
            return;
        }

        if crate::shortcut_binding::binding_requires_side_aware_hook(&binding) {
            take_combo_hotkey_on_main_thread(&inner);
            if inner.side_aware_combo.lock().is_some() {
                return;
            }
            let (tx, rx) = mpsc::channel::<ComboHotkeyEvent>();
            match crate::side_aware_combo::SideAwareComboMonitor::start(binding, tx) {
                Ok(monitor) => {
                    *inner.side_aware_combo.lock() = Some(monitor);
                    let inner_clone = Arc::clone(&inner);
                    std::thread::Builder::new()
                        .name("openless-side-combo-bridge".into())
                        .spawn(move || combo_hotkey_bridge_loop(inner_clone, rx))
                        .ok();
                    return;
                }
                Err(e) => {
                    attempts += 1;
                    if attempts <= 3 || attempts % 10 == 0 {
                        log::warn!(
                            "[coord] side-aware combo 第 {attempts} 次注册失败: {e}; 3s 后重试"
                        );
                    }
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    continue;
                }
            }
        }

        inner.side_aware_combo.lock().take();

        if inner.combo_hotkey.lock().is_some() {
            return;
        }

        // 原版经 AppHandle.run_on_main_thread 调度 + 同步等待回执；P0 直接调用
        // （global-hotkey macOS 实现是 Carbon run loop，无需 AppKit 主线程）。
        let (tx, rx) = mpsc::channel::<ComboHotkeyEvent>();
        let init_result = ComboHotkeyMonitor::start(binding.clone(), tx);

        match init_result {
            Ok(monitor) => {
                *inner.combo_hotkey.lock() = Some(monitor);
                log::info!(
                    "[coord] combo hotkey listener installed (after {} attempt(s))",
                    attempts + 1
                );
                let inner_clone = Arc::clone(&inner);
                std::thread::Builder::new()
                    .name("openless-combo-hotkey-bridge".into())
                    .spawn(move || combo_hotkey_bridge_loop(inner_clone, rx))
                    .ok();
                #[cfg(target_os = "linux")]
                sync_custom_dictation_to_plugin(&inner);
                return;
            }
            Err(e) => {
                attempts += 1;
                if attempts <= 3 || attempts % 10 == 0 {
                    log::warn!("[coord] combo hotkey 第 {attempts} 次注册失败: {e}; 3s 后重试");
                }
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        }
    }
}

pub(super) fn combo_hotkey_bridge_loop(inner: Arc<Inner>, rx: mpsc::Receiver<ComboHotkeyEvent>) {
    while let Ok(evt) = rx.recv() {
        if inner.shortcut_recording_active.load(Ordering::SeqCst) {
            continue;
        }
        let inner_cloned = Arc::clone(&inner);
        match evt {
            // P0 #468/#475: 同 hotkey_bridge_loop —— Pressed/Released 必须串行 await，
            // 否则 latch 竞态导致 combo 快捷键二次按键失效。
            ComboHotkeyEvent::Pressed { at } => {
                crate::block_on(async {
                    handle_pressed_edge(&inner_cloned, at, 0).await;
                });
            }
            ComboHotkeyEvent::Released { at } => {
                crate::block_on(async {
                    handle_released_edge(&inner_cloned, at).await;
                });
            }
        }
    }
}

pub(super) fn translation_hotkey_supervisor_loop(inner: Arc<Inner>) {
    let mut attempts: u32 = 0;
    loop {
        if inner.shutdown.load(Ordering::SeqCst) {
            return;
        }
        let binding = inner.prefs.get().translation_hotkey;
        if is_builtin_translation_shift(&binding)
            || crate::shortcut_binding::legacy_modifier_trigger(&binding).is_some()
        {
            take_translation_hotkey_on_main_thread(&inner);
            if let Some(monitor) = inner.hotkey.lock().as_ref() {
                let (qa_trigger, translation_trigger) = modifier_shortcut_triggers(&inner);
                monitor.update_modifier_shortcuts(qa_trigger, translation_trigger);
            }
            // 对齐主 supervisor 的 exit-on-success：装/卸交给 try_update_translation_hotkey_binding 主动路径，issue #470
            return;
        }

        if inner.translation_hotkey.lock().is_some() {
            // 对齐主 supervisor 的 exit-on-success：装/卸交给 try_update_translation_hotkey_binding 主动路径，issue #470
            return;
        }

        // 原版经 AppHandle.run_on_main_thread 调度；P0 直接调用（见 combo supervisor）。
        let (tx, rx) = mpsc::channel::<ComboHotkeyEvent>();
        let init_result = ComboHotkeyMonitor::start(binding.clone(), tx);

        match init_result {
            Ok(monitor) => {
                *inner.translation_hotkey.lock() = Some(monitor);
                let inner_clone = Arc::clone(&inner);
                std::thread::Builder::new()
                    .name("openless-translation-hotkey-bridge".into())
                    .spawn(move || translation_hotkey_bridge_loop(inner_clone, rx))
                    .ok();
                attempts = 0;
            }
            Err(e) => {
                attempts += 1;
                if attempts <= 3 || attempts % 10 == 0 {
                    log::warn!(
                        "[coord] translation hotkey 第 {attempts} 次注册失败: {e}; 3s 后重试"
                    );
                }
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        }
    }
}

#[allow(dead_code)] // P1 设置页翻译快捷键修改时接回
pub(super) fn update_translation_hotkey_on_main_thread(
    inner: Arc<Inner>,
    binding: crate::types::ShortcutBinding,
) -> Result<(), ComboHotkeyError> {
    if let Some(monitor) = inner.translation_hotkey.lock().as_ref() {
        return monitor.update_binding(binding);
    }
    let (tx, rx) = mpsc::channel::<ComboHotkeyEvent>();
    let monitor = ComboHotkeyMonitor::start(binding, tx)?;
    *inner.translation_hotkey.lock() = Some(monitor);
    let bridge_inner = Arc::clone(&inner);
    std::thread::Builder::new()
        .name("openless-translation-hotkey-bridge".into())
        .spawn(move || translation_hotkey_bridge_loop(bridge_inner, rx))
        .map_err(|e| ComboHotkeyError::RegisterFailed(format!("spawn bridge thread: {e}")))?;
    Ok(())
}

pub(super) fn translation_hotkey_bridge_loop(
    inner: Arc<Inner>,
    rx: mpsc::Receiver<ComboHotkeyEvent>,
) {
    while let Ok(evt) = rx.recv() {
        if inner.shortcut_recording_active.load(Ordering::SeqCst) {
            continue;
        }
        if matches!(evt, ComboHotkeyEvent::Pressed { .. }) {
            mark_translation_modifier_seen(&inner);
        }
    }
}

pub(super) fn action_hotkey_supervisor_loop(inner: Arc<Inner>, kind: ActionHotkeyKind) {
    let mut attempts: u32 = 0;
    loop {
        if inner.shutdown.load(Ordering::SeqCst) {
            return;
        }
        // None = 用户主动停用：反注册后退出守护（由 update_action_hotkey_binding 主动路径重装）。
        let Some(binding) = action_hotkey_binding(&inner, kind) else {
            take_action_hotkey_on_main_thread(&inner, kind);
            // 对齐主 supervisor 的 exit-on-success：装/卸交给 update_action_hotkey_binding 主动路径，issue #470
            return;
        };
        if is_modifier_only_shortcut(&binding) {
            take_action_hotkey_on_main_thread(&inner, kind);
            // 对齐主 supervisor 的 exit-on-success：装/卸交给 update_action_hotkey_binding 主动路径，issue #470
            return;
        }

        if action_hotkey_slot(&inner, kind).lock().is_some() {
            // 对齐主 supervisor 的 exit-on-success：装/卸交给 update_action_hotkey_binding 主动路径，issue #470
            return;
        }

        // 原版经 AppHandle.run_on_main_thread 调度；P0 直接调用（见 combo supervisor）。
        let (tx, rx) = mpsc::channel::<ComboHotkeyEvent>();
        let init_result = ComboHotkeyMonitor::start(binding.clone(), tx);

        match init_result {
            Ok(monitor) => {
                *action_hotkey_slot(&inner, kind).lock() = Some(monitor);
                log::info!(
                    "[coord] action hotkey {kind:?} listener installed after {} attempt(s)",
                    attempts + 1
                );
                let inner_clone = Arc::clone(&inner);
                std::thread::Builder::new()
                    .name(action_hotkey_bridge_thread_name(kind).into())
                    .spawn(move || action_hotkey_bridge_loop(inner_clone, rx, kind))
                    .ok();
                attempts = 0;
            }
            Err(e) => {
                attempts += 1;
                if attempts <= 3 || attempts % 10 == 0 {
                    log::warn!(
                        "[coord] action hotkey {kind:?} 第 {attempts} 次注册失败: {e}; 3s 后重试"
                    );
                }
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
        }
    }
}

pub(super) fn action_hotkey_bridge_loop(
    inner: Arc<Inner>,
    rx: mpsc::Receiver<ComboHotkeyEvent>,
    kind: ActionHotkeyKind,
) {
    while let Ok(evt) = rx.recv() {
        if inner.shortcut_recording_active.load(Ordering::SeqCst) {
            continue;
        }
        if matches!(evt, ComboHotkeyEvent::Pressed { .. }) {
            handle_action_hotkey_pressed(&inner, kind);
        }
    }
}

pub(super) fn handle_action_hotkey_pressed(_inner: &Arc<Inner>, kind: ActionHotkeyKind) {
    match kind {
        ActionHotkeyKind::OpenApp => {
            // 原版经 AppHandle.run_on_main_thread 调 show_main_window（show+focus 主窗口）；
            // P0 事件化：Swift 宿主收到后自行 show/activate 设置窗口。
            crate::event_bus::emit_unit("app:show-main-window");
        }
    }
}

// 原版经 AppHandle.run_on_main_thread 调 take；P0 直接取（global-hotkey 的
// monitor 反注册不要求 AppKit 主线程，见 update_combo_hotkey_binding 的说明）。
pub(super) fn take_combo_hotkey_on_main_thread(inner: &Arc<Inner>) {
    inner.combo_hotkey.lock().take();
}

pub(super) fn take_translation_hotkey_on_main_thread(inner: &Arc<Inner>) {
    inner.translation_hotkey.lock().take();
}

pub(super) fn take_action_hotkey_on_main_thread(inner: &Arc<Inner>, kind: ActionHotkeyKind) {
    action_hotkey_slot(inner, kind).lock().take();
}

pub(super) fn action_hotkey_slot(
    inner: &Arc<Inner>,
    kind: ActionHotkeyKind,
) -> &Mutex<Option<ComboHotkeyMonitor>> {
    match kind {
        ActionHotkeyKind::OpenApp => &inner.open_app_hotkey,
    }
}

pub(super) fn action_hotkey_binding(
    inner: &Arc<Inner>,
    kind: ActionHotkeyKind,
) -> Option<crate::types::ShortcutBinding> {
    let prefs = inner.prefs.get();
    match kind {
        ActionHotkeyKind::OpenApp => prefs.open_app_hotkey,
    }
}

pub(super) fn is_modifier_only_shortcut(binding: &crate::types::ShortcutBinding) -> bool {
    binding.modifiers.is_empty()
        && (binding.primary.eq_ignore_ascii_case("shift")
            || crate::shortcut_binding::legacy_modifier_trigger(binding).is_some())
}

pub(super) fn is_unconfigured_shortcut(binding: &crate::types::ShortcutBinding) -> bool {
    binding.primary.trim().is_empty()
}

pub(super) fn action_hotkey_bridge_thread_name(kind: ActionHotkeyKind) -> &'static str {
    match kind {
        ActionHotkeyKind::OpenApp => "openless-open-app-hotkey-bridge",
    }
}

pub(super) fn is_builtin_translation_shift(binding: &crate::types::ShortcutBinding) -> bool {
    binding.modifiers.is_empty() && binding.primary.eq_ignore_ascii_case("shift")
}

/// Linux: 从 prefs 读取自定义组合键，同步到 fcitx5 插件。
#[cfg(target_os = "linux")]
pub(super) fn custom_dictation_key_string(inner: &Arc<Inner>) -> Option<String> {
    let prefs = inner.prefs.get();
    let key_string = crate::linux_fcitx::binding_to_fcitx_key_string(&prefs.dictation_hotkey);
    if key_string.is_empty() {
        None
    } else {
        Some(key_string)
    }
}

#[cfg(target_os = "linux")]
pub(super) fn sync_custom_dictation_to_plugin(inner: &Arc<Inner>) {
    let prefs = inner.prefs.get();
    let dictation = &prefs.dictation_hotkey;
    let key_string = crate::linux_fcitx::binding_to_fcitx_key_string(dictation);
    if key_string.is_empty() {
        return;
    }
    match crate::linux_fcitx::set_custom_dictation_trigger(&key_string) {
        Ok(()) => log::info!(
            "[fcitx] Synced custom dictation trigger '{}' to plugin",
            key_string
        ),
        Err(e) => log::warn!("[fcitx] Failed to sync custom dictation trigger: {e}"),
    }
}

pub(super) fn modifier_shortcut_triggers(
    inner: &Arc<Inner>,
) -> (
    Option<crate::types::HotkeyTrigger>,
    Option<crate::types::HotkeyTrigger>,
) {
    let prefs = inner.prefs.get();
    // QA 已移除（批次 4）：槽位恒 None。签名保留两个槽供 Linux fcitx 同步编译。
    let qa_trigger = None;
    let translation_trigger = if is_builtin_translation_shift(&prefs.translation_hotkey) {
        None
    } else {
        crate::shortcut_binding::legacy_modifier_trigger(&prefs.translation_hotkey)
    };
    (qa_trigger, translation_trigger)
}

pub(super) fn mark_translation_modifier_seen(inner: &Arc<Inner>) {
    let phase = inner.state.lock().phase;
    if matches!(phase, SessionPhase::Starting | SessionPhase::Listening) {
        inner
            .translation_modifier_seen
            .store(true, Ordering::SeqCst);
        log::info!("[coord] translation modifier seen during {phase:?}");
    }
}

pub(super) fn hotkey_bridge_loop(inner: Arc<Inner>, rx: mpsc::Receiver<HotkeyEvent>) {
    while let Ok(evt) = rx.recv() {
        if inner.shortcut_recording_active.load(Ordering::SeqCst) {
            continue;
        }
        let inner_cloned = Arc::clone(&inner);
        match evt {
            // P0 #468/#475: Pressed/Released 必须串行处理，否则在 Windows 上 WH_KEYBOARD_LL
            // 边沿间隔微秒级 → 两个独立 spawn 的 task 被 work-stealing 调度器并行执行 →
            // `hotkey_trigger_held` latch 翻转顺序错乱 → 下次按键被静默吞掉
            // (UI 关不掉 / 录音停不下来)。改为 bridge 线程内 block_on 顺序 await，
            // recv 的 FIFO 顺序就是 handler 执行顺序。
            // 注意：handle_pressed_edge / handle_released_edge 内部走 .await（含网络
            // 握手），会暂时阻塞本 bridge 线程；Hold 模式短按时 Released 会排队在 channel
            // 里直到 begin_session 完成，但 SessionPhase::Starting 已经有
            // request_stop_during_starting 兜底，begin_session 完成进 Listening 后
            // bridge 立刻 recv Released → end_session，行为正确，仅有短暂 stop 延迟。
            HotkeyEvent::Pressed { at, press_id } => {
                crate::block_on(async {
                    handle_pressed_edge(&inner_cloned, at, press_id).await;
                });
            }
            HotkeyEvent::Released { at } => {
                crate::block_on(async {
                    handle_released_edge(&inner_cloned, at).await;
                });
            }
            // Esc 取消与组合键撤销都不在此枚举里：分别走 esc_cancel_bridge_loop /
            // combo_abort_bridge_loop，避免被上面 Released → end_session /
            // Pressed → begin_session 的同步流程堵在队列里（见各自函数注释）。
            HotkeyEvent::TranslationModifierPressed => {
                let translation_hotkey = inner_cloned.prefs.get().translation_hotkey;
                if is_builtin_translation_shift(&translation_hotkey)
                    || crate::shortcut_binding::legacy_modifier_trigger(&translation_hotkey)
                        .is_some()
                {
                    mark_translation_modifier_seen(&inner_cloned);
                }
            }
            // QA 已移除（批次 4）：事件槽保留供 Linux fcitx 插件同步编译，收到即忽略。
            HotkeyEvent::QaShortcutPressed => {}
        }
    }
}

#[allow(dead_code)] // P1 快捷键录制/重置路径接回
pub(super) fn reset_shortcut_held_state(inner: &Arc<Inner>) {
    inner.hotkey_trigger_held.store(false, Ordering::SeqCst);
    if let Some(monitor) = inner.hotkey.lock().as_ref() {
        monitor.reset_held_state();
    }
    let prefs = inner.prefs.get();
    if !is_builtin_translation_shift(&prefs.translation_hotkey)
        && crate::shortcut_binding::legacy_modifier_trigger(&prefs.translation_hotkey).is_none()
    {
        if let Some(monitor) = inner.translation_hotkey.lock().as_ref() {
            if let Err(e) = monitor.update_binding(prefs.translation_hotkey.clone()) {
                log::warn!("[coord] reset translation hotkey latch failed: {e}");
            }
        }
    }
    if let Some(open_app) = prefs.open_app_hotkey.as_ref() {
        if !is_modifier_only_shortcut(open_app) {
            if let Some(monitor) = inner.open_app_hotkey.lock().as_ref() {
                if let Err(e) = monitor.update_binding(open_app.clone()) {
                    log::warn!("[coord] reset open-app hotkey latch failed: {e}");
                }
            }
        }
    }
}

#[allow(dead_code)] // P1 窗口热键（app 热键）接回
pub(super) async fn handle_window_hotkey_event(
    inner: &Arc<Inner>,
    event_type: String,
    key: String,
    code: String,
    repeat: bool,
) -> Result<(), String> {
    if inner.shortcut_recording_active.load(Ordering::SeqCst) {
        return Ok(());
    }
    if event_type == "keydown" && key == "Escape" {
        cancel_session(inner);
        return Ok(());
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (inner, event_type, key, code, repeat);
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        if !window_hotkey_fallback_enabled() {
            if event_type == "keydown" && !repeat {
                log::info!(
                    "[window-hotkey] ignored because Windows lifecycle owner is the low-level hook"
                );
            }
            return Ok(());
        }

        let Some(trigger) =
            crate::shortcut_binding::legacy_modifier_trigger(&inner.prefs.get().dictation_hotkey)
        else {
            return Ok(());
        };
        if !window_key_matches_trigger(trigger, &key, &code) {
            return Ok(());
        }

        match event_type.as_str() {
            "keydown" => {
                if repeat {
                    return Ok(());
                }
                log::info!(
                    "[window-hotkey] pressed trigger={trigger:?} code={code} repeat={repeat}"
                );
                handle_pressed_edge(inner, std::time::Instant::now(), 0).await;
            }
            "keyup" => {
                log::info!("[window-hotkey] released trigger={trigger:?} code={code}");
                handle_released_edge(inner, std::time::Instant::now()).await;
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(any(target_os = "windows", test))]
pub(super) fn window_hotkey_fallback_enabled() -> bool {
    crate::types::HotkeyCapability::current().explicit_fallback_available
}

#[cfg(any(target_os = "windows", test))]
pub(super) fn window_key_matches_trigger(
    trigger: crate::types::HotkeyTrigger,
    key: &str,
    code: &str,
) -> bool {
    use crate::types::HotkeyTrigger;

    match trigger {
        HotkeyTrigger::RightControl => key == "Control" && code == "ControlRight",
        HotkeyTrigger::LeftControl => key == "Control" && code == "ControlLeft",
        HotkeyTrigger::RightOption | HotkeyTrigger::RightAlt => {
            (key == "Alt" || key == "AltGraph") && code == "AltRight"
        }
        HotkeyTrigger::LeftOption => (key == "Alt" || key == "AltGraph") && code == "AltLeft",
        HotkeyTrigger::RightCommand => key == "Meta" && code == "MetaRight",
        HotkeyTrigger::LeftCommand => key == "Meta" && code == "MetaLeft",
        HotkeyTrigger::LeftShift => key == "Shift" && code == "ShiftLeft",
        HotkeyTrigger::RightShift => key == "Shift" && code == "ShiftRight",
        HotkeyTrigger::Fn => key == "Control" && code == "ControlRight",
        // MediaPlayPause 走 WH_KEYBOARD_LL，不走 window hotkey fallback
        HotkeyTrigger::MediaPlayPause => false,
        // Custom 走 global-hotkey crate，不走 window hotkey fallback
        HotkeyTrigger::Custom => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 轮询 `inner.state.cancelled` 直到满足条件，超时返回 false。
    fn wait_until(mut cond: impl FnMut() -> bool, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    /// 构造一个处于 Processing 阶段、cancelled=false 的 Coordinator。
    fn coordinator_in_processing() -> Coordinator {
        let coordinator = Coordinator::new();
        let mut state = coordinator.inner.state.lock();
        state.phase = SessionPhase::Processing;
        state.cancelled = false;
        drop(state);
        coordinator
    }

    /// 后台运行 esc_cancel_bridge_loop，返回 sender 与 join handle。
    fn spawn_loop(inner: &Arc<Inner>) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
        let (tx, rx) = mpsc::channel::<()>();
        let bridge_inner = Arc::clone(inner);
        let handle = std::thread::spawn(move || esc_cancel_bridge_loop(bridge_inner, rx));
        (tx, handle)
    }

    #[test]
    fn esc_cancel_bridge_sets_cancelled_during_processing() {
        let coordinator = coordinator_in_processing();
        let (tx, handle) = spawn_loop(&coordinator.inner);

        tx.send(()).unwrap();
        assert!(
            wait_until(
                || coordinator.inner.state.lock().cancelled,
                std::time::Duration::from_secs(2)
            ),
            "取消信号应置 cancelled 旗标"
        );
        // #798 语义：Processing 阶段保持 phase=Processing，由 end_session 自行收尾。
        assert_eq!(
            coordinator.inner.state.lock().phase,
            SessionPhase::Processing
        );

        drop(tx);
        handle.join().unwrap();
    }

    #[test]
    fn esc_cancel_bridge_skips_while_shortcut_recording_active() {
        let coordinator = coordinator_in_processing();
        let (tx, handle) = spawn_loop(&coordinator.inner);

        coordinator.set_shortcut_recording_active(true);
        tx.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(120));
        assert!(
            !coordinator.inner.state.lock().cancelled,
            "录制快捷键期间按 Esc 应被忽略"
        );

        // 录制结束后 Esc 恢复生效。
        coordinator.set_shortcut_recording_active(false);
        tx.send(()).unwrap();
        assert!(
            wait_until(
                || coordinator.inner.state.lock().cancelled,
                std::time::Duration::from_secs(2)
            ),
            "录制结束后取消信号应恢复生效"
        );

        drop(tx);
        handle.join().unwrap();
    }

    #[test]
    fn esc_cancel_bridge_is_idempotent_on_repeat_signals() {
        let coordinator = coordinator_in_processing();
        let (tx, handle) = spawn_loop(&coordinator.inner);

        for _ in 0..3 {
            tx.send(()).unwrap();
        }
        assert!(
            wait_until(
                || coordinator.inner.state.lock().cancelled,
                std::time::Duration::from_secs(2)
            ),
            "首个取消信号应置 cancelled 旗标"
        );
        // 连按 Esc / 双通道重复触发时 cancel_session 幂等：不 panic、状态不回写。
        tx.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(120));
        assert_eq!(
            coordinator.inner.state.lock().phase,
            SessionPhase::Processing
        );

        drop(tx);
        handle.join().unwrap();
    }
}
