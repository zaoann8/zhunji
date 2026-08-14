//! ASR engine wiring, credential/permission gates, and release scheduling
//! extracted from `coordinator.rs` (behavior-preserving move).
//!
//! References parent items via `use super::*;`; `pub(super)` so the parent
//! `coordinator` module reaches them through `use asr_wiring::*;`.

use super::*;

#[cfg(any(debug_assertions, test))]
pub(super) fn hotkey_injection_dry_run_enabled() -> bool {
    std::env::var_os("OPENLESS_HOTKEY_INJECTION_DRY_RUN").is_some()
}

#[cfg(any(debug_assertions, test))]
pub(super) fn debug_transcript_override_text() -> Option<String> {
    let path = std::env::var_os("OPENLESS_DEBUG_TRANSCRIPT_FILE")?;
    let text = std::fs::read_to_string(path).ok()?;
    let trimmed = text.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

pub(super) fn ensure_microphone_permission(_inner: &Arc<Inner>) -> Result<(), String> {
    use crate::permissions::{self, PermissionStatus};

    #[cfg(target_os = "windows")]
    {
        if permissions::windows_microphone_access_explicitly_denied() {
            return Err("需要麦克风权限，当前状态: Denied".to_string());
        }
        // 注册表只反映隐私开关；没插麦克风时不能当成“已就绪”，
        // 否则用户会被误导去系统设置找不存在的麦克风权限。见 issue #779。
        if permissions::has_microphone_input_device() {
            return Ok(());
        }
        return Err("未检测到麦克风，请连接麦克风后重试".to_string());
    }

    let status = permissions::check_microphone();
    if matches!(
        status,
        PermissionStatus::Granted | PermissionStatus::NotApplicable
    ) {
        return Ok(());
    }
    if status == PermissionStatus::NoDevice {
        return Err("未检测到麦克风，请连接麦克风后重试".to_string());
    }

    // 听写路径不抢前台焦点：缺 mic 权限时直接请求系统授权，不再先 show_main_window。
    // 用户在设置页手动点“请求权限”仍走 request_microphone_from_foreground，那是显式操作。
    // 这里若系统不弹框，后续会通过 capsule error 引导用户主动去权限页处理。详见 #166。
    let requested = permissions::request_microphone();
    if matches!(
        requested,
        PermissionStatus::Granted | PermissionStatus::NotApplicable
    ) {
        Ok(())
    } else {
        Err(format!("需要麦克风权限，当前状态: {requested:?}"))
    }
}

pub(super) fn ensure_asr_credentials(active_asr: &str) -> Result<(), String> {

    // 豆包 IME：内置免费引擎，无需凭据
    if active_asr == "builtin-doubao" || crate::asr::doubao::is_doubao(&active_asr) {
        return Ok(());
    }

    // 第三方供应商：从 providers.json 查
    if let Some(p) = find_provider(&active_asr) {
        if p.url.is_empty() {
            return Err(format!("供应商「{}」未配置 URL", p.name));
        }
        return Ok(());
    }

    Err(format!("未找到语音引擎「{active_asr}」，请在供应商页面添加"))
}

pub(super) fn find_provider(id: &str) -> Option<crate::providers::Provider> {
    let list = crate::providers::list_providers().ok()?;
    list.into_iter().find(|p| p.id == id)
}

pub(super) fn apply_chinese_script_preference(text: &str, pref: ChineseScriptPreference) -> String {
    if text.is_empty() {
        return String::new();
    }
    let config = match pref {
        ChineseScriptPreference::Simplified => Some(BuiltinConfig::T2s),
        ChineseScriptPreference::Traditional => Some(BuiltinConfig::S2t),
        ChineseScriptPreference::Auto => None,
    };
    let Some(config) = config else {
        return text.to_string();
    };
    match OpenCC::from_config(config) {
        Ok(converter) => converter.convert(text),
        Err(err) => {
            log::warn!("[coord] OpenCC init failed, skip script conversion: {err}");
            text.to_string()
        }
    }
}

/// QA / 重转录用的独立 ASR 会话句柄（与主听写路径复用常驻实例互不抢占）。
pub(super) enum AsrSessionHandle {
    Doubao(Arc<crate::asr::DoubaoImeASR>),
    GrokStt(Arc<crate::asr::GrokSttASR>),
}

impl AsrSessionHandle {
    async fn open_session(&self) -> Result<(), String> {
        match self {
            AsrSessionHandle::Doubao(asr) => asr.open_session().await.map_err(|e| e.to_string()),
            AsrSessionHandle::GrokStt(asr) => asr.open_session().await.map_err(|e| e.to_string()),
        }
    }
}

pub(super) enum QaAsrStart {
    Ready {
        active: ActiveAsr,
        consumer: Arc<dyn crate::recorder::AudioConsumer>,
        asr: AsrSessionHandle,
        bridge: Arc<DeferredAsrBridge>,
    },
}

impl QaAsrStart {
    pub(super) fn active_asr(&self) -> ActiveAsr {
        match self {
            QaAsrStart::Ready { active, .. } => active.clone(),
        }
    }

    pub(super) fn recorder_consumer(&self) -> Arc<dyn crate::recorder::AudioConsumer> {
        match self {
            QaAsrStart::Ready { consumer, .. } => Arc::clone(consumer),
        }
    }

    pub(super) async fn open_streaming_session(&self) -> Result<(), String> {
        match self {
            QaAsrStart::Ready { asr, bridge, .. } => {
                asr.open_session().await?;
                let target: Arc<dyn crate::asr::AudioConsumer> = match asr {
                    AsrSessionHandle::Doubao(a) => Arc::clone(a) as _,
                    AsrSessionHandle::GrokStt(a) => Arc::clone(a) as _,
                };
                let flushed = bridge.attach(target);
                log::info!("[coord] QA ASR connected; flushed {flushed} deferred audio bytes");
                Ok(())
            }
        }
    }
}

/// 返回 (启动器, 构建时 (provider, model) 快照)。快照供 QA / 重转录把「实际用了哪个
/// 模型」写回历史（PR #826 review：归因必须来自构建现场，不能事后重读设置）。
pub(super) async fn build_qa_asr_start(
    _inner: &Arc<Inner>,
    active_asr: &str,
) -> Result<(QaAsrStart, AsrCallLabel), String> {
    // QA 会话用独立实例，与主听写路径（复用 inner.doubao / inner.grok_stt 常驻实例）
    // 互不抢占会话。
    let is_third_party = crate::asr::grok_stt::is_grok_stt(active_asr)
        || find_provider(active_asr).is_some();
    if is_third_party {
        // 第三方供应商：从 providers.json 获取凭据并写入 grok_stt 凭据文件
        if !crate::asr::grok_stt::is_grok_stt(active_asr) {
            if let Some(p) = find_provider(active_asr) {
                let _ = crate::asr::grok_stt::save_credentials_file(
                    &p.url,
                    p.api_key.as_deref().unwrap_or(""),
                );
            }
        }
        let asr = crate::asr::GrokSttASR::new();
        let bridge = Arc::new(DeferredAsrBridge::new());
        let consumer: Arc<dyn crate::recorder::AudioConsumer> = bridge.clone();
        let active = ActiveAsr::GrokStt(Arc::clone(&asr));
        let label = AsrCallLabel::new(crate::asr::grok_stt::PROVIDER_ID.to_string(), None);
        return Ok((
            QaAsrStart::Ready {
                active,
                consumer,
                asr: AsrSessionHandle::GrokStt(asr),
                bridge,
            },
            label,
        ));
    }

    let asr = crate::asr::DoubaoImeASR::new(None);
    let bridge = Arc::new(DeferredAsrBridge::new());
    let consumer: Arc<dyn crate::recorder::AudioConsumer> = bridge.clone();
    let active = ActiveAsr::Doubao(Arc::clone(&asr));
    let label = AsrCallLabel::new(crate::asr::doubao::PROVIDER_ID.to_string(), None);
    Ok((
        QaAsrStart::Ready {
            active,
            consumer,
            asr: AsrSessionHandle::Doubao(asr),
            bridge,
        },
        label,
    ))
}
