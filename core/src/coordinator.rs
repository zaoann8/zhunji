#![cfg_attr(
    target_os = "linux",
    allow(dead_code, unused_imports, unused_variables)
)]
//! Dictation coordinator.
//!
//! Mirrors the Swift `DictationCoordinator` state machine. Single owner of
//! session state. Receives hotkey edges, drives recorder + ASR + polish +
//! insertion, persists history, emits `capsule:state` events to the capsule
//! window.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use ferrous_opencc::{config::BuiltinConfig, OpenCC};
use parking_lot::Mutex;
#[cfg(any(target_os = "windows", test))]
use uuid::Uuid;

use crate::asr::RawTranscript;
use crate::combo_hotkey::{ComboHotkeyError, ComboHotkeyEvent, ComboHotkeyMonitor};
use crate::coordinator_state::{
    begin_cancel_session_state, begin_recording_abort_before_restore, begin_session_state,
    finish_cancel_session_state, finish_starting_session_state, publish_abort_idle_after_restore,
    start_processing_if_listening, startup_race_status, BeginOutcome, SessionId, SessionPhase,
    SessionState, StartupRaceStatus,
};
use crate::hotkey::{HotkeyEvent, HotkeyMonitor};
use crate::insertion::TextInserter;
use crate::persistence::{ActivityStore, CredentialsVault, HistoryStore, PreferencesStore};

use crate::recorder::{Recorder, RecorderError};
#[cfg(target_os = "windows")]
use crate::types::PasteShortcut;
use crate::types::{
    CapsulePayload, CapsuleState, ChineseScriptPreference, DictationSession, HotkeyCapability,
    HotkeyStatus, HotkeyStatusState, InsertStatus, PolishMode,
};
#[cfg(target_os = "windows")]
use crate::windows_ime_ipc::ImeSubmitTarget;
#[cfg(target_os = "windows")]
use crate::windows_ime_session::{PreparedWindowsImeSession, WindowsImeSessionController};

mod asr_wiring;
mod capsule_focus;
mod dictation;
mod hotkey_loops;
mod resources;
use asr_wiring::*;
use capsule_focus::*;
use hotkey_loops::*;

#[cfg(test)]
use dictation::dictation_error_code;
use dictation::{
    begin_session, cancel_session, end_session, handle_pressed_edge, handle_released_edge,
    handle_trigger_combined, request_stop_during_starting,
};
#[cfg(any(debug_assertions, test))]
use dictation::{handle_pressed, handle_released};
#[cfg(test)]
use resources::discard_startup_resources_for_session;
use resources::{cancel_active_asr, SessionResource, SharedRecordingMuteState};

/// 给 #470 诊断日志用的 capsule 状态短名。显式枚举每个变体到 &'static str，
/// 不走 `Debug` —— 哪天 CapsuleState 加了 `String` 字段，`:?` 会把 ASR / polish
/// 内容意外灌进日志（pr_agent 提的 forward-looking 隐患）；这里只输出状态名。
fn capsule_state_log_name(state: CapsuleState) -> &'static str {
    match state {
        CapsuleState::Idle => "idle",
        CapsuleState::Recording => "recording",
        CapsuleState::Transcribing => "transcribing",
        CapsuleState::Polishing => "polishing",
        CapsuleState::Done => "done",
        CapsuleState::Cancelled => "cancelled",
        CapsuleState::Error => "error",
    }
}

#[derive(Clone)]
enum ActiveAsr {
    /// 豆包 IME 免费通道（无凭据，自动注册）。
    Doubao(Arc<crate::asr::DoubaoImeASR>),
    /// Grok STT（worker-search 中转，非流式，需 endpoint + apiKey 凭据）。
    GrokStt(Arc<crate::asr::GrokSttASR>),
}

fn asr_transcribe_uses_global_timeout(_asr: &ActiveAsr) -> bool {
    true
}

/// 豆包引擎会话状态（概览页状态卡）。
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub ok: bool,
    pub error: Option<String>,
}

/// 实时上屏状态：录音中 partial 结果直接输入光标处，结束时替换为最终文本。
struct LiveInsert {
    enabled: AtomicBool,
    prev_len: AtomicUsize,
    /// 注入前切换到的 ABC 输入源（结束时恢复用户原输入法）。
    prev_ime: Mutex<Option<crate::unicode_keystroke::PreviousInputSource>>,
}

impl LiveInsert {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(false),
            prev_len: AtomicUsize::new(0),
            prev_ime: Mutex::new(None),
        })
    }
}

#[derive(Clone)]
pub struct Coordinator {
    inner: Arc<Inner>,
}

struct Inner {
    history: HistoryStore,
    /// 每日活动计数（热力图数据源），与 history 的保留策略解耦。
    activity: ActivityStore,
    prefs: PreferencesStore,
    inserter: TextInserter,
    #[cfg(target_os = "windows")]
    windows_ime: WindowsImeSessionController,
    #[cfg(target_os = "windows")]
    prepared_windows_ime_session: Arc<Mutex<Vec<PreparedWindowsImeSessionSlot>>>,
    state: Mutex<SessionState>,
    asr: Mutex<Option<SessionResource<ActiveAsr>>>,
    /// 与 `asr` 同生命周期的构建时快照：本次会话实际构建的 (provider, model)。
    /// store_asr_for_session 一并写入，end_session 取走落 history——比事后重读
    /// 全局设置可靠：会话中途切 provider/model 不会污染归因（PR #826 review）。
    asr_label: Mutex<Option<SessionResource<AsrCallLabel>>>,
    /// 常驻豆包引擎实例（预热 + 跨会话复用，避免每轮重新注册/token/连接）。
    doubao: Arc<crate::asr::DoubaoImeASR>,
    /// 常驻 Grok STT 实例（跨会话复用；非流式无预热连接，凭据在会话开始时读取）。
    grok_stt: Arc<crate::asr::GrokSttASR>,
    /// 实时上屏状态（partial → 光标处；结束替换）。
    live_insert: Arc<LiveInsert>,
    /// 豆包引擎上次会话结果（概览页状态卡）。
    last_engine_ok: AtomicBool,
    last_engine_error: Mutex<Option<String>>,
    recorder: Mutex<Option<SessionResource<Recorder>>>,
    /// 当前 dictation / QA session 的 wav 归档是否真的被写到磁盘上。
    /// 由 Recorder::start 返回值 (archive_active) 写入；history.append 路径读取，
    /// 决定 DictationSession.has_audio_recording 字段。比单纯读 prefs.record_audio_for_debug
    /// 更准确：用户开了开关但路径无法创建（权限 / 磁盘满）也算 false。
    audio_archive_active: AtomicBool,
    recording_mute: Mutex<SharedRecordingMuteState>,
    hotkey: Mutex<Option<HotkeyMonitor>>,
    hotkey_status: Mutex<HotkeyStatus>,
    hotkey_trigger_held: AtomicBool,
    /// 当前主听写热键按下的代次。组合键撤销通道使用同一代次，避免迟到事件
    /// 误取消下一次按下开启的会话。
    hotkey_press_generation: AtomicU64,
    /// 当前代次是否真的开出了会话；0 表示没有可撤销的会话。
    hotkey_press_began_session: AtomicU64,
    /// 组合键事件可能先于 Pressed 事件抵达协调器，暂存其代次供仲裁窗口消费。
    /// 用队列而不是单个槽，避免主 bridge 忙于上一轮仲裁时覆盖连续按下的事件。
    hotkey_combo_pending_presses: Mutex<std::collections::VecDeque<u64>>,
    /// 防抖时间戳：handle_pressed_edge 入口检查与本字段的距离，< 250ms 的边沿直接
    /// 丢弃（误触双击 / 微动开关回弹 / 用户连点过快造成的空转写报错）。
    /// 与 `hotkey_trigger_held` 互补 —— held 防 press-without-release，本字段防
    /// press-release-press 三连过快。
    last_hotkey_dispatch_at: Mutex<Option<std::time::Instant>>,
    /// Auto 模式下这次会话「按下」的事件时刻。松手时用按下/松开的事件时间戳差值
    /// 判定短按（Toggle 锁存）还是长按（Hold 松手即停）。见 dictation.rs 的
    /// AUTO_HOLD_THRESHOLD。
    hotkey_press_at: Mutex<Option<std::time::Instant>>,
    /// 会话收尾（成功 / 取消 / 失败）将 phase 设为 Idle 时记录的时间戳 + POST_SESSION_COOLDOWN_MS。
    /// handle_pressed 在 (Toggle, Idle) 分支检查此字段：未过期则忽略该次按键，防止胶囊离场
    /// 动画期间误激活新听写（issue #545）；也让识别中排队的热键按下在收尾后一律静默丢弃（issue #856）。
    session_cooldown_until: Mutex<Option<std::time::Instant>>,
    shortcut_recording_active: AtomicBool,
    /// 自定义组合键监听器（global-hotkey crate）。当 `prefs.hotkey.trigger == Custom` 时
    /// 代替 modifier-only 的 hotkey monitor。`None` 表示不使用自定义组合键或还没成功安装。
    combo_hotkey: Mutex<Option<ComboHotkeyMonitor>>,
    side_aware_combo: Mutex<Option<crate::side_aware_combo::SideAwareComboMonitor>>,
    translation_hotkey: Mutex<Option<ComboHotkeyMonitor>>,
    open_app_hotkey: Mutex<Option<ComboHotkeyMonitor>>,
    /// 翻译模式触发标志。每次 begin_session 重置为 false；hotkey 监听器在
    /// Listening / Starting 阶段看到 Shift down 边沿时 set true。
    /// end_session 在调 polish/translate 前读这个 flag + translation_target_language
    /// 决定走哪条管线。详见 issue #4。
    translation_modifier_seen: AtomicBool,
    /// 最近一次 emit_capsule 下发的 state，纯内省/测试用途（在 app 句柄校验之前写入，
    /// 因此无 GUI 的测试环境也能断言「按下热键 → 弹了哪种胶囊」）。写入是单次廉价
    /// 加锁，对 ~30Hz 录音回调可忽略。
    last_capsule_state: Mutex<Option<CapsuleState>>,
    /// 每次 capsule payload 递增。选区润色的终态自动隐藏会带上该代数，防止旧 timer
    /// 覆盖新的选区润色/语音/QA 可见状态。
    capsule_event_epoch: AtomicU64,
    /// 将 capsule 事件与自动隐藏线性化。这样一个旧 timer 要么在新的 payload 之前收起
    /// 旧提示，要么发现代数已改变直接放弃，绝不会在新会话之后补发 Idle。
    capsule_event_lock: Mutex<()>,
    /// 预备态标志：按下热键即"乐观显示"胶囊（带入场动画），此时麦克风还在 cpal
    /// init 窗口内、没有第一帧 PCM。为 true 时 emit_capsule 把 Recording payload 的
    /// `warming` 打成 true（前端渲染"待命"光效）；`level_handler` 首次触发（PCM 真的
    /// 流入）后置 false，光条"点亮"进入正式录音。begin_session 每次入场重置为 true。
    capsule_warming: AtomicBool,
    /// Coordinator 退出信号。各 hotkey supervisor loop 在每轮重试 sleep 之前会检查
    /// 此 flag；为 true 时 loop 立刻 return。生产场景里 process exit 一并 reap 所有
    /// supervisor 线程，但 integration test 和未来 RunEvent::Exit 钩子需要这条
    /// 显式退出路径。审计 3.1.2。
    shutdown: AtomicBool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionHotkeyKind {
    OpenApp,
}

#[cfg(target_os = "windows")]
#[derive(Debug)]
struct PreparedWindowsImeSessionSlot {
    session_id: SessionId,
    prepared: PreparedWindowsImeSession,
}

/// 历史音频静默重试的 ASR 资源护栏。
///
/// 重试 future 被 select 丢弃时，局部 QaAsrStart 不会再经过正常的
/// end_session 收尾；这里用 Drop 补 cancel 和本地模型释放，尤其覆盖
/// spawn_blocking 已经开始运行的本地 ASR。
struct CancellableRetranscribeGuard {
    inner: Arc<Inner>,
    asr: Option<ActiveAsr>,
    session_id: SessionId,
}

impl CancellableRetranscribeGuard {
    fn new(inner: Arc<Inner>, asr: ActiveAsr, session_id: SessionId) -> Self {
        Self {
            inner,
            asr: Some(asr),
            session_id,
        }
    }

    fn disarm(mut self) {
        self.asr.take();
    }
}

impl Drop for CancellableRetranscribeGuard {
    fn drop(&mut self) {
        let Some(asr) = self.asr.take() else {
            return;
        };
        let asr_for_release = asr.clone();
        cancel_active_asr(asr);
        dictation::schedule_cancelled_asr_release(&self.inner, &asr_for_release, self.session_id);
    }
}

// P0 暂存（P1 设置页 / P2 概览页接回）：stop_*_hotkey_listener、
// update_*_hotkey_binding / try_update_translation_hotkey_binding、
// engine_status / EngineStatus。allow 只加在 impl 块上，随接回逐步移除。
#[allow(dead_code)]
impl Coordinator {
    pub fn new() -> Self {
        let history = HistoryStore::new().unwrap_or_else(|e| {
            log::error!("[coord] HistoryStore init failed: {e}; 降级为空历史记录");
            HistoryStore::new_fallback()
        });
        let prefs = PreferencesStore::new().unwrap_or_else(|e| {
                log::error!(
                    "[coord] PreferencesStore init failed: {e}; 降级为默认偏好设置"
                );
                PreferencesStore::new_fallback()
            });
        // 启动即同步系统代理开关（issue #869），让首个请求就按用户设置建客户端。
        crate::net::set_use_system_proxy(prefs.get().use_system_proxy);
        let activity = ActivityStore::load().unwrap_or_else(|e| {
            log::error!("[coord] ActivityStore init failed: {e}; 活动计数降级为内存态");
            ActivityStore::new_fallback()
        });

        // 实时转写回调：豆包 partial 结果 → 实时上屏 + partial-text 事件。
        let live_insert = LiveInsert::new();
        let live_for_cb = Arc::clone(&live_insert);
        let doubao = crate::asr::DoubaoImeASR::new(Some(Arc::new(move |text| {
            log::info!("[doubao] partial received: {text}");
            if live_for_cb.enabled.load(Ordering::SeqCst) {
                let prev = live_for_cb
                    .prev_len
                    .swap(text.chars().count(), Ordering::SeqCst);
                if prev > 0 {
                    let _ = crate::insertion::macos::delete_chars(prev);
                }
                let _ = crate::unicode_keystroke::type_unicode_chunk(&text);
            }
            crate::event_bus::emit("partial-text", &serde_json::json!({ "text": text }));
        })));

        Self {
            inner: Arc::new(Inner {
                history,
                activity,
                prefs,
                inserter: TextInserter::new(),
                state: Mutex::new(SessionState::default()),
                asr: Mutex::new(None),
                asr_label: Mutex::new(None),
                live_insert,
                doubao,
                grok_stt: crate::asr::GrokSttASR::new(),
                last_engine_ok: AtomicBool::new(true),
                last_engine_error: Mutex::new(None),
                recorder: Mutex::new(None),
                audio_archive_active: AtomicBool::new(false),
                recording_mute: Mutex::new(SharedRecordingMuteState::new()),
                hotkey: Mutex::new(None),
                hotkey_status: Mutex::new(HotkeyStatus::default()),
                hotkey_trigger_held: AtomicBool::new(false),
                hotkey_press_generation: AtomicU64::new(0),
                hotkey_press_began_session: AtomicU64::new(0),
                hotkey_combo_pending_presses: Mutex::new(std::collections::VecDeque::new()),
                last_hotkey_dispatch_at: Mutex::new(None),
                hotkey_press_at: Mutex::new(None),
                session_cooldown_until: Mutex::new(None),
                shortcut_recording_active: AtomicBool::new(false),
                combo_hotkey: Mutex::new(None),
                side_aware_combo: Mutex::new(None),
                translation_hotkey: Mutex::new(None),
                open_app_hotkey: Mutex::new(None),
                translation_modifier_seen: AtomicBool::new(false),
                last_capsule_state: Mutex::new(None),
                capsule_event_epoch: AtomicU64::new(0),
                capsule_event_lock: Mutex::new(()),
                capsule_warming: AtomicBool::new(false),
                shutdown: AtomicBool::new(false),
            }),
        }
    }

    /// 豆包引擎版配置迁移（启动时调用一次；不放在 new() 里避免测试污染用户配置文件）。
    /// 只兜底未知/空 provider（默认豆包），不覆盖用户已选择的 grok_stt 等合法引擎。
    pub fn migrate_doubao_prefs(&self) {
        let mut p = self.inner.prefs.get();
        let mut changed = false;
        let known = [
            crate::asr::doubao::PROVIDER_ID,
            crate::asr::grok_stt::PROVIDER_ID,
        ];
        // 自定义供应商（providers.json 中已存在）也是合法引擎，不能被兜底重置回豆包
        let is_known_provider = known.contains(&p.active_asr_provider.as_str())
            || crate::providers::list_providers()
                .ok()
                .is_some_and(|list| list.iter().any(|prov| prov.id == p.active_asr_provider));
        if !is_known_provider {
            p.active_asr_provider = crate::asr::doubao::PROVIDER_ID.into();
            changed = true;
        }
        if p.default_mode != crate::types::PolishMode::Raw {
            p.default_mode = crate::types::PolishMode::Raw;
            changed = true;
        }
        let provider = p.active_asr_provider.clone();
        if changed {
            let _ = self.inner.prefs.set(p);
        }
        let _ = self.sync_active_asr_provider_to_vault(&provider);
    }

    /// 后台预热引擎连接，热键按下时只剩握手/校验。
    /// 豆包：注册/token/WS 提前建立；Grok STT：建连 + DPoP session 预热（常驻保活）。
    /// 判断与 build_qa_asr_start 的 is_third_party 对齐：自定义第三方供应商也走 GrokSttASR。
    pub fn warmup_engine(&self) {
        let active = CredentialsVault::get_active_asr();
        if crate::asr::grok_stt::is_grok_stt(&active) || find_provider(&active).is_some() {
            let asr = Arc::clone(&self.inner.grok_stt);
            // 任意线程可调用：zhunji_init 主线程 / 热键线程都会走到这里。
            crate::runtime().spawn(async move {
                asr.warmup().await;
            });
        } else {
            let asr = Arc::clone(&self.inner.doubao);
            crate::runtime().spawn(async move {
                asr.warmup().await;
            });
        }
    }

    pub fn engine_status(&self) -> EngineStatus {
        EngineStatus {
            ok: self.inner.last_engine_ok.load(Ordering::SeqCst),
            error: self.inner.last_engine_error.lock().clone(),
        }
    }

    /// 让所有 hotkey supervisor loop（dictation / combo / translation /
    /// switch_style / open_app）在下一轮 sleep / poll 后退出。生产场景下进程退出
    /// 一并 reap 所有线程，但 integration test 和未来 RunEvent::Exit 钩子需要
    /// 显式退出路径。审计 3.1.2。
    #[allow(dead_code)]
    pub fn request_shutdown(&self) {
        self.inner.shutdown.store(true, Ordering::SeqCst);
    }

    pub fn start_hotkey_listener(&self) {
        // 起一个守护线程，反复尝试安装 hotkey hook。Accessibility 一被授予就立即生效，
        // 用户不需要手动重启 OpenLess。
        let inner = Arc::clone(&self.inner);
        std::thread::Builder::new()
            .name("openless-hotkey-supervisor".into())
            .spawn(move || hotkey_supervisor_loop(inner))
            .ok();
    }

    pub fn stop_hotkey_listener(&self) {
        self.inner.hotkey.lock().take();
    }

    /// 启动自定义组合键监听器。当 `prefs.hotkey.trigger == Custom` 时，
    /// 代替 modifier-only 的 hotkey monitor。
    pub fn start_combo_hotkey_listener(&self) {
        let inner = Arc::clone(&self.inner);
        std::thread::Builder::new()
            .name("openless-combo-hotkey-supervisor".into())
            .spawn(move || combo_hotkey_supervisor_loop(inner))
            .ok();
    }

    pub fn stop_combo_hotkey_listener(&self) {
        take_combo_hotkey_on_main_thread(&self.inner);
    }

    pub fn start_translation_hotkey_listener(&self) {
        let inner = Arc::clone(&self.inner);
        std::thread::Builder::new()
            .name("openless-translation-hotkey-supervisor".into())
            .spawn(move || translation_hotkey_supervisor_loop(inner))
            .ok();
    }

    pub fn stop_translation_hotkey_listener(&self) {
        take_translation_hotkey_on_main_thread(&self.inner);
    }

    pub fn start_open_app_hotkey_listener(&self) {
        let inner = Arc::clone(&self.inner);
        std::thread::Builder::new()
            .name("openless-open-app-hotkey-supervisor".into())
            .spawn(move || action_hotkey_supervisor_loop(inner, ActionHotkeyKind::OpenApp))
            .ok();
    }

    pub fn stop_open_app_hotkey_listener(&self) {
        take_action_hotkey_on_main_thread(&self.inner, ActionHotkeyKind::OpenApp);
    }

    /// 用户在设置里改了自定义组合键时调用。
    pub fn update_combo_hotkey_binding(&self) {
        let prefs = self.inner.prefs.get();
        if crate::shortcut_binding::legacy_modifier_trigger(&prefs.dictation_hotkey).is_some() {
            take_combo_hotkey_on_main_thread(&self.inner);
            self.inner.side_aware_combo.lock().take();
            log::info!("[coord] combo hotkey 已关闭（modifier-only）");
            return;
        }
        let binding = prefs.dictation_hotkey.clone();
        if is_unconfigured_shortcut(&binding) {
            take_combo_hotkey_on_main_thread(&self.inner);
            self.inner.side_aware_combo.lock().take();
            log::info!("[coord] combo hotkey 已关闭（无绑定）");
            return;
        }

        if crate::shortcut_binding::binding_requires_side_aware_hook(&binding) {
            take_combo_hotkey_on_main_thread(&self.inner);
            self.inner.side_aware_combo.lock().take();
            let (tx, rx) = mpsc::channel::<ComboHotkeyEvent>();
            match crate::side_aware_combo::SideAwareComboMonitor::start(binding, tx) {
                Ok(monitor) => {
                    *self.inner.side_aware_combo.lock() = Some(monitor);
                    let bridge_inner = Arc::clone(&self.inner);
                    std::thread::Builder::new()
                        .name("openless-side-combo-bridge".into())
                        .spawn(move || combo_hotkey_bridge_loop(bridge_inner, rx))
                        .ok();
                    log::info!("[coord] side-aware combo hotkey listener installed (via update)");
                }
                Err(e) => {
                    log::warn!("[coord] update side-aware combo binding 失败: {e}");
                }
            }
            return;
        }

        self.inner.side_aware_combo.lock().take();
        // 原版经 AppHandle.run_on_main_thread 调度到 AppKit 主线程（tauri 插件要求）；
        // P0 直接调用：global-hotkey 的 macOS 实现是 Carbon 事件循环 + 自建 run loop，
        // 不依赖 AppKit 主线程。
        let inner_clone = Arc::clone(&self.inner);
        {
            // 具名 guard 块：锁在块末即 drop（避免 if-let scrutinee 临时值的
            // Drop 延后到函数尾，见 E0597）。
            let slot = inner_clone.combo_hotkey.lock();
            if let Some(monitor) = slot.as_ref() {
                if let Err(e) = monitor.update_binding(binding.clone()) {
                    log::warn!("[coord] update combo hotkey binding 失败: {e}");
                }
                return;
            }
        }
        let (tx, rx) = mpsc::channel::<ComboHotkeyEvent>();
        match ComboHotkeyMonitor::start(binding, tx) {
            Ok(monitor) => {
                *inner_clone.combo_hotkey.lock() = Some(monitor);
                log::info!("[coord] combo hotkey listener installed (via update)");
                let bridge_inner = Arc::clone(&inner_clone);
                std::thread::Builder::new()
                    .name("openless-combo-hotkey-bridge".into())
                    .spawn(move || combo_hotkey_bridge_loop(bridge_inner, rx))
                    .ok();
                #[cfg(target_os = "linux")]
                sync_custom_dictation_to_plugin(&inner_clone);
            }
            Err(e) => {
                log::warn!("[coord] update combo hotkey binding 失败: {e}");
            }
        }
    }

    pub fn update_translation_hotkey_binding(&self) {
        if let Err(e) = self.try_update_translation_hotkey_binding() {
            log::warn!("[coord] update translation hotkey binding 失败: {e}");
        }
    }

    /// P1.4 设置页热键修改：按当前 prefs 立即重注册听写热键（CGEventTap 换绑定，
    /// 无需重建监听线程）。原版 update_hotkey_binding（coordinator.rs:712）语义：
    /// 单修饰键 → 关闭 combo 监听；自定义组合键 → 重建/更新 combo 监听器。
    pub fn update_dictation_hotkey_binding(&self) -> Result<(), String> {
        let prefs = self.inner.prefs.get();
        let trigger = crate::shortcut_binding::legacy_modifier_trigger(&prefs.dictation_hotkey)
            .unwrap_or(crate::types::HotkeyTrigger::Custom);
        if trigger == crate::types::HotkeyTrigger::Custom {
            self.update_combo_hotkey_binding();
        } else {
            take_combo_hotkey_on_main_thread(&self.inner);
            // 原版 legacy 分支只清 combo_hotkey；这里额外清 side_aware，
            // 避免「组合键 → 单修饰键」切换后旧监听器残留。
            self.inner.side_aware_combo.lock().take();
        }
        let binding = crate::types::HotkeyBinding {
            trigger,
            mode: prefs.hotkey.mode,
            keys: None,
        };
        let hotkey_guard = self.inner.hotkey.lock();
        let Some(monitor) = hotkey_guard.as_ref() else {
            return Err("dictation hotkey monitor 未初始化".into());
        };
        monitor.update_binding(binding);
        log::info!("[coord] dictation hotkey 已更新: {:?}", prefs.dictation_hotkey);
        Ok(())
    }

    pub fn try_update_translation_hotkey_binding(&self) -> Result<(), String> {
        let prefs = self.inner.prefs.get();
        if is_builtin_translation_shift(&prefs.translation_hotkey)
            || crate::shortcut_binding::legacy_modifier_trigger(&prefs.translation_hotkey).is_some()
        {
            take_translation_hotkey_on_main_thread(&self.inner);
            self.update_modifier_shortcut_bindings();
            log::info!("[coord] translation hotkey uses modifier-only listener");
            return Ok(());
        }
        self.update_modifier_shortcut_bindings();
        // 原版经 AppHandle.run_on_main_thread 调度 + 同步等待结果；P0 直接调用
        // （见 update_combo_hotkey_binding 的说明）。
        update_translation_hotkey_on_main_thread(
            Arc::clone(&self.inner),
            prefs.translation_hotkey.clone(),
        )
        .map_err(|e| e.to_string())
    }

    pub fn update_open_app_hotkey_binding(&self) {
        self.update_action_hotkey_binding(ActionHotkeyKind::OpenApp);
    }

    fn update_action_hotkey_binding(&self, kind: ActionHotkeyKind) {
        // None = 用户主动停用：反注册全局键，立即生效。
        let Some(binding) = action_hotkey_binding(&self.inner, kind) else {
            take_action_hotkey_on_main_thread(&self.inner, kind);
            log::info!("[coord] action hotkey {kind:?} 已停用（用户清空）");
            return;
        };
        if is_modifier_only_shortcut(&binding) {
            take_action_hotkey_on_main_thread(&self.inner, kind);
            log::warn!("[coord] action hotkey {kind:?} 使用了不支持的 modifier-only 绑定，已关闭");
            return;
        }

        // 原版经 AppHandle.run_on_main_thread 调度；P0 直接调用（见
        // update_combo_hotkey_binding 的说明）。
        let inner_clone = Arc::clone(&self.inner);
        {
            // 具名 guard 块：锁在块末即 drop（避免 if-let scrutinee 临时值的
            // Drop 延后到函数尾，见 E0597）。
            let slot = action_hotkey_slot(&inner_clone, kind).lock();
            if let Some(monitor) = slot.as_ref() {
                if let Err(e) = monitor.update_binding(binding.clone()) {
                    log::warn!("[coord] update action hotkey {kind:?} binding 失败: {e}");
                }
                return;
            }
        }
        let (tx, rx) = mpsc::channel::<ComboHotkeyEvent>();
        match ComboHotkeyMonitor::start(binding, tx) {
            Ok(monitor) => {
                *action_hotkey_slot(&inner_clone, kind).lock() = Some(monitor);
                let bridge_inner = Arc::clone(&inner_clone);
                std::thread::Builder::new()
                    .name(action_hotkey_bridge_thread_name(kind).into())
                    .spawn(move || action_hotkey_bridge_loop(bridge_inner, rx, kind))
                    .ok();
            }
            Err(e) => log::warn!("[coord] update action hotkey {kind:?} binding 失败: {e}"),
        }
    }

    pub fn history(&self) -> &HistoryStore {
        &self.inner.history
    }

    pub fn activity(&self) -> &ActivityStore {
        &self.inner.activity
    }

    pub fn prefs(&self) -> &PreferencesStore {
        &self.inner.prefs
    }
    #[cfg(any(target_os = "windows", test))]
    pub fn sync_active_asr_provider_from_preferences(&self) -> Result<(), String> {
        let provider = self.inner.prefs.get().active_asr_provider;
        self.sync_active_asr_provider_to_vault(&provider)
    }
    pub fn sync_active_asr_provider_to_vault(&self, provider: &str) -> Result<(), String> {
        if CredentialsVault::get_active_asr() == provider {
            return Ok(());
        }
        CredentialsVault::set_active_asr_provider(provider).map_err(|e| e.to_string())
    }
    pub fn update_hotkey_binding(&self) {
        let prefs = self.inner.prefs.get();
        let dictation_trigger =
            crate::shortcut_binding::legacy_modifier_trigger(&prefs.dictation_hotkey);
        let binding = crate::types::HotkeyBinding {
            trigger: dictation_trigger.unwrap_or(crate::types::HotkeyTrigger::Custom),
            mode: prefs.hotkey.mode,
            keys: None,
        };
        if dictation_trigger.is_some() {
            take_combo_hotkey_on_main_thread(&self.inner);
        } else {
            self.update_combo_hotkey_binding();
        }
        self.ensure_modifier_hotkey_monitor(binding);
        self.update_modifier_shortcut_bindings();
    }

    fn ensure_modifier_hotkey_monitor(&self, binding: crate::types::HotkeyBinding) {
        if let Some(monitor) = self.inner.hotkey.lock().as_ref() {
            #[cfg(target_os = "linux")]
            let plugin_binding = binding.clone();
            monitor.update_binding(binding);
            #[cfg(target_os = "linux")]
            if plugin_binding.trigger == crate::types::HotkeyTrigger::Custom {
                sync_custom_dictation_to_plugin(&self.inner);
            } else {
                crate::linux_fcitx::sync_binding_to_plugin(&plugin_binding);
            }
            return;
        }
        let (tx, rx) = mpsc::channel::<HotkeyEvent>();
        #[cfg(target_os = "linux")]
        let (fcitx_tx, fcitx_binding) = (tx.clone(), binding.clone());
        let cancel_tx = spawn_esc_cancel_bridge(&self.inner);
        let combo_tx = spawn_combo_abort_bridge(&self.inner, handle_trigger_combined);
        #[cfg(target_os = "linux")]
        let combo_tx_for_fcitx = combo_tx.clone();
        match HotkeyMonitor::start(binding, tx, cancel_tx, combo_tx) {
            Ok(monitor) => {
                let adapter = monitor.kind();
                *self.inner.hotkey.lock() = Some(monitor);
                *self.inner.hotkey_status.lock() = HotkeyStatus {
                    adapter,
                    state: HotkeyStatusState::Installed,
                    message: Some(format!("{} 已安装", adapter.display_name())),
                    last_error: None,
                };
                let inner_clone = Arc::clone(&self.inner);
                std::thread::Builder::new()
                    .name("openless-hotkey-bridge".into())
                    .spawn(move || hotkey_bridge_loop(inner_clone, rx))
                    .ok();
                // Linux: 启动 fcitx5 插件信号监听作为热键源。
                #[cfg(target_os = "linux")]
                {
                    let (qa_trigger, translation_trigger) = modifier_shortcut_triggers(&self.inner);
                    let custom_key = custom_dictation_key_string(&self.inner);
                    crate::linux_fcitx::start_dictation_signal_listener(
                        fcitx_tx,
                        combo_tx_for_fcitx,
                        fcitx_binding.clone(),
                        qa_trigger,
                        translation_trigger,
                        custom_key,
                    );
                    if fcitx_binding.trigger == crate::types::HotkeyTrigger::Custom {
                        sync_custom_dictation_to_plugin(&self.inner);
                    } else {
                        crate::linux_fcitx::sync_binding_to_plugin(&fcitx_binding);
                    }
                }
            }
            Err(e) => {
                *self.inner.hotkey_status.lock() = HotkeyStatus {
                    adapter: HotkeyMonitor::capability().adapter,
                    state: HotkeyStatusState::Failed,
                    message: Some(e.message.clone()),
                    last_error: Some(e),
                };
            }
        }
    }

    pub fn update_modifier_shortcut_bindings(&self) {
        if let Some(monitor) = self.inner.hotkey.lock().as_ref() {
            let (qa_trigger, translation_trigger) = modifier_shortcut_triggers(&self.inner);
            monitor.update_modifier_shortcuts(qa_trigger, translation_trigger);
        }
    }

    pub fn hotkey_status(&self) -> HotkeyStatus {
        self.inner.hotkey_status.lock().clone()
    }

    pub fn hotkey_capability(&self) -> HotkeyCapability {
        HotkeyMonitor::capability()
    }

    pub async fn start_dictation(&self) -> Result<(), String> {
        begin_session(&self.inner).await
    }

    pub async fn stop_dictation(&self) -> Result<(), String> {
        if self.inner.state.lock().phase == SessionPhase::Starting {
            request_stop_during_starting(&self.inner, "manual stop");
            return Ok(());
        }
        end_session(&self.inner).await
    }

    pub fn cancel_dictation(&self) {
        cancel_session(&self.inner);
    }

    /// 返回当前听写阶段（read-only 快照），供 CLI 入口在 dispatch toggle 时决策。
    /// 与原热键边沿走的 `handle_pressed` 分支完全相同的判定逻辑：Idle → start，
    /// Listening → stop。可用于桌面快捷键 → CLI 转发的备用触发路径。
    pub fn dictation_phase_for_cli(&self) -> SessionPhase {
        self.inner.state.lock().phase
    }

    pub fn set_shortcut_recording_active(&self, active: bool) {
        self.inner
            .shortcut_recording_active
            .store(active, Ordering::SeqCst);
        if active {
            reset_shortcut_held_state(&self.inner);
        }
        log::info!("[coord] shortcut recording active={active}");
    }

    pub async fn handle_window_hotkey_event(
        &self,
        event_type: String,
        key: String,
        code: String,
        repeat: bool,
    ) -> Result<(), String> {
        handle_window_hotkey_event(&self.inner, event_type, key, code, repeat).await
    }

    #[cfg(any(debug_assertions, test))]
    pub async fn inject_hotkey_click_for_dev(&self) -> Result<(), String> {
        log::info!("[coord] dev hotkey injection started");
        handle_pressed(&self.inner, std::time::Instant::now(), 0).await;
        handle_released(&self.inner, std::time::Instant::now()).await;
        cancel_session(&self.inner);
        Ok(())
    }

    /// 返回 (转写文本, 本次实际构建的 ASR (provider, model) 快照)。快照供命令层把
    /// 「重转用了哪个模型」写回历史（构建时归因，PR #826 review）。
    pub async fn retranscribe_pcm(&self, pcm: Vec<u8>) -> Result<(String, AsrCallLabel), String> {
        self.retranscribe_pcm_inner(pcm, false, None).await
    }

    pub(super) async fn retranscribe_pcm_until_cancelled(
        &self,
        pcm: Vec<u8>,
    ) -> (Result<String, String>, Option<AsrCallLabel>) {
        // 自动静默重试会重新读取当前设置并构建一条全新的 ASR 会话，因此必须把这次
        // 实际构建的标签交还给调用方。即使请求最终失败，也保留“本次尝试了谁”，让
        // 彻底失败的历史不会退回首次会话的旧归因。
        let mut attempted_label = None;
        let result = self
            .retranscribe_pcm_inner(pcm, true, Some(&mut attempted_label))
            .await
            .map(|(text, _)| text);
        (result, attempted_label)
    }

    async fn retranscribe_pcm_inner(
        &self,
        pcm: Vec<u8>,
        cancel_on_drop: bool,
        attempted_label: Option<&mut Option<AsrCallLabel>>,
    ) -> Result<(String, AsrCallLabel), String> {
        let inner = &self.inner;
        let active_asr = CredentialsVault::get_active_asr();
        let (start, asr_call_label) = build_qa_asr_start(inner, &active_asr).await?;
        if let Some(label_slot) = attempted_label {
            *label_slot = Some(asr_call_label.clone());
        }
        let retry_guard = if cancel_on_drop {
            Some(CancellableRetranscribeGuard::new(
                Arc::clone(inner),
                start.active_asr(),
                inner.state.lock().session_id,
            ))
        } else {
            None
        };
        start.open_streaming_session().await?;
        let consumer = start.recorder_consumer();
        consumer.consume_pcm_chunk(&pcm);
        let timeout = std::time::Duration::from_secs(COORDINATOR_GLOBAL_TIMEOUT_SECS);
        let raw = match start.active_asr() {
            ActiveAsr::Doubao(asr) => {
                let outcome: Result<String, String> = async {
                    asr.send_last_frame().await.map_err(|e| e.to_string())?;
                    tokio::time::timeout(timeout, asr.await_final_result())
                        .await
                        .map_err(|_| "重新转录超时".to_string())?
                        .map_err(|e| e.to_string())
                }
                .await;
                // 重转录用的是独立 ASR 实例，用完即关 WS：不关的话服务端把「已结束但
                // 连接未关」的会话计入免费通道并发配额（5 路），历史重转 / 自动静默重试
                // 多跑几次就顶到 40200011。成功失败都要关。
                let duration_ms = asr.session_duration_ms();
                asr.close();
                RawTranscript {
                    text: outcome?,
                    duration_ms,
                }
            }
            ActiveAsr::GrokStt(asr) => {
                let outcome: Result<String, String> = async {
                    asr.send_last_frame().await.map_err(|e| e.to_string())?;
                    tokio::time::timeout(timeout, asr.await_final_result())
                        .await
                        .map_err(|_| "重新转录超时".to_string())?
                        .map_err(|e| e.to_string())
                }
                .await;
                let duration_ms = asr.session_duration_ms();
                asr.close();
                RawTranscript {
                    text: outcome?,
                    duration_ms,
                }
            }
        };
        if let Some(guard) = retry_guard {
            guard.disarm();
        }
        Ok((raw.text, asr_call_label))
    }
}

// ─────────────────────────── session lifecycle ───────────────────────────

#[cfg(target_os = "windows")]
fn store_prepared_windows_ime_session(
    slots: &mut Vec<PreparedWindowsImeSessionSlot>,
    session_id: SessionId,
    prepared: PreparedWindowsImeSession,
) {
    slots.retain(|slot| slot.session_id != session_id);
    slots.push(PreparedWindowsImeSessionSlot {
        session_id,
        prepared,
    });
}

#[cfg(target_os = "windows")]
fn take_matching_prepared_windows_ime_session(
    slots: &mut Vec<PreparedWindowsImeSessionSlot>,
    session_id: SessionId,
) -> Option<PreparedWindowsImeSession> {
    let index = slots
        .iter()
        .position(|slot| slot.session_id == session_id)?;
    Some(slots.remove(index).prepared)
}

#[cfg(target_os = "windows")]
fn take_current_prepared_windows_ime_session_for_restore(
    slots: &mut Vec<PreparedWindowsImeSessionSlot>,
    session_id: SessionId,
    current_session_id: SessionId,
) -> Option<PreparedWindowsImeSession> {
    let prepared = take_matching_prepared_windows_ime_session(slots, session_id)?;
    if current_session_id == session_id {
        Some(prepared)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn restore_prepared_windows_ime_session(inner: &Arc<Inner>, session_id: SessionId) {
    let state = inner.state.lock();
    let prepared = {
        let mut slot = inner.prepared_windows_ime_session.lock();
        take_current_prepared_windows_ime_session_for_restore(
            &mut slot,
            session_id,
            state.session_id,
        )
    };
    if let Some(prepared) = prepared {
        inner.windows_ime.restore_session(prepared);
    }
}

#[cfg(not(target_os = "windows"))]
fn restore_prepared_windows_ime_session(_inner: &Arc<Inner>, _session_id: SessionId) {}

#[cfg(target_os = "windows")]
async fn insert_with_windows_ime_first(
    inner: &Arc<Inner>,
    session_id: SessionId,
    polished: &str,
    restore_clipboard: bool,
    allow_non_tsf_insertion_fallback: bool,
    paste_shortcut: PasteShortcut,
    ime_target: Option<ImeSubmitTarget>,
) -> InsertStatus {
    let prepared = {
        let mut slot = inner.prepared_windows_ime_session.lock();
        take_matching_prepared_windows_ime_session(&mut slot, session_id)
    };
    let Some(prepared) = prepared else {
        log::warn!("[windows-ime] no prepared TSF session for this dictation");
        if should_try_non_tsf_insertion_fallback(
            allow_non_tsf_insertion_fallback,
            InsertStatus::Failed,
        ) {
            return insert_via_non_tsf_fallback(inner, polished, restore_clipboard, paste_shortcut);
        }
        log::warn!("[windows-ime] non-TSF insertion fallback is disabled; failing insert");
        return InsertStatus::Failed;
    };

    let request = crate::windows_ime_ipc::ImeSubmitRequest {
        session_id: Uuid::new_v4().to_string(),
        text: polished.to_string(),
        created_at: Utc::now().to_rfc3339(),
        target: ime_target,
    };

    let ime_status = match inner.windows_ime.submit_prepared(&prepared, request).await {
        Ok(status) => status,
        Err(error) => {
            log::warn!("[windows-ime] TSF submit failed: {error}");
            InsertStatus::Failed
        }
    };
    inner.windows_ime.restore_session(prepared);

    if ime_status == InsertStatus::Inserted {
        ime_status
    } else if should_try_non_tsf_insertion_fallback(allow_non_tsf_insertion_fallback, ime_status) {
        insert_via_non_tsf_fallback(inner, polished, restore_clipboard, paste_shortcut)
    } else {
        log::warn!("[windows-ime] TSF did not insert; non-TSF insertion fallback is disabled");
        InsertStatus::Failed
    }
}

#[cfg(target_os = "windows")]
fn should_try_non_tsf_insertion_fallback(
    allow_non_tsf_insertion_fallback: bool,
    ime_status: InsertStatus,
) -> bool {
    allow_non_tsf_insertion_fallback && ime_status != InsertStatus::Inserted
}

#[cfg(target_os = "windows")]
pub(super) fn insert_via_non_tsf_fallback(
    inner: &Arc<Inner>,
    polished: &str,
    _restore_clipboard: bool,
    _paste_shortcut: PasteShortcut,
) -> InsertStatus {
    let prefs = inner.prefs.get();
    let sendinput_options = dictation::windows_sendinput_options_from_prefs(&prefs);
    let status = finish_non_tsf_insertion_fallback(
        || {
            inner
                .inserter
                .insert_via_unicode_keystrokes(polished, sendinput_options)
        },
        || inner.inserter.copy_fallback(polished),
    );

    match status {
        InsertStatus::Inserted => {
            log::warn!(
                "[windows-ime] TSF unavailable; inserted via paced Unicode SendInput fallback"
            );
        }
        InsertStatus::CopiedFallback => {
            log::warn!(
                "[windows-ime] TSF unavailable; Unicode SendInput failed, left text on clipboard"
            );
        }
        InsertStatus::PasteSent | InsertStatus::Failed => {
            log::warn!(
                "[windows-ime] TSF unavailable; Unicode SendInput fallback failed and copy fallback failed"
            );
        }
    }

    status
}

#[cfg(any(target_os = "windows", test))]
fn finish_non_tsf_insertion_fallback<U, C>(
    mut unicode_fallback: U,
    mut copy_only_fallback: C,
) -> InsertStatus
where
    U: FnMut() -> InsertStatus,
    C: FnMut() -> InsertStatus,
{
    match unicode_fallback() {
        InsertStatus::Inserted => InsertStatus::Inserted,
        InsertStatus::PasteSent | InsertStatus::CopiedFallback | InsertStatus::Failed => {
            match copy_only_fallback() {
                InsertStatus::CopiedFallback => InsertStatus::CopiedFallback,
                // TextInserter::copy_fallback is copy-only: success is CopiedFallback.
                // Treat any other status as failure so this helper never invents an insert.
                InsertStatus::Inserted | InsertStatus::PasteSent | InsertStatus::Failed => {
                    InsertStatus::Failed
                }
            }
        }
    }
}

#[cfg(test)]
mod non_tsf_fallback_tests {
    use super::finish_non_tsf_insertion_fallback;
    use crate::types::InsertStatus;

    #[test]
    fn unicode_fallback_runs_before_copy_fallback() {
        let mut copy_called = false;
        let status = finish_non_tsf_insertion_fallback(
            || InsertStatus::Inserted,
            || {
                copy_called = true;
                InsertStatus::CopiedFallback
            },
        );

        assert_eq!(status, InsertStatus::Inserted);
        assert!(!copy_called);
    }

    #[test]
    fn copy_fallback_runs_after_unicode_failure() {
        let mut copy_called = false;
        let status = finish_non_tsf_insertion_fallback(
            || InsertStatus::Failed,
            || {
                copy_called = true;
                InsertStatus::CopiedFallback
            },
        );

        assert_eq!(status, InsertStatus::CopiedFallback);
        assert!(copy_called);
    }

    #[test]
    fn double_failure_does_not_pretend_text_was_copied() {
        let mut copy_called = false;
        let status = finish_non_tsf_insertion_fallback(
            || InsertStatus::Failed,
            || {
                copy_called = true;
                InsertStatus::Failed
            },
        );

        assert_eq!(status, InsertStatus::Failed);
        assert!(copy_called);
    }
}

// ─────────────────────────── helpers ───────────────────────────

/// 构建 ASR 客户端那一刻捕获的 (provider, model) 快照。随会话资源一起存放
/// （store_asr_for_session），end_session 取走写 history。provider 是实际构建用的
/// 具体协议 id（统一百炼入口会先经 resolve_effective_asr_provider 重定向）；model
/// 是构建时实际传给客户端的值（含 alias 归一化与默认回退）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrCallLabel {
    pub provider: String,
    pub model: Option<String>,
}

impl AsrCallLabel {
    pub(crate) fn new(provider: impl Into<String>, model: Option<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.filter(|m| !m.trim().is_empty()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::dictation::abort_recording_with_error;
    use super::dictation::{handle_pressed_edge, handle_released_edge};
    use super::*;
    use crate::types::{HotkeyMode, HotkeyTrigger};
    use once_cell::sync::Lazy;

    static ENV_LOCK: Lazy<tokio::sync::Mutex<()>> = Lazy::new(|| tokio::sync::Mutex::new(()));

    fn session_id(n: u128) -> SessionId {
        Uuid::from_u128(n)
    }

    #[tokio::test]
    async fn hotkey_injection_gate_logs_pressed_and_cancels() {
        let _ = env_logger::builder()
            .filter_level(log::LevelFilter::Info)
            .is_test(false)
            .try_init();
        let _guard = ENV_LOCK.lock().await;
        std::env::set_var("OPENLESS_HOTKEY_INJECTION_DRY_RUN", "1");

        let coordinator = Coordinator::new();
        coordinator.inject_hotkey_click_for_dev().await.unwrap();

        assert_eq!(coordinator.inner.state.lock().phase, SessionPhase::Idle);
        std::env::remove_var("OPENLESS_HOTKEY_INJECTION_DRY_RUN");
    }

    #[tokio::test]
    async fn begin_session_dry_run_enters_listening_and_clears_stale_edges() {
        let _guard = ENV_LOCK.lock().await;
        std::env::set_var("OPENLESS_HOTKEY_INJECTION_DRY_RUN", "1");

        let coordinator = Coordinator::new();
        let old_session_id = coordinator.inner.state.lock().session_id;
        {
            let mut state = coordinator.inner.state.lock();
            state.pending_stop = true;
            state.cancelled = true;
        }

        coordinator.start_dictation().await.unwrap();

        let state = coordinator.inner.state.lock();
        assert_eq!(state.phase, SessionPhase::Listening);
        assert!(!state.pending_stop);
        assert!(!state.cancelled);
        assert_ne!(state.session_id, old_session_id);

        std::env::remove_var("OPENLESS_HOTKEY_INJECTION_DRY_RUN");
    }

    #[tokio::test]
    async fn begin_session_ignores_non_idle_phase() {
        let _guard = ENV_LOCK.lock().await;
        std::env::set_var("OPENLESS_HOTKEY_INJECTION_DRY_RUN", "1");

        let coordinator = Coordinator::new();
        let old_session_id = {
            let mut state = coordinator.inner.state.lock();
            state.phase = SessionPhase::Processing;
            state.session_id = session_id(99);
            state.session_id
        };

        coordinator.start_dictation().await.unwrap();

        let state = coordinator.inner.state.lock();
        assert_eq!(state.phase, SessionPhase::Processing);
        assert_eq!(state.session_id, old_session_id);

        std::env::remove_var("OPENLESS_HOTKEY_INJECTION_DRY_RUN");
    }

    #[test]
    fn window_key_matcher_mirrors_windows_trigger_aliases() {
        let cases = [
            (HotkeyTrigger::RightControl, "Control", "ControlRight"),
            (HotkeyTrigger::LeftControl, "Control", "ControlLeft"),
            (HotkeyTrigger::RightOption, "Alt", "AltRight"),
            (HotkeyTrigger::RightAlt, "AltGraph", "AltRight"),
            (HotkeyTrigger::RightCommand, "Meta", "MetaRight"),
            (HotkeyTrigger::LeftOption, "Alt", "AltLeft"),
            // Mirrors Windows trigger_to_vk_code aliases.
            (HotkeyTrigger::Fn, "Control", "ControlRight"),
        ];
        for (trigger, key, code) in cases {
            assert!(
                window_key_matches_trigger(trigger, key, code),
                "{trigger:?} should match {key}/{code}"
            );
        }

        assert!(!window_key_matches_trigger(
            HotkeyTrigger::RightControl,
            "Control",
            "ControlLeft"
        ));
        assert!(!window_key_matches_trigger(
            HotkeyTrigger::LeftOption,
            "Alt",
            "AltRight"
        ));
        assert!(!window_key_matches_trigger(HotkeyTrigger::Fn, "Fn", "Fn"));
    }

    #[test]
    fn deferred_asr_bridge_flushes_startup_audio_before_live_chunks() {
        #[derive(Default)]
        struct RecordingConsumer {
            bytes: Mutex<Vec<u8>>,
        }

        impl crate::asr::AudioConsumer for RecordingConsumer {
            fn consume_pcm_chunk(&self, pcm: &[u8]) {
                self.bytes.lock().extend_from_slice(pcm);
            }
        }

        let bridge = DeferredAsrBridge::new();
        crate::recorder::AudioConsumer::consume_pcm_chunk(&bridge, &[1, 2]);
        crate::recorder::AudioConsumer::consume_pcm_chunk(&bridge, &[3, 4]);

        let target = Arc::new(RecordingConsumer::default());
        let target_for_attach: Arc<dyn crate::asr::AudioConsumer> = target.clone();
        assert_eq!(bridge.attach(target_for_attach), 4);

        crate::recorder::AudioConsumer::consume_pcm_chunk(&bridge, &[5, 6]);
        assert_eq!(&*target.bytes.lock(), &[1, 2, 3, 4, 5, 6]);
    }

    #[tokio::test]
    async fn manual_stop_during_starting_is_queued() {
        let coordinator = Coordinator::new();
        {
            let mut state = coordinator.inner.state.lock();
            state.phase = SessionPhase::Starting;
            state.pending_stop = false;
        }

        coordinator.stop_dictation().await.unwrap();

        let state = coordinator.inner.state.lock();
        assert_eq!(state.phase, SessionPhase::Starting);
        assert!(state.pending_stop);
    }

    #[tokio::test]
    async fn stop_dictation_from_listening_without_asr_returns_idle() {
        let coordinator = Coordinator::new();
        {
            let mut state = coordinator.inner.state.lock();
            state.phase = SessionPhase::Listening;
            state.session_id = session_id(123);
        }

        coordinator.stop_dictation().await.unwrap();

        assert_eq!(coordinator.inner.state.lock().phase, SessionPhase::Idle);
    }

    #[tokio::test]
    async fn stale_capsule_idle_schedule_does_not_hide_newer_state() {
        let coordinator = Coordinator::new();
        // 旧 schedule 触发时若期间有更新的 emit，应跳过隐藏（voice agent 取消双 emit 竞争）。
        emit_capsule(&coordinator.inner, CapsuleState::Done, 0.0, 0, None, None);
        schedule_capsule_idle(&coordinator.inner, 30);
        emit_capsule(
            &coordinator.inner,
            CapsuleState::Cancelled,
            0.0,
            0,
            None,
            None,
        );
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        assert_eq!(
            coordinator
                .inner
                .last_capsule_state
                .lock()
                .as_ref()
                .copied(),
            Some(CapsuleState::Cancelled),
            "旧 schedule 不应把更新的 Cancelled 状态提前隐藏"
        );
    }

    #[tokio::test]
    async fn capsule_idle_schedule_hides_when_no_newer_state() {
        let coordinator = Coordinator::new();
        emit_capsule(&coordinator.inner, CapsuleState::Done, 0.0, 0, None, None);
        schedule_capsule_idle(&coordinator.inner, 30);
        tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        assert_eq!(
            coordinator
                .inner
                .last_capsule_state
                .lock()
                .as_ref()
                .copied(),
            Some(CapsuleState::Idle),
            "无新 emit 时 schedule 应隐藏胶囊"
        );
    }

    #[test]
    fn cancel_session_state_machine_is_table_driven() {
        let cases = [
            (SessionPhase::Idle, SessionPhase::Idle, false),
            (SessionPhase::Starting, SessionPhase::Idle, true),
            (SessionPhase::Listening, SessionPhase::Idle, true),
            (SessionPhase::Processing, SessionPhase::Processing, true),
            (SessionPhase::Inserting, SessionPhase::Inserting, false),
        ];

        for (initial, expected_phase, expected_cancelled) in cases {
            let coordinator = Coordinator::new();
            {
                let mut state = coordinator.inner.state.lock();
                state.phase = initial;
                state.cancelled = false;
                state.focus_target = Some(1);
            }

            coordinator.cancel_dictation();

            let state = coordinator.inner.state.lock();
            assert_eq!(state.phase, expected_phase, "initial={initial:?}");
            assert_eq!(state.cancelled, expected_cancelled, "initial={initial:?}");
            if matches!(initial, SessionPhase::Starting | SessionPhase::Listening) {
                assert!(state.focus_target.is_none(), "initial={initial:?}");
            }
        }
    }

    #[test]
    fn recorder_runtime_error_aborts_active_session() {
        let coordinator = Coordinator::new();
        {
            let mut state = coordinator.inner.state.lock();
            state.phase = SessionPhase::Listening;
            state.cancelled = false;
        }

        abort_recording_with_error(&coordinator.inner, "录音中断: stream failed".to_string());

        let state = coordinator.inner.state.lock();
        assert_eq!(state.phase, SessionPhase::Idle);
        assert!(state.cancelled);
        assert!(coordinator.inner.recorder.lock().is_none());
        assert!(coordinator.inner.asr.lock().is_none());
    }

    #[test]
    fn abort_recording_keeps_session_non_idle_until_restore_can_run() {
        let mut state = SessionState::default();
        state.phase = SessionPhase::Listening;
        state.cancelled = false;
        state.session_id = session_id(7);

        let abort = begin_recording_abort_before_restore(&mut state).unwrap();

        assert_eq!(abort.session_id, session_id(7));
        assert!(state.cancelled);
        assert_eq!(state.phase, SessionPhase::Listening);

        publish_abort_idle_after_restore(&mut state, abort.session_id);

        assert_eq!(state.phase, SessionPhase::Idle);
    }

    #[tokio::test]
    async fn pressed_edge_during_inserting_does_not_start_new_session() {
        let coordinator = Coordinator::new();
        {
            let mut state = coordinator.inner.state.lock();
            state.phase = SessionPhase::Inserting;
            state.session_id = session_id(41);
        }

        handle_pressed_edge(&coordinator.inner, std::time::Instant::now(), 1).await;

        let state = coordinator.inner.state.lock();
        assert_eq!(state.phase, SessionPhase::Inserting);
        assert_eq!(state.session_id, session_id(41));
    }

    // #856：识别中按下热键想录下一条的 Pressed 会在会话收尾后被串行 bridge 取出（落在
    // 冷却期内）—— 现在一律静默丢弃，不再像「排队接力」那样放行开录下一条（无反馈排队 +
    // 延迟开录的惊吓成本大于省下的等待时间；Esc 取消后也不会因此再弹出一条新录音）。
    #[tokio::test]
    async fn toggle_press_within_cooldown_is_dropped() {
        let coordinator = Coordinator::new();
        // Coordinator::new() 会读真实用户 prefs（测试未隔离 HOME），用户可能把热键设成了
        // Hold（Hold 分支没有冷却检查，会直接开录）。显式锁定 Toggle 才能稳定测冷却语义。
        coordinator
            .inner
            .prefs
            .set(crate::types::UserPreferences {
                hotkey: crate::types::HotkeyBinding {
                    mode: HotkeyMode::Toggle,
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap();
        // Idle + 冷却未过期：模拟「识别中按下 → 会话收尾 → bridge 取出该 Pressed」的时刻。
        *coordinator.inner.session_cooldown_until.lock() = Some(
            std::time::Instant::now() + std::time::Duration::from_millis(POST_SESSION_COOLDOWN_MS),
        );

        handle_pressed_edge(&coordinator.inner, std::time::Instant::now(), 1).await;

        // 静默丢弃：没有开录下一条（phase 仍是 Idle）。
        assert_eq!(coordinator.inner.state.lock().phase, SessionPhase::Idle);
    }

    #[tokio::test]
    async fn repeated_pressed_edge_during_hold_session_does_not_restart() {
        let coordinator = Coordinator::new();
        coordinator
            .inner
            .prefs
            .set(crate::types::UserPreferences {
                hotkey: crate::types::HotkeyBinding {
                    trigger: HotkeyTrigger::RightControl,
                    mode: HotkeyMode::Hold,
                    keys: None,
                },
                ..Default::default()
            })
            .unwrap();
        coordinator.inner.state.lock().phase = SessionPhase::Listening;
        coordinator
            .inner
            .hotkey_trigger_held
            .store(true, Ordering::SeqCst);

        handle_pressed_edge(&coordinator.inner, std::time::Instant::now(), 1).await;

        assert_eq!(
            coordinator.inner.state.lock().phase,
            SessionPhase::Listening
        );
        assert!(coordinator.inner.hotkey_trigger_held.load(Ordering::SeqCst));
    }

    fn set_auto_mode(coordinator: &Coordinator) {
        coordinator
            .inner
            .prefs
            .set(crate::types::UserPreferences {
                hotkey: crate::types::HotkeyBinding {
                    trigger: HotkeyTrigger::RightControl,
                    mode: HotkeyMode::Auto,
                    keys: None,
                },
                ..Default::default()
            })
            .unwrap();
    }

    // Auto 模式短按：松手时按住时长 < 阈值 → 锁存为切换态，保持 Listening（不结束会话）。
    #[tokio::test]
    async fn auto_short_tap_release_latches_recording() {
        let coordinator = Coordinator::new();
        set_auto_mode(&coordinator);
        coordinator.inner.state.lock().phase = SessionPhase::Listening;
        // 刚按下（elapsed ≈ 0 < 350ms）→ 短按。
        let pressed_at = std::time::Instant::now();
        *coordinator.inner.hotkey_press_at.lock() = Some(pressed_at);
        coordinator
            .inner
            .hotkey_trigger_held
            .store(true, Ordering::SeqCst);

        handle_released_edge(
            &coordinator.inner,
            pressed_at + std::time::Duration::from_millis(100),
        )
        .await;

        // 短按松手不结束录音，等下一次按下再停。
        assert_eq!(
            coordinator.inner.state.lock().phase,
            SessionPhase::Listening
        );
    }

    #[tokio::test]
    async fn auto_short_tap_stays_latched_when_bridge_handles_release_late() {
        let coordinator = Coordinator::new();
        set_auto_mode(&coordinator);
        coordinator.inner.state.lock().phase = SessionPhase::Listening;
        let pressed_at = std::time::Instant::now();
        *coordinator.inner.hotkey_press_at.lock() = Some(pressed_at);
        coordinator
            .inner
            .hotkey_trigger_held
            .store(true, Ordering::SeqCst);

        // 模拟上一条会话阻塞 bridge：处理发生在物理松手很久之后。
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        handle_released_edge(
            &coordinator.inner,
            pressed_at + std::time::Duration::from_millis(100),
        )
        .await;

        assert_eq!(
            coordinator.inner.state.lock().phase,
            SessionPhase::Listening
        );
        assert!(coordinator.inner.hotkey_press_at.lock().is_none());
    }

    // Auto 模式长按：松手时按住时长 >= 阈值 → 按住说话语义，结束会话（Listening → Idle）。
    #[tokio::test]
    async fn auto_long_hold_release_ends_session() {
        let coordinator = Coordinator::new();
        set_auto_mode(&coordinator);
        coordinator.inner.state.lock().phase = SessionPhase::Listening;
        // 按住已超过阈值 → 长按。
        let pressed_at = std::time::Instant::now();
        *coordinator.inner.hotkey_press_at.lock() = Some(pressed_at);
        coordinator
            .inner
            .hotkey_trigger_held
            .store(true, Ordering::SeqCst);

        handle_released_edge(
            &coordinator.inner,
            pressed_at + std::time::Duration::from_millis(500),
        )
        .await;

        // 无 recorder / ASR 的测试会话下，end_session 直接收尾到 Idle。
        assert_eq!(coordinator.inner.state.lock().phase, SessionPhase::Idle);
        assert!(coordinator.inner.hotkey_press_at.lock().is_none());
    }

    // Option+任意字母/数字键：这次按下开出来的会话必须被撤销，且随后的松手边沿不能再被当成
    // Auto 短按锁存（否则录音一直开着，正是用户报的「按 Option+其他键唤起听写」）。
    #[tokio::test]
    async fn trigger_combined_cancels_session_started_by_this_press() {
        let coordinator = Coordinator::new();
        set_auto_mode(&coordinator);
        coordinator.inner.state.lock().phase = SessionPhase::Listening;
        // 长按场景（按住 400ms 说话中触发组合键）：保护后松手应正常结束会话。
        let pressed_at = std::time::Instant::now() - std::time::Duration::from_millis(400);
        *coordinator.inner.hotkey_press_at.lock() = Some(pressed_at);
        coordinator
            .inner
            .hotkey_trigger_held
            .store(true, Ordering::SeqCst);
        coordinator
            .inner
            .hotkey_press_generation
            .store(1, Ordering::SeqCst);
        coordinator
            .inner
            .hotkey_press_began_session
            .store(1, Ordering::SeqCst);

        handle_trigger_combined(&coordinator.inner, 1);

        // 长时说话保护：录音中组合键不取消会话、不重置 held（松手仍正常结束）。
        assert_eq!(
            coordinator.inner.state.lock().phase,
            SessionPhase::Listening
        );
        assert!(coordinator.inner.hotkey_trigger_held.load(Ordering::SeqCst));

        handle_released_edge(&coordinator.inner, std::time::Instant::now()).await;

        // 长按松开：会话正常结束（组合键保护不破坏松手结束链路）。
        assert_eq!(coordinator.inner.state.lock().phase, SessionPhase::Idle);
    }

    // 这次按下是 toggle 停止（没开出会话）时，组合键撤销不能顺手取消正在跑的会话 ——
    // 那条录音是上一次按下锁存的，取消 = 用户白说一段。
    #[tokio::test]
    async fn trigger_combined_leaves_session_it_did_not_start() {
        let coordinator = Coordinator::new();
        set_auto_mode(&coordinator);
        coordinator.inner.state.lock().phase = SessionPhase::Listening;
        coordinator
            .inner
            .hotkey_trigger_held
            .store(true, Ordering::SeqCst);
        coordinator
            .inner
            .hotkey_press_generation
            .store(1, Ordering::SeqCst);
        coordinator
            .inner
            .hotkey_press_began_session
            .store(0, Ordering::SeqCst);

        handle_trigger_combined(&coordinator.inner, 1);

        // 录音中保护：held 不被重置（组合键不破坏松手结束链路）。
        assert_eq!(
            coordinator.inner.state.lock().phase,
            SessionPhase::Listening
        );
        assert!(coordinator.inner.hotkey_trigger_held.load(Ordering::SeqCst));
    }

    // 组合键撤销通道独立于 Released；若正常松手已经把会话收尾到 Idle，迟到的撤销
    // 不能清掉正常会话的冷却/防抖，否则下一次三连按会绕过 #545 的保护。
    #[tokio::test]
    async fn late_trigger_combined_does_not_clear_completed_session_guards() {
        let coordinator = Coordinator::new();
        set_auto_mode(&coordinator);
        let now = std::time::Instant::now();
        *coordinator.inner.session_cooldown_until.lock() =
            Some(now + std::time::Duration::from_secs(1));
        *coordinator.inner.last_hotkey_dispatch_at.lock() = Some(now);
        coordinator
            .inner
            .hotkey_press_generation
            .store(1, Ordering::SeqCst);
        coordinator
            .inner
            .hotkey_press_began_session
            .store(1, Ordering::SeqCst);

        handle_trigger_combined(&coordinator.inner, 1);

        assert_eq!(coordinator.inner.state.lock().phase, SessionPhase::Idle);
        assert!(coordinator.inner.session_cooldown_until.lock().is_some());
        assert!(coordinator.inner.last_hotkey_dispatch_at.lock().is_some());
    }

    // 撤销走独立线程后，它与 Pressed/Released 那条串行 bridge 之间没有先后保证。
    // 万一 Released 抢先跑完（把按住态清了、Auto 还锁存成了切换态），撤销仍然必须认出
    // 这条会话是自己那次按下开的并取消掉 —— 否则组合键会留下一条停不下来的录音，
    // 正是本 PR 要修的老毛病换个形式复发。
    #[tokio::test]
    async fn trigger_combined_still_cancels_when_released_edge_wins_the_race() {
        let coordinator = Coordinator::new();
        set_auto_mode(&coordinator);
        coordinator.inner.state.lock().phase = SessionPhase::Listening;
        let pressed_at = std::time::Instant::now();
        *coordinator.inner.hotkey_press_at.lock() = Some(pressed_at);
        coordinator
            .inner
            .hotkey_trigger_held
            .store(true, Ordering::SeqCst);
        coordinator
            .inner
            .hotkey_press_generation
            .store(1, Ordering::SeqCst);
        coordinator
            .inner
            .hotkey_press_began_session
            .store(1, Ordering::SeqCst);

        // 先跑 Released（短按 → Auto 锁存成切换态，录音继续），撤销后到。
        handle_released_edge(
            &coordinator.inner,
            pressed_at + std::time::Duration::from_millis(80),
        )
        .await;
        assert_eq!(
            coordinator.inner.state.lock().phase,
            SessionPhase::Listening
        );

        handle_trigger_combined(&coordinator.inner, 1);

        // 录音中保护：组合键不取消已锁存的会话。
        assert_eq!(
            coordinator.inner.state.lock().phase,
            SessionPhase::Listening
        );
    }

    #[test]
    fn enabling_shortcut_recording_clears_dictation_hold_latch() {
        let coordinator = Coordinator::new();
        coordinator
            .inner
            .hotkey_trigger_held
            .store(true, Ordering::SeqCst);

        coordinator.set_shortcut_recording_active(true);

        assert!(!coordinator.inner.hotkey_trigger_held.load(Ordering::SeqCst));
    }

    #[test]
    fn window_hotkey_fallback_is_disabled_when_no_explicit_fallback_is_advertised() {
        assert_eq!(
            window_hotkey_fallback_enabled(),
            crate::types::HotkeyCapability::current().explicit_fallback_available
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn prepared_windows_ime_slot_is_taken_only_for_matching_session() {
        let mut slots = vec![PreparedWindowsImeSessionSlot {
            session_id: session_id(2),
            prepared: PreparedWindowsImeSession::unavailable(),
        }];

        assert!(take_matching_prepared_windows_ime_session(&mut slots, session_id(1)).is_none());
        assert_eq!(
            slots.iter().map(|slot| slot.session_id).collect::<Vec<_>>(),
            vec![session_id(2)]
        );

        assert!(take_matching_prepared_windows_ime_session(&mut slots, session_id(2)).is_some());
        assert!(slots.is_empty());
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn prepared_windows_ime_sessions_keep_overlapping_snapshots() {
        let mut slots = Vec::new();
        store_prepared_windows_ime_session(
            &mut slots,
            session_id(1),
            PreparedWindowsImeSession::unavailable(),
        );
        store_prepared_windows_ime_session(
            &mut slots,
            session_id(2),
            PreparedWindowsImeSession::unavailable(),
        );

        assert_eq!(
            slots.iter().map(|slot| slot.session_id).collect::<Vec<_>>(),
            vec![session_id(1), session_id(2)]
        );

        assert!(take_matching_prepared_windows_ime_session(&mut slots, session_id(1)).is_some());
        assert_eq!(
            slots.iter().map(|slot| slot.session_id).collect::<Vec<_>>(),
            vec![session_id(2)]
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn stale_prepared_windows_ime_restore_discards_old_snapshot_without_restoring() {
        let mut slots = Vec::new();
        store_prepared_windows_ime_session(
            &mut slots,
            session_id(1),
            PreparedWindowsImeSession::unavailable(),
        );
        store_prepared_windows_ime_session(
            &mut slots,
            session_id(2),
            PreparedWindowsImeSession::unavailable(),
        );

        assert!(take_current_prepared_windows_ime_session_for_restore(
            &mut slots,
            session_id(1),
            session_id(2)
        )
        .is_none());
        assert_eq!(
            slots.iter().map(|slot| slot.session_id).collect::<Vec<_>>(),
            vec![session_id(2)]
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn non_tsf_insertion_fallback_gate_blocks_only_when_disabled() {
        assert!(should_try_non_tsf_insertion_fallback(
            true,
            InsertStatus::CopiedFallback
        ));
        assert!(should_try_non_tsf_insertion_fallback(
            true,
            InsertStatus::Failed
        ));
        assert!(!should_try_non_tsf_insertion_fallback(
            true,
            InsertStatus::Inserted
        ));
        assert!(!should_try_non_tsf_insertion_fallback(
            false,
            InsertStatus::CopiedFallback
        ));
        assert!(!should_try_non_tsf_insertion_fallback(
            false,
            InsertStatus::Failed
        ));
    }

    #[test]
    fn focus_restore_failure_uses_specific_error_code_when_insert_fails() {
        assert_eq!(
            dictation_error_code(
                InsertStatus::Failed,
                false,
                false,
                false,
                crate::types::WindowsInsertionMode::Tsf,
            ),
            Some("focusRestoreFailed")
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn missing_windows_hwnd_is_not_present() {
        use windows::Win32::Foundation::HWND;

        assert!(!windows_hwnd_is_present(HWND::default()));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn tsf_required_failure_keeps_tsf_error_when_focus_was_ready() {
        assert_eq!(
            dictation_error_code(
                InsertStatus::Failed,
                false,
                true,
                false,
                crate::types::WindowsInsertionMode::Tsf,
            ),
            Some("windowsImeTsfRequired")
        );
    }

    #[test]
    fn sendinput_only_mode_skips_tsf_required_error() {
        assert_eq!(
            dictation_error_code(
                InsertStatus::Failed,
                false,
                true,
                false,
                crate::types::WindowsInsertionMode::SendInput,
            ),
            None
        );
    }

    #[test]
    fn startup_race_check_treats_newer_session_as_stale() {
        let mut state = SessionState::default();
        state.phase = SessionPhase::Starting;
        state.cancelled = false;
        state.session_id = session_id(2);

        assert_eq!(
            startup_race_status(&state, session_id(1)),
            StartupRaceStatus::StaleContinuation
        );
    }

    #[test]
    fn startup_race_check_is_table_driven_for_begin_session_edges() {
        let cases = [
            (
                SessionPhase::Starting,
                false,
                session_id(7),
                StartupRaceStatus::ActiveStarting,
            ),
            (
                SessionPhase::Starting,
                true,
                session_id(7),
                StartupRaceStatus::CancelRaced,
            ),
            (
                SessionPhase::Idle,
                false,
                session_id(7),
                StartupRaceStatus::CancelRaced,
            ),
            (
                SessionPhase::Listening,
                false,
                session_id(7),
                StartupRaceStatus::CancelRaced,
            ),
            (
                SessionPhase::Starting,
                false,
                session_id(8),
                StartupRaceStatus::StaleContinuation,
            ),
        ];

        for (phase, cancelled, actual_session_id, expected) in cases {
            let mut state = SessionState::default();
            state.phase = phase;
            state.cancelled = cancelled;
            state.session_id = actual_session_id;

            assert_eq!(
                startup_race_status(&state, session_id(7)),
                expected,
                "phase={phase:?} cancelled={cancelled} actual_session={actual_session_id}"
            );
        }
    }

    #[test]
    fn begin_recording_abort_is_noop_after_prior_cancel_or_idle() {
        let cases = [
            (SessionPhase::Idle, false),
            (SessionPhase::Processing, false),
            (SessionPhase::Listening, true),
        ];

        for (phase, cancelled) in cases {
            let mut state = SessionState::default();
            state.phase = phase;
            state.cancelled = cancelled;

            assert!(begin_recording_abort_before_restore(&mut state).is_none());
            assert_eq!(state.phase, phase);
            assert_eq!(state.cancelled, cancelled);
        }
    }

    #[test]
    fn stale_startup_cleanup_keeps_newer_asr_resource() {
        let coordinator = Coordinator::new();
        let newer_asr = crate::asr::DoubaoImeASR::new(None);
        *coordinator.inner.asr.lock() = Some(SessionResource::new(
            session_id(2),
            ActiveAsr::Doubao(Arc::clone(&newer_asr)),
        ));

        discard_startup_resources_for_session(&coordinator.inner, session_id(1));

        assert_eq!(
            coordinator
                .inner
                .asr
                .lock()
                .as_ref()
                .map(|resource| resource.session_id),
            Some(session_id(2))
        );

        discard_startup_resources_for_session(&coordinator.inner, session_id(2));

        assert!(coordinator.inner.asr.lock().is_none());
    }
}

/// 终止态（Done / Error）后延迟 N ms 把胶囊改回 Idle，让浮窗自动消失。
/// 点 ✓ / 中途出错走这里，保留 2 秒让用户看清结果 / 错误提示。
const CAPSULE_AUTO_HIDE_DELAY_MS: u64 = 150;

/// 用户主动取消（Esc / 点 ✕）时的收起延迟。取消是明确的「我不要了」意图，
/// 不需要像 Done/Error 那样停留 2 秒给用户读——立刻回 Idle，由前端 capsule-out
/// 淡出动画（520ms）负责优雅收尾，观感上「按下即消失」（对齐 Typeless）。
const CAPSULE_CANCEL_HIDE_DELAY_MS: u64 = 0;

/// Toggle 模式下，end_session 将 phase 设为 Idle 后在此时间内禁止新的 begin_session。
/// 避免用户三连按时第 3 次按下误激活新听写（此时胶囊仍在离场动画周期内）。
/// 值取 capsule EXIT_ANIM_MS (360ms) + 余量 ≈ 600ms。
const POST_SESSION_COOLDOWN_MS: u64 = 600;

/// Coordinator 全局超时保护：防止 ASR await_final_result() 永远挂起。
/// 设置为 30 秒，为云端 batch ASR（OpenRouter Whisper 等）提供足够的
/// 网络超时预算；只在 ASR 自身超时机制失效时作为最后的防线触发。
const COORDINATOR_GLOBAL_TIMEOUT_SECS: u64 = 30;

/// 检查 begin_session 的 await 间隙是否被 cancel_session 打断。
/// 必须在持有 state lock 的瞬间读，结果一拿就过期，所以用 helper 名字提醒只在
/// 「准备做下一步副作用前」用。
fn startup_race_status_for_starting(
    inner: &Arc<Inner>,
    captured_session_id: SessionId,
) -> StartupRaceStatus {
    let state = inner.state.lock();
    startup_race_status(&state, captured_session_id)
}

fn set_phase_idle_if_session_matches(inner: &Arc<Inner>, session_id: SessionId) {
    let mut state = inner.state.lock();
    if state.session_id == session_id {
        state.phase = SessionPhase::Idle;
    }
}

fn schedule_capsule_idle(inner: &Arc<Inner>, delay_ms: u64) {
    // 记录触发时胶囊显示的状态；到点时若期间有更新的 emit（last_capsule_state 已变），
    // 说明本次状态已被后续 emit 取代，隐藏交给那次 emit 自己的 schedule——避免旧
    // schedule 把新状态提前隐藏（如 voice agent 取消路径 cancel_session 与收尾双 emit）。
    let expect = inner.last_capsule_state.lock().as_ref().copied();
    let inner_clone = Arc::clone(inner);
    crate::runtime().spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        if inner_clone.last_capsule_state.lock().as_ref().copied() != expect {
            return;
        }
        // 必须 dictation **和** QA 同时空闲才能隐藏胶囊。否则旧 dictation Done timer
        // 的尾巴会在新 QA 录音/思考中把胶囊意外收掉（issue #118 v2 复现）。
        // 选区润色进行中或出现新 payload 时，函数内部依据 capsule epoch 放弃隐藏。
        hide_capsule_if_all_sessions_idle(&inner_clone);
    });
}

/// 选区润色终态的短暂展示。旧的 timer 只能收起自己那一代的 payload；若用户已经
/// 触发了下一轮 selection，或在此期间开始语音/QA，会直接放弃，不碰当前 capsule。
// ─────────────────────────── audio bridge ───────────────────────────

struct DeferredAsrBridge {
    state: Mutex<DeferredAsrState>,
}

struct DeferredAsrState {
    target: Option<Arc<dyn crate::asr::AudioConsumer>>,
    pending_audio: Vec<u8>,
    attaching: bool,
}

impl DeferredAsrBridge {
    fn new() -> Self {
        Self {
            state: Mutex::new(DeferredAsrState {
                target: None,
                pending_audio: Vec::new(),
                attaching: false,
            }),
        }
    }

    fn attach(&self, target: Arc<dyn crate::asr::AudioConsumer>) -> usize {
        let mut flushed_bytes = 0;
        {
            let mut state = self.state.lock();
            state.attaching = true;
        }

        loop {
            let pending = {
                let mut state = self.state.lock();
                if state.pending_audio.is_empty() {
                    state.target = Some(Arc::clone(&target));
                    state.attaching = false;
                    return flushed_bytes;
                }
                std::mem::take(&mut state.pending_audio)
            };
            flushed_bytes += pending.len();
            target.consume_pcm_chunk(&pending);
        }
    }
}

impl crate::recorder::AudioConsumer for DeferredAsrBridge {
    fn consume_pcm_chunk(&self, pcm: &[u8]) {
        let target = {
            let mut state = self.state.lock();
            if state.attaching {
                state.pending_audio.extend_from_slice(pcm);
                return;
            }
            if let Some(target) = state.target.as_ref() {
                Some(Arc::clone(target))
            } else {
                state.pending_audio.extend_from_slice(pcm);
                None
            }
        };

        if let Some(target) = target {
            target.consume_pcm_chunk(pcm);
        }
    }
}
