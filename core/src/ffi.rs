//! C ABI——SwiftUI 宿主与 core 的唯一接口。
//!
//! Swift 侧调用（见 app/Sources/FFI/Core.swift）：
//! - `zhunji_init()`：构造 Coordinator 并按原版 tauri setup 的序列初始化（热键监听、
//!   引擎预热、设备变更监听）。幂等，重复调用直接返回 0。**必须从 Swift 主线程调用
//!   一次**——global-hotkey 的 manager 要求主线程构造（见 combo_hotkey.rs 注释），
//!   init 里的预热保证之后 supervisor 线程 register 安全。
//! - `zhunji_set_event_callback(cb)`：注册事件回调。P0 冒烟用；P1 由 EventSink
//!   持强引用。已经注册过则保留第一个（返回 1）。
//! - `zhunji_request_shutdown()`：请求 core 退出（supervisor 线程 + 设备监听停止）。
//!
//! 事件统一 JSON：`{"event":"<name>","payload":<json>}`，经 `event_bus` 在任意线程以
//! NUL 结尾 CString 推给回调。回调是 `unsafe extern "C"`，Swift 侧必须持强引用防止
//! 提前释放（core 只存裸函数指针）。

use std::ffi::{c_char, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use super::coordinator::Coordinator;

static COORDINATOR: OnceLock<Coordinator> = OnceLock::new();

/// 把 String 转为堆分配 CString 指针（调用方用 zhunji_free_string 释放）。
fn into_c_string(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// 从 *const c_char 读 String（NULL 安全，返回空串）。
unsafe fn read_c_string(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(ptr).to_string_lossy().into_owned() }
}

/// 初始化 core。返回 0 成功（或已初始化），1 失败。
#[no_mangle]
pub extern "C" fn zhunji_init() -> i32 {
    static INITIALIZED: OnceLock<()> = OnceLock::new();
    if INITIALIZED.set(()).is_err() {
        return 0;
    }
    // 日志先行：之后 core 的所有 log::info 都能落到文件（zhunji.log）。
    crate::logging::init_file_logger();
    let coordinator = COORDINATOR.get_or_init(Coordinator::new);
    // 主线程预热 global-hotkey runtime：macOS 的 manager 必须在主线程构造，
    // 之后 hotkey supervisor 线程直接 register 才安全（原版靠
    // AppHandle.run_on_main_thread 隐式保证，native 靠 zhunji_init 的调用约定）。
    if let Err(e) = crate::global_hotkey_runtime::warmup_on_main_thread() {
        log::warn!("[startup] global-hotkey runtime warmup failed: {e}");
    }
    // 与 zhunlu 原版 run_desktop + setup 序列一致（无 bind_app——已随 tauri 剥离；
    // 窗口类操作全部事件化，由 Swift 执行）。
    coordinator.migrate_doubao_prefs();
    #[cfg(target_os = "windows")]
    if let Err(error) = coordinator.sync_active_asr_provider_from_preferences() {
        log::warn!("[startup] sync active ASR provider from preferences failed: {error}");
    }
    coordinator.start_hotkey_listener();
    coordinator.start_combo_hotkey_listener();
    coordinator.start_translation_hotkey_listener();
    coordinator.start_open_app_hotkey_listener();
    coordinator.warmup_engine();
    start_device_watcher();
    log::info!("[ffi] zhunji_init: core ready");
    // 通知宿主 core 就绪。调用约定：zhunji_set_event_callback 必须先于
    // zhunji_init（EventSink 在启动早期注册），否则这条事件丢失。
    crate::event_bus::emit_unit("app:core-ready");
    0
}

/// 注册事件回调。已注册过则保留第一个（返回 1），首次注册成功返回 0。
#[no_mangle]
pub extern "C" fn zhunji_set_event_callback(callback: crate::event_bus::EventCallback) -> i32 {
    if crate::event_bus::set_callback(callback) {
        0
    } else {
        1
    }
}

/// 请求 core 退出：supervisor 线程 + 设备监听线程停止。
#[no_mangle]
pub extern "C" fn zhunji_request_shutdown() {
    if let Some(coordinator) = COORDINATOR.get() {
        coordinator.request_shutdown();
    }
    crate::device_watch::stop_watcher();
}

/// 胶囊「取消」按钮 → 取消当前会话（原版 commands::cancel_dictation）。
#[no_mangle]
pub extern "C" fn zhunji_capsule_cancel() {
    if let Some(coordinator) = COORDINATOR.get() {
        coordinator.cancel_dictation();
    }
}

// MARK: - 偏好 / 凭据 / 设备（P1.4 设置页）

/// 偏好全量 JSON（camelCase，与 preferences.json 同构）。返回堆 CString，
/// Swift 侧用 zhunji_free_string 释放。
#[no_mangle]
pub extern "C" fn zhunji_get_prefs() -> *mut c_char {
    let json = match COORDINATOR.get() {
        Some(c) => serde_json::to_string(&c.prefs().get())
            .unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#)),
        None => "null".to_string(),
    };
    into_c_string(json)
}

/// 保存偏好（全量 JSON，缺失字段走 serde default）。副作用对齐 zhunlu 原版
/// set_settings：热键字段变化 → 立即重注册；完成后 emit prefs:changed
/// （Swift 侧刷新设置 UI / 胶囊外观）。
/// 返回 0 成功，1 未初始化，2 JSON 解析失败，3 写入失败。
#[no_mangle]
pub extern "C" fn zhunji_set_prefs(json: *const c_char) -> i32 {
    let Some(coordinator) = COORDINATOR.get() else {
        return 1;
    };
    let json = unsafe { read_c_string(json) };
    let prefs: crate::types::UserPreferences = match serde_json::from_str(&json) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("[ffi] set_prefs 解析失败: {e}");
            return 2;
        }
    };
    let old = coordinator.prefs().get();
    if let Err(e) = coordinator.prefs().set(prefs.clone()) {
        log::warn!("[ffi] set_prefs 写入失败: {e}");
        return 3;
    }
    // 热键变化 → 立即生效（不重启进程）。
    if old.dictation_hotkey != prefs.dictation_hotkey {
        if let Err(e) = coordinator.update_dictation_hotkey_binding() {
            log::warn!("[ffi] dictation hotkey 重注册失败: {e}");
        }
    }
    if old.translation_hotkey != prefs.translation_hotkey {
        if let Err(e) = coordinator.try_update_translation_hotkey_binding() {
            log::warn!("[ffi] translation hotkey 重注册失败: {e}");
        }
    }
    if old.open_app_hotkey != prefs.open_app_hotkey {
        coordinator.update_open_app_hotkey_binding();
    }
    crate::event_bus::emit_unit("prefs:changed");
    0
}

/// 麦克风设备列表 JSON（原版 list_microphone_devices）：
/// `[{"name":"...","isDefault":true}, ...]`。
#[no_mangle]
pub extern "C" fn zhunji_list_microphone_devices() -> *mut c_char {
    let json = match crate::recorder::list_input_devices() {
        Ok(devices) => serde_json::to_string(&devices).unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#)),
        Err(e) => format!(r#"{{"error":"{e}"}}"#),
    };
    into_c_string(json)
}

/// ASR 供应商注册表 JSON（原版 list_providers，引擎下拉的数据源）：
/// `[{"id":"...","name":"...","url":"...","default":true}, ...]`（内置豆包常驻）。
#[no_mangle]
pub extern "C" fn zhunji_list_providers() -> *mut c_char {
    let json = match crate::providers::list_providers() {
        Ok(list) => serde_json::to_string(&list)
            .unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#)),
        Err(e) => format!(r#"{{"error":"{e}"}}"#),
    };
    into_c_string(json)
}

/// 释放 core 分配的内存（get_prefs / list_microphone_devices 等
/// 返回 JSON 字符串的命令的返回值）。
#[no_mangle]
pub extern "C" fn zhunji_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        unsafe { drop(CString::from_raw(ptr)) };
    }
}

/// 胶囊「确认」按钮 → 停止听写并提交插入（原版 commands::stop_dictation）。
#[no_mangle]
pub extern "C" fn zhunji_capsule_confirm() {
    if let Some(coordinator) = COORDINATOR.get() {
        let coordinator = coordinator.clone();
        crate::runtime().spawn(async move {
            let _ = coordinator.stop_dictation().await;
        });
    }
}

/// 麦克风设备变更监听（原版 tray 菜单刷新：签名去抖 + in-flight 防并发）。
/// P0 简化：in-flight flag 防并发 + 300ms 时间节流，变更时发 `device:changed`
/// 事件（P1 由 Swift 刷新设备列表 / 设置页；签名比较随设备列表一起做）。
fn start_device_watcher() {
    let in_flight = Arc::new(AtomicBool::new(false));
    let last_event = Arc::new(parking_lot::Mutex::new(std::time::Instant::now()));
    let handler = {
        let in_flight = Arc::clone(&in_flight);
        let last_event = Arc::clone(&last_event);
        move || {
            if in_flight.swap(true, Ordering::AcqRel) {
                return;
            }
            let flag = Arc::clone(&in_flight);
            let last = Arc::clone(&last_event);
            if let Err(err) = std::thread::Builder::new()
                .name("zhunji-mic-event".into())
                .spawn(move || {
                    if last.lock().elapsed() >= std::time::Duration::from_millis(300) {
                        *last.lock() = std::time::Instant::now();
                        crate::event_bus::emit_unit("device:changed");
                    }
                    flag.store(false, Ordering::Release);
                })
            {
                in_flight.store(false, Ordering::Release);
                log::warn!("[ffi] start microphone event refresh failed: {err}");
            }
        }
    };
    if crate::device_watch::spawn_native_watcher(handler) {
        log::info!("[ffi] OS native microphone device watcher registered");
    } else {
        log::info!(
            "[ffi] no OS native microphone device watcher (unsupported or failed); \
             relying on poll fallback"
        );
    }
}

// MARK: - 权限页（P1.4）

/// 热键监听状态 JSON：`{"adapter":"...","state":"installed|starting|failed","message":...}`
/// （原版 get_hotkey_status）。
#[no_mangle]
pub extern "C" fn zhunji_get_hotkey_status() -> *mut c_char {
    let json = match COORDINATOR.get() {
        Some(c) => {
            let s = c.hotkey_status();
            serde_json::json!({
                "adapter": s.adapter,
                "state": s.state,
                "message": s.message,
            })
            .to_string()
        }
        None => "null".to_string(),
    };
    into_c_string(json)
}

/// 网络连通性检查（原版 check_network）：GET 首选 ASR 端点测延迟，异步执行，
/// 完成后发 `network:result` 事件 `{"online":bool,"latencyMs":u64|null}`。
/// 返回 0 已发起，1 core 未初始化。
#[no_mangle]
pub extern "C" fn zhunji_check_network() -> i32 {
    let Some(coordinator) = COORDINATOR.get() else {
        return 1;
    };
    // 按 active ASR provider 探真实端点（原版 misc.rs check_network）：
    // 豆包 → token 签发域；第三方供应商 → providers.json 里的 URL；兜底 token 签发域。
    // Grok STT 凭据文件是供应商凭据的搬运缓存，不做独立探测入口。
    let active = coordinator.prefs().get().active_asr_provider;
    let url = if active == "builtin-doubao" || active == crate::asr::doubao::PROVIDER_ID {
        "https://is.snssdk.com/".to_string()
    } else if let Ok(list) = crate::providers::list_providers() {
        list.into_iter()
            .find(|p| p.id == active)
            .map(|p| p.url)
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| "https://is.snssdk.com/".to_string())
    } else {
        "https://is.snssdk.com/".to_string()
    };
    crate::runtime().spawn(async move {
        let start = std::time::Instant::now();
        let online = crate::net::http()
            .get(url)
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await
            .is_ok();
        let payload = serde_json::json!({
            "online": online,
            "latencyMs": online.then(|| start.elapsed().as_millis() as u64),
        });
        crate::event_bus::emit("network:result", &payload);
    });
    0
}

// MARK: - 麦克风电平监听（P1.4 设置页麦克风下拉）

/// 电平探针 consumer：PCM 直接丢弃（电平由 level_handler 算好推送，不需消费）。
struct LevelProbeConsumer;

impl crate::recorder::AudioConsumer for LevelProbeConsumer {
    fn consume_pcm_chunk(&self, _pcm: &[u8]) {}
}

/// 当前电平监听（同一时刻至多一个；设置页下拉打开时启用，关闭即停）。
static LEVEL_MONITOR: OnceLock<parking_lot::Mutex<Option<crate::recorder::Recorder>>> =
    OnceLock::new();

/// 开始监听指定麦克风电平（device_name 空 = 系统默认）。已存在的监听先停掉
/// （原版 start_microphone_level_monitor）。电平 0..1 经 `microphone:level`
/// 事件 `{"level":f32}` 推送。返回 0 成功，1 启动失败。
#[no_mangle]
pub extern "C" fn zhunji_start_microphone_level_monitor(device_name: *const c_char) -> i32 {
    let monitor = LEVEL_MONITOR.get_or_init(|| parking_lot::Mutex::new(None));
    if let Some(existing) = monitor.lock().take() {
        existing.stop();
    }
    let selected = unsafe { read_c_string(device_name) }.trim().to_string();
    let device = if selected.is_empty() { None } else { Some(selected) };
    let consumer: Arc<dyn crate::recorder::AudioConsumer> = Arc::new(LevelProbeConsumer);
    let handler: Arc<dyn Fn(f32) + Send + Sync> = Arc::new(|level| {
        crate::event_bus::emit(
            "microphone:level",
            &serde_json::json!({ "level": level }),
        );
    });
    match crate::recorder::Recorder::start(device, consumer, handler, None) {
        Ok((recorder, _runtime_errors, _archive_active)) => {
            *monitor.lock() = Some(recorder);
            0
        }
        Err(e) => {
            log::warn!("[ffi] 麦克风电平监听启动失败: {e}");
            1
        }
    }
}

/// 停止电平监听（幂等）。
#[no_mangle]
pub extern "C" fn zhunji_stop_microphone_level_monitor() {
    if let Some(monitor) = LEVEL_MONITOR.get() {
        if let Some(recorder) = monitor.lock().take() {
            recorder.stop();
        }
    }
}

// MARK: - 概览页数据（P2a：原版 list_history / get_activity_stats / get_credentials / get_engine_status / test_engine）

/// 历史列表 JSON（原版 list_history）：`[DictationSession...]`（camelCase，同 history.json）。
#[no_mangle]
pub extern "C" fn zhunji_list_history() -> *mut c_char {
    let json = match COORDINATOR.get() {
        Some(c) => serde_json::to_string(&c.history().list().unwrap_or_default())
            .unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#)),
        None => "null".to_string(),
    };
    into_c_string(json)
}

/// 年度活动计数 JSON（原版 get_activity_stats）：`[{"date":"YYYY-MM-DD","count":n}...]`
/// 日期升序。独立于历史内容，清空历史不影响。
#[no_mangle]
pub extern "C" fn zhunji_get_activity_stats() -> *mut c_char {
    let json = match COORDINATOR.get() {
        Some(c) => {
            let days: Vec<crate::types::ActivityDay> = c
                .activity()
                .snapshot()
                .into_iter()
                .map(|(date, count)| crate::types::ActivityDay { date, count })
                .collect();
            serde_json::to_string(&days).unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#))
        }
        None => "null".to_string(),
    };
    into_c_string(json)
}

/// 凭据状态 JSON（原版 get_credentials，概览页引擎卡用）：
/// `{"activeAsrProvider":"...","asrConfigured":bool}`。
#[no_mangle]
pub extern "C" fn zhunji_get_credentials() -> *mut c_char {
    let active_asr_provider = COORDINATOR
        .get()
        .map(|c| c.prefs().get().active_asr_provider)
        .unwrap_or_default();
    let asr_configured = asr_configured_for_provider(&active_asr_provider);
    into_c_string(
        serde_json::json!({
            "activeAsrProvider": active_asr_provider,
            "asrConfigured": asr_configured,
        })
        .to_string(),
    )
}

/// 原版 asr_configured_for_provider（credentials.rs）：豆包常驻 true；
/// Grok STT 看凭据文件；第三方供应商看 providers.json 的 URL 是否配置。
fn asr_configured_for_provider(provider: &str) -> bool {
    if provider == "builtin-doubao" || provider == crate::asr::doubao::PROVIDER_ID {
        return true;
    }
    if provider == crate::asr::grok_stt::PROVIDER_ID {
        return crate::asr::grok_stt::load_credentials_file().configured();
    }
    if let Ok(list) = crate::providers::list_providers() {
        if let Some(p) = list.into_iter().find(|p| p.id == provider) {
            return !p.url.is_empty();
        }
    }
    false
}

/// 引擎上次会话状态 JSON（原版 get_engine_status）：`{"ok":bool,"error":string|null}`。
#[no_mangle]
pub extern "C" fn zhunji_get_engine_status() -> *mut c_char {
    let json = match COORDINATOR.get() {
        Some(c) => {
            let s = c.engine_status();
            serde_json::json!({ "ok": s.ok, "error": s.error }).to_string()
        }
        None => "null".to_string(),
    };
    into_c_string(json)
}

// MARK: - 历史页（P2：原版 commands/history.rs）

/// UUID-v4 字面校验（原版 commands/mod.rs 的 is_valid_session_id）：
/// 36 字符 + 4 个 `-`（8-4-4-4-12）+ 仅 ASCII 十六进制。白名单胜过黑名单，
/// 挡掉所有 Path::join 越界的可能（IPC = boundary，按 boundary 规则严格校验）。
fn is_valid_session_id(s: &str) -> bool {
    if s.len() != 36 {
        return false;
    }
    for (i, b) in s.as_bytes().iter().enumerate() {
        let is_dash_position = matches!(i, 8 | 13 | 18 | 23);
        if is_dash_position {
            if *b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// 删除一条历史（原版 delete_history_entry）。删除后发 `history:changed`
/// 事件（概览/历史页常驻挂载，实时刷新）。返回 0 成功，1 未初始化，2 删除失败。
#[no_mangle]
pub extern "C" fn zhunji_delete_history_entry(id: *const c_char) -> i32 {
    let Some(coordinator) = COORDINATOR.get() else {
        return 1;
    };
    let id = unsafe { read_c_string(id) };
    if let Err(e) = coordinator.history().delete(&id) {
        log::warn!("[ffi] delete_history_entry 失败: {e}");
        return 2;
    }
    crate::event_bus::emit_unit("history:changed");
    0
}

/// 清空历史（原版 clear_history）。完成后发 `history:changed` 事件。
/// 返回 0 成功，1 未初始化，2 清空失败。
#[no_mangle]
pub extern "C" fn zhunji_clear_history() -> i32 {
    let Some(coordinator) = COORDINATOR.get() else {
        return 1;
    };
    if let Err(e) = coordinator.history().clear() {
        log::warn!("[ffi] clear_history 失败: {e}");
        return 2;
    }
    crate::event_bus::emit_unit("history:changed");
    0
}

/// 读取某次会话的原始麦克风 wav 字节流，返回 data URL（原版 read_audio_recording）。
/// 文件名规约 `<data_dir>/recordings/<session_id>.wav`，与 DictationSession.id 同名。
/// 返回 JSON：`{"data":"data:audio/wav;base64,..."}` 或 `{"error":"recording not found"}`。
#[no_mangle]
pub extern "C" fn zhunji_read_audio_recording(session_id: *const c_char) -> *mut c_char {
    let session_id = unsafe { read_c_string(session_id) };
    if !is_valid_session_id(&session_id) {
        return into_c_string(r#"{"error":"invalid session id"}"#.into());
    }
    let path = match crate::persistence::recording_path_for_session(&session_id) {
        Ok(p) => p,
        Err(e) => {
            return into_c_string(format!(r#"{{"error":"{e}"}}"#));
        }
    };
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return into_c_string(r#"{"error":"recording not found"}"#.into());
        }
        Err(e) => {
            return into_c_string(format!(r#"{{"error":"read wav failed: {e}"}}"#));
        }
    };
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &data);
    into_c_string(format!("{{\"data\":\"data:audio/wav;base64,{b64}\"}}"))
}

/// 把一条「转录失败」历史条目的归档录音用当前 ASR provider 重新转录（原版
/// retranscribe_recording，issue #613）。异步：读 `recordings/<id>.wav` → 取 PCM
/// （跳过 44 字节 WAV 头）→ 现 provider 重转 → 成功则原地回写 rawTranscript /
/// finalText、清除 errorCode（润色字段一并清掉，避免旧信息挂在新转写上）。
/// 完成发 `history:retranscribed` 事件：成功 `{"id","entry":{...}}`（整条记录供
/// 前端局部刷新），失败 `{"id","error"}`。返回 0 已发起，1 未初始化，2 id 非法。
#[no_mangle]
pub extern "C" fn zhunji_retranscribe_recording(session_id: *const c_char) -> i32 {
    let Some(coordinator) = COORDINATOR.get() else {
        return 1;
    };
    let session_id = unsafe { read_c_string(session_id) };
    if !is_valid_session_id(&session_id) {
        return 2;
    }
    let coordinator = coordinator.clone();
    crate::runtime().spawn(async move {
        let emit_err = |err: String| {
            crate::event_bus::emit(
                "history:retranscribed",
                &serde_json::json!({ "id": session_id, "error": err }),
            );
        };
        let path = match crate::persistence::recording_path_for_session(&session_id) {
            Ok(p) => p,
            Err(e) => return emit_err(e.to_string()),
        };
        let wav = match tokio::fs::read(&path).await {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return emit_err("recording not found".into());
            }
            Err(e) => return emit_err(format!("read wav failed: {e}")),
        };
        // 归档 wav 是 16k/mono/16-bit、固定 44 字节标准头。
        if wav.len() <= 44 {
            return emit_err("recording is empty or corrupt".into());
        }
        let pcm = wav[44..].to_vec();

        let retranscribe_started = std::time::Instant::now();
        let (text, label) = match coordinator.retranscribe_pcm(pcm).await {
            Ok(v) => v,
            Err(e) => return emit_err(e),
        };
        if text.trim().is_empty() {
            return emit_err("重新转录仍未识别到语音".into());
        }
        let asr_ms = retranscribe_started.elapsed().as_millis() as u64;

        // 找到原条目，保留其它字段，只更新转写结果 + 清错误码（原版 apply_retranscription）。
        let list = match coordinator.history().list() {
            Ok(l) => l,
            Err(e) => return emit_err(e.to_string()),
        };
        let mut entry = match list.into_iter().find(|s| s.id == session_id) {
            Some(e) => e,
            None => return emit_err("history entry not found".into()),
        };
        entry.raw_transcript = text.clone();
        entry.final_text = text;
        entry.error_code = None;
        entry.asr_provider = Some(label.provider.clone());
        entry.asr_model = label.model.clone();
        entry.asr_ms = Some(asr_ms);
        // 重转没有润色环节：清掉 llm_* / polish_ms，避免详情页把旧润色信息错挂在新转写上。
        entry.llm_provider = None;
        entry.llm_model = None;
        entry.polish_ms = None;
        if let Err(e) = coordinator.history().update_entry(entry.clone()) {
            return emit_err(format!("history update failed: {e}"));
        }
        crate::event_bus::emit(
            "history:retranscribed",
            &serde_json::json!({ "id": session_id, "entry": entry }),
        );
    });
    0
}

/// 把归档录音 wav 导出到目标路径（原版 export_audio_recording；对话框由 Swift 侧
/// NSSavePanel 负责，core 只做校验 + 复制）。返回 0 成功，1 未初始化，2 id 非法，
/// 3 录音不存在，4 复制失败。
#[no_mangle]
pub extern "C" fn zhunji_export_audio_recording(
    session_id: *const c_char,
    dest_path: *const c_char,
) -> i32 {
    let session_id = unsafe { read_c_string(session_id) };
    let dest_path = unsafe { read_c_string(dest_path) };
    if !is_valid_session_id(&session_id) {
        return 2;
    }
    if dest_path.is_empty() {
        return 4;
    }
    let source = match crate::persistence::recording_path_for_session(&session_id) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("[ffi] export_audio_recording 路径解析失败: {e}");
            return 4;
        }
    };
    let mut src_file = match std::fs::File::open(&source) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return 3,
        Err(e) => {
            log::warn!("[ffi] export_audio_recording 打开失败: {e}");
            return 4;
        }
    };
    let mut dst_file = match std::fs::File::create(&dest_path) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("[ffi] export_audio_recording 创建失败: {e}");
            return 4;
        }
    };
    if std::io::copy(&mut src_file, &mut dst_file).is_err() {
        return 4;
    }
    0
}

// MARK: - 供应商管理页（P2：原版 commands/providers.rs）

/// 新增供应商（原版 add_provider）。入参 JSON：`{"name","url","apiKey","notes"}`
/// （apiKey/notes 可空）。成功返回新 Provider JSON（camelCase），失败 `{"error":"..."}`。
#[no_mangle]
pub extern "C" fn zhunji_add_provider(json: *const c_char) -> *mut c_char {
    let json = unsafe { read_c_string(json) };
    let value: serde_json::Value = match serde_json::from_str(&json) {
        Ok(v) => v,
        Err(_) => return into_c_string(r#"{"error":"参数解析失败"}"#.into()),
    };
    let name = value["name"].as_str().unwrap_or("").to_string();
    let url = value["url"].as_str().unwrap_or("").to_string();
    let api_key = value["apiKey"].as_str().map(|s| s.to_string());
    let notes = value["notes"].as_str().map(|s| s.to_string());
    match crate::providers::add_provider(name, url, api_key, notes) {
        Ok(p) => into_c_string(serde_json::to_string(&p).unwrap_or_default()),
        Err(e) => into_c_string(format!(r#"{{"error":"{e}"}}"#)),
    }
}

/// 更新供应商（原版 update_provider）。入参 JSON：`{"id","name","url","apiKey","notes"}`。
/// 返回 0 成功，1 更新失败。
#[no_mangle]
pub extern "C" fn zhunji_update_provider(json: *const c_char) -> i32 {
    let json = unsafe { read_c_string(json) };
    let value: serde_json::Value = match serde_json::from_str(&json) {
        Ok(v) => v,
        Err(_) => return 1,
    };
    let id = value["id"].as_str().unwrap_or("").to_string();
    let name = value["name"].as_str().unwrap_or("").to_string();
    let url = value["url"].as_str().unwrap_or("").to_string();
    let api_key = value["apiKey"].as_str().map(|s| s.to_string());
    let notes = value["notes"].as_str().map(|s| s.to_string());
    match crate::providers::update_provider(id, name, url, api_key, notes) {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("[ffi] update_provider 失败: {e}");
            1
        }
    }
}

/// 删除供应商（原版 remove_provider；内置豆包不可删，删默认引擎自动切回豆包）。
/// 返回 0 成功，1 失败。
#[no_mangle]
pub extern "C" fn zhunji_remove_provider(id: *const c_char) -> i32 {
    let id = unsafe { read_c_string(id) };
    match crate::providers::remove_provider(id) {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("[ffi] remove_provider 失败: {e}");
            1
        }
    }
}

/// 设为默认供应商（原版 set_default_provider + 前端同步 prefs.activeAsrProvider：
/// 一并更新偏好并发 prefs:changed，设置页引擎下拉随之刷新）。返回 0 成功，1 失败。
#[no_mangle]
pub extern "C" fn zhunji_set_default_provider(id: *const c_char) -> i32 {
    let id = unsafe { read_c_string(id) };
    if let Err(e) = crate::providers::set_default_provider(id.clone()) {
        log::warn!("[ffi] set_default_provider 失败: {e}");
        return 1;
    }
    if let Some(coordinator) = COORDINATOR.get() {
        let mut prefs = coordinator.prefs().get();
        if prefs.active_asr_provider != id {
            prefs.active_asr_provider = id;
            let _ = coordinator.prefs().set(prefs);
        }
    }
    crate::event_bus::emit_unit("prefs:changed");
    0
}

/// 测试供应商连通性（原版 test_provider）：GET `{url}/v1/models`（带 Bearer auth，
/// 10s 超时）。内置豆包无需 URL/API Key——单独走豆包引擎连通性测试
/// （同 test_engine：注册 → token → WS 握手）。异步，完成发 `provider:test-result`
/// 事件 `{"id":"...","ok":bool,"error":string|null}`。返回 0 已发起，1 失败。
#[no_mangle]
pub extern "C" fn zhunji_test_provider(id: *const c_char) -> i32 {
    let id = unsafe { read_c_string(id) };
    // 内置豆包：无 URL/API Key 概念，直接测豆包引擎（用户要求单独处理）。
    if id == "builtin-doubao" || id == crate::asr::doubao::PROVIDER_ID {
        crate::runtime().spawn(async move {
            let asr = Arc::new(crate::asr::DoubaoImeASR::new(None));
            let result: Result<(), String> = match asr.open_session().await {
                Ok(()) => {
                    asr.close();
                    Ok(())
                }
                Err(e) => Err(e.to_string()),
            };
            crate::event_bus::emit(
                "provider:test-result",
                &serde_json::json!({
                    "id": id,
                    "ok": result.is_ok(),
                    "error": result.err(),
                }),
            );
        });
        return 0;
    }
    let provider = match crate::providers::list_providers() {
        Ok(list) => match list.into_iter().find(|p| p.id == id) {
            Some(p) => p,
            None => {
                crate::event_bus::emit(
                    "provider:test-result",
                    &serde_json::json!({ "id": id, "ok": false, "error": "供应商不存在" }),
                );
                return 0;
            }
        },
        Err(e) => {
            crate::event_bus::emit(
                "provider:test-result",
                &serde_json::json!({ "id": id, "ok": false, "error": e }),
            );
            return 0;
        }
    };
    if provider.url.trim().is_empty() {
        crate::event_bus::emit(
            "provider:test-result",
            &serde_json::json!({ "id": id, "ok": false, "error": "请先填写 URL" }),
        );
        return 0;
    }
    let url = format!("{}/v1/models", provider.url.trim_end_matches('/'));
    crate::runtime().spawn(async move {
        let mut req = crate::net::http()
            .get(url)
            .timeout(std::time::Duration::from_secs(10));
        if let Some(key) = provider.api_key.as_ref() {
            req = req.bearer_auth(key);
        }
        let result = match req.send().await {
            Ok(resp) if resp.status().is_success() => Ok(()),
            Ok(resp) => Err(format!("返回 {}", resp.status())),
            Err(e) => Err(format!("网络错误: {e}")),
        };
        crate::event_bus::emit(
            "provider:test-result",
            &serde_json::json!({
                "id": provider.id,
                "ok": result.is_ok(),
                "error": result.err(),
            }),
        );
    });
    0
}

/// 主动检测引擎连通性（原版 test_engine）：豆包真实走一遍注册 → token → WS 握手；
/// 第三方供应商 GET 探测 URL。异步，结果经 `engine:test-result` 事件
/// `{"ok":bool,"error":string|null}` 回调。返回 0 已发起，1 core 未初始化。
#[no_mangle]
pub extern "C" fn zhunji_test_engine() -> i32 {
    let Some(coordinator) = COORDINATOR.get() else {
        return 1;
    };
    let active = coordinator.prefs().get().active_asr_provider;
    crate::runtime().spawn(async move {
        let result: Result<(), String> = if active == "builtin-doubao"
            || active == crate::asr::doubao::PROVIDER_ID
        {
            let asr = Arc::new(crate::asr::DoubaoImeASR::new(None));
            match asr.open_session().await {
                Ok(()) => {
                    asr.close();
                    Ok(())
                }
                Err(e) => Err(e.to_string()),
            }
        } else if let Ok(list) = crate::providers::list_providers() {
            match list.iter().find(|p| p.id == active) {
                Some(p) if p.url.is_empty() => Err("供应商未配置 URL".into()),
                Some(p) => {
                    let url = p.url.trim().trim_end_matches('/');
                    match crate::net::http()
                        .get(url)
                        .timeout(std::time::Duration::from_secs(8))
                        .send()
                        .await
                    {
                        Ok(_) => Ok(()),
                        Err(e) => Err(format!("连接失败：{e}")),
                    }
                }
                None => Err("未找到当前 ASR 供应商".into()),
            }
        } else {
            Err("供应商列表读取失败".into())
        };
        let payload = serde_json::json!({
            "ok": result.is_ok(),
            "error": result.err(),
        });
        crate::event_bus::emit("engine:test-result", &payload);
    });
    0
}

// MARK: - 词典页（P2：原版 commands/dictionary.rs）

/// 热词列表 JSON 数组 `["词1","词2"]`（空词典返回 `[]`）。
#[no_mangle]
pub extern "C" fn zhunji_list_terms() -> *mut c_char {
    match crate::dictionary::list_terms() {
        Ok(list) => into_c_string(serde_json::to_string(&list).unwrap_or_default()),
        Err(e) => into_c_string(format!(r#"{{"error":"{e}"}}"#)),
    }
}

/// 新增热词（原版 add_term：trim + 非空 + ≤50 字符 + 去重 + ≤100 条）。
/// 返回 `{"ok":true}` 或 `{"error":"..."}`（前端红字显示，同原版 error state）。
#[no_mangle]
pub extern "C" fn zhunji_add_term(term: *const c_char) -> *mut c_char {
    let term = unsafe { read_c_string(term) };
    match crate::dictionary::add_term(term) {
        Ok(()) => into_c_string(r#"{"ok":true}"#.into()),
        Err(e) => into_c_string(format!(r#"{{"error":"{e}"}}"#)),
    }
}

/// 删除热词（原版 remove_term）。返回 0 成功，1 失败。
#[no_mangle]
pub extern "C" fn zhunji_remove_term(term: *const c_char) -> i32 {
    let term = unsafe { read_c_string(term) };
    match crate::dictionary::remove_term(term) {
        Ok(()) => 0,
        Err(e) => {
            log::warn!("[ffi] remove_term 失败: {e}");
            1
        }
    }
}

// MARK: - 高级页（P2：原版 DebugToolsSection / misc.rs export_error_log）

/// 导出错误日志：复制 `~/Library/Logs/Zhunji/zhunji.log` 到目标路径
/// （NSSavePanel 在 Swift 侧）。返回 `{"ok":true}` 或 `{"error":"..."}`。
#[no_mangle]
pub extern "C" fn zhunji_export_error_log(target: *const c_char) -> *mut c_char {
    let target = unsafe { read_c_string(target) };
    let src = crate::logging::log_dir_path().join("zhunji.log");
    if !src.exists() {
        return into_c_string(
            format!(r#"{{"error":"日志文件不存在：{}"}}"#, src.display()),
        );
    }
    match std::fs::copy(&src, std::path::Path::new(&target)) {
        Ok(_) => into_c_string(r#"{"ok":true}"#.into()),
        Err(e) => into_c_string(format!(r#"{{"error":"复制日志失败：{e}"}}"#)),
    }
}
