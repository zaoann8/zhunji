//! Grok STT — 经 worker-search 中转的免费通道 provider（非流式）。
//!
//! 录音期间累积 16k/16bit/mono PCM（recorder 输出），结束时封装 WAV 头，
//! 一次性 POST 到 `{baseURL}/v1/audio/transcriptions`（OpenAI 兼容 multipart），
//! worker-search 侧用 SSO token 池转发 grok.com web STT 接口并自动换号/冷却。
//!
//! 凭据（endpoint + apiKey）独立存 `~/.doudou_mac_grok_stt.json`——
//! desktop 的 CredentialsVault 是内存态（豆包引擎版裁掉了 Keychain），
//! 重启会丢；沿用豆包 `~/.doudou_mac_doubao.json` 的文件持久化先例。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::oneshot;

/// doudou provider 注册 id（对应 prefs.asrProvider 存的值）。
pub const PROVIDER_ID: &str = "grok_stt";

pub fn is_grok_stt(id: &str) -> bool {
    id == PROVIDER_ID
}

// MARK: - 错误

#[derive(Debug, Error)]
pub enum GrokSttError {
    #[error("未配置{0}，请在设置中填写")]
    MissingCredentials(String),
    #[error("请求失败: {0}")]
    Request(String),
    #[error("上游错误 {0}: {1}")]
    Upstream(u16, String),
    #[error("上游返回为空")]
    EmptyResponse,
    #[error("等待结果超时")]
    Timeout,
    #[error("通道中断: {0}")]
    Channel(String),
}

// MARK: - 会话内部状态

struct SyncState {
    /// 累积的 16k/16bit/mono PCM 字节。
    pcm: Vec<u8>,
    timestamp_base_ms: u64,
    /// send_last_frame 启动的请求结果通道（tx 移交给请求任务，rx 供 await_final_result 取）。
    result_rx: Option<oneshot::Receiver<Result<String, GrokSttError>>>,
}

impl Default for SyncState {
    fn default() -> Self {
        Self {
            pcm: Vec::new(),
            timestamp_base_ms: 0,
            result_rx: None,
        }
    }
}

// MARK: - Provider

pub struct GrokSttASR {
    state: Mutex<SyncState>,
}

impl GrokSttASR {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(SyncState::default()),
        })
    }

    /// 启动时预热并常驻保活：校验凭据后 spawn 循环，每 60s 一次 /health ping。
    /// 60s < net.rs 连接池 idle 回收线（90s）→ 连接持续活跃，任意时刻说话都免握手。
    /// 网络中断时 ping 失败静默，恢复后下个周期自动重连。
    pub async fn warmup(self: &Arc<Self>) {
        if let Err(e) = check_credentials() {
            log::warn!("[grok_stt] warmup credentials missing: {e}");
            return;
        }
        let t = Instant::now();
        self.preheat().await;
        log::info!("[grok_stt] 首次预热完成: {}ms", t.elapsed().as_millis());
        let asr = Arc::clone(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(60));
            let mut count = 0u32;
            loop {
                tick.tick().await;
                count += 1;
                let t = Instant::now();
                asr.preheat().await;
                // 单次成功仍走 debug；每 10 次(10分钟)一条 info 证明保活在工作。
                if count % 10 == 0 {
                    log::info!(
                        "[grok_stt] 保活第 {count} 次: {}ms",
                        t.elapsed().as_millis()
                    );
                }
            }
        });
    }

    /// 预热到 worker-search：GET /v1/dpop/ping 建连（TCP+TLS 进连接池）并让
    /// worker 侧把 DPoP session 提前换好进缓存——STT 首请求也免握手、免换 token。
    /// 端点不存在（旧版 worker）时失败静默降级。失败静默（只是优化）。
    async fn preheat(self: &Arc<Self>) {
        let Ok((endpoint, api_key)) = credentials() else { return };
        let url = format!("{}/v1/dpop/ping", endpoint.trim_end_matches('/'));
        let t = Instant::now();
        match crate::net::http()
            .get(&url)
            .bearer_auth(api_key)
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) => log::debug!(
                "[grok_stt] preheat ok: {}ms status={}",
                t.elapsed().as_millis(),
                resp.status()
            ),
            Err(e) => log::warn!("[grok_stt] preheat failed: {e}"),
        }
    }

    pub async fn open_session(self: &Arc<Self>) -> Result<(), GrokSttError> {
        check_credentials()?;
        // 用户正按住热键说话：趁机预热连接，松手发 STT 时免握手。不阻塞会话启动。
        let asr = Arc::clone(self);
        tokio::spawn(async move {
            asr.preheat().await;
        });
        // 新会话：清掉上一轮的残留状态。
        let mut st = self.state.lock();
        st.pcm.clear();
        st.result_rx = None;
        Ok(())
    }

    /// 累积 PCM 封装 WAV → 后台 POST → 结果经 oneshot 交回 await_final_result。
    pub async fn send_last_frame(&self) -> Result<(), GrokSttError> {
        let (endpoint, api_key) = credentials()?;
        let (pcm, tx) = {
            let mut st = self.state.lock();
            if st.pcm.is_empty() {
                // 没录到音频：与豆包一致，干净结束（await 立即拿到空文本），不报错。
                let (tx, rx) = oneshot::channel();
                let _ = tx.send(Ok(String::new()));
                st.result_rx = Some(rx);
                return Ok(());
            }
            let pcm = std::mem::take(&mut st.pcm);
            let (tx, rx) = oneshot::channel();
            st.result_rx = Some(rx);
            (pcm, tx)
        };

        tokio::spawn(async move {
            let keyterms = crate::dictionary::get_terms();
            let result = transcribe(endpoint, api_key, &pcm, &keyterms).await;
            // rx 已被 cancel()/await 取走时 send 失败，静默即可。
            let _ = tx.send(result);
        });
        Ok(())
    }

    /// 等 send_last_frame 启动的请求完成，返回最终文本。
    pub async fn await_final_result(&self) -> Result<String, GrokSttError> {
        let rx = {
            let mut st = self.state.lock();
            st.result_rx.take()
        };
        let Some(rx) = rx else {
            return Err(GrokSttError::Channel("没有在途请求".into()));
        };
        tokio::time::timeout(Duration::from_secs(20), rx)
            .await
            .map_err(|_| GrokSttError::Timeout)?
            .map_err(|_| GrokSttError::Channel("发送端已丢弃".into()))?
    }

    /// 取消会话：丢弃结果通道，在途请求随任务自然结束。
    pub fn cancel(self: &Arc<Self>) {
        let mut st = self.state.lock();
        st.result_rx = None;
    }

    /// 校准帧时间戳基准为录音开始时刻（缓冲音频在握手前已录制，见 DeferredAsrBridge）。
    pub fn set_timestamp_base(&self, ms: u64) {
        self.state.lock().timestamp_base_ms = ms;
    }

    /// 本次会话实际录音时长（ms）= 现在 − 录音起点。供 history.durationMs 使用。
    pub fn session_duration_ms(&self) -> u64 {
        let base = self.state.lock().timestamp_base_ms;
        if base == 0 {
            return 0;
        }
        unix_ms().saturating_sub(base)
    }

    /// 会话结束：丢弃在途状态（下一轮 open_session 会重建）。
    pub fn close(self: &Arc<Self>) {
        let mut st = self.state.lock();
        st.pcm.clear();
        st.result_rx = None;
    }
}

// asr/mod.rs 的 AudioConsumer（doudou 双 trait 约定：recorder 推流 / asr 灌入）。
impl crate::asr::AudioConsumer for GrokSttASR {
    /// recorder 推来的 16k/16bit/mono PCM 字节流 → 累积，结束时封装 WAV 一次性发送。
    fn consume_pcm_chunk(&self, pcm: &[u8]) {
        self.state.lock().pcm.extend_from_slice(pcm);
    }
}

// MARK: - 凭据

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GrokSttCredentials {
    pub endpoint: String,
    pub api_key: String,
}

impl GrokSttCredentials {
    #[allow(dead_code)] // P2 设置页凭据状态卡
    pub fn configured(&self) -> bool {
        !self.endpoint.trim().is_empty() && !self.api_key.trim().is_empty()
    }
}

fn credentials_path() -> PathBuf {
    std::env::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".doudou_mac_grok_stt.json")
}

/// 读凭据文件（不存在/损坏 → 空凭据）。
pub fn load_credentials_file() -> GrokSttCredentials {
    std::fs::read(credentials_path())
        .ok()
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or_default()
}

/// 写凭据文件（设置页保存用）。
pub fn save_credentials_file(endpoint: &str, api_key: &str) -> Result<(), String> {
    let creds = GrokSttCredentials {
        endpoint: endpoint.trim().to_string(),
        api_key: api_key.trim().to_string(),
    };
    let data = serde_json::to_vec(&creds).map_err(|e| e.to_string())?;
    std::fs::write(credentials_path(), data).map_err(|e| e.to_string())
}

fn check_credentials() -> Result<(), GrokSttError> {
    credentials().map(|_| ())
}

fn credentials() -> Result<(String, String), GrokSttError> {
    let creds = load_credentials_file();
    if creds.endpoint.trim().is_empty() {
        return Err(GrokSttError::MissingCredentials("端点(baseURL)".into()));
    }
    if creds.api_key.trim().is_empty() {
        return Err(GrokSttError::MissingCredentials("API Key".into()));
    }
    Ok((
        creds.endpoint.trim().to_string(),
        creds.api_key.trim().to_string(),
    ))
}

// MARK: - 请求

/// 16k/16bit/mono PCM → WAV（RIFF 头 + 数据）。
fn wav_from_pcm(pcm: &[u8]) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let mut wav = Vec::with_capacity(pcm.len() + 44);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&16_000u32.to_le_bytes()); // sample rate
    wav.extend_from_slice(&32_000u32.to_le_bytes()); // byte rate = 16000 * 2
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm);
    wav
}

async fn transcribe(
    base_url: String,
    api_key: String,
    pcm: &[u8],
    keyterms: &[String],
) -> Result<String, GrokSttError> {
    let wav = wav_from_pcm(pcm);
    let part = reqwest::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| GrokSttError::Request(e.to_string()))?;
    // xAI STT 文档：「Option fields should precede `file` in the multipart body —
    // for streamable uploads, fields sent after `file` may be ignored.」
    // 此前 file 排第一、keyterm/prompt 排后面 → 上游静默丢弃 → 热词从未生效。
    let mut form = reqwest::multipart::Form::new();
    if !keyterms.is_empty() {
        // keyterm 是 xAI STT 官方热词参数（bias transcription toward specific
        // terms，重复字段传多个词）；prompt 是 OpenAI 兼容网关的标准参数，
        // 自建网关（one-api/grok2api 等）场景下生效，xAI 忽略无害。
        form = form
            .text("keyterm", keyterms.join(","))
            .text("prompt", keyterms.join(", "));
    }
    let form = form.text("model", "grok-stt").part("file", part);
    let url = format!("{}/v1/audio/transcriptions", base_url.trim_end_matches('/'));

    // 观测日志：请求网络耗时 = 连接(复用?)+上传+worker 转发+上游 STT，对比首次/后续差异。
    let t_req = Instant::now();
    let resp = crate::net::http()
        .post(&url)
        .bearer_auth(api_key)
        .multipart(form)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| GrokSttError::Request(e.to_string()))?;
    log::info!(
        "[grok_stt] 请求耗时 {}ms (音频 {}KB)",
        t_req.elapsed().as_millis(),
        pcm.len() / 1024
    );

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(GrokSttError::Upstream(
            status.as_u16(),
            body.chars().take(300).collect(),
        ));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| GrokSttError::Request(e.to_string()))?;
    let text = json
        .get("text")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or(GrokSttError::EmptyResponse)?;
    log::info!(
        "[grok_stt] 转写完成: 总 {}ms, 文本 {} 字符",
        t_req.elapsed().as_millis(),
        text.chars().count()
    );
    Ok(text)
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::AudioConsumer;

    /// 端到端引擎验证（真实网络）：读真实凭据文件 → 喂 1 秒静音 → WAV 封装 →
    /// POST worker /v1/audio/transcriptions → 拿回文本。静音可能返回空文本，链路通即可。
    /// 手动运行：cargo test --lib -- --ignored --nocapture asr::grok_stt::tests::engine
    #[tokio::test]
    #[ignore]
    async fn engine_end_to_end() {
        if !load_credentials_file().configured() {
            panic!("凭据文件未配置：~/.doudou_mac_grok_stt.json（endpoint + apiKey）");
        }
        let asr = GrokSttASR::new();

        let outcome = tokio::time::timeout(Duration::from_secs(60), async {
            println!("[1/4] 校验凭据…");
            asr.open_session().await.expect("open_session 失败");
            println!("[2/4] ✅ 凭据配齐");

            // 1 秒静音（16000 samples，16bit mono）
            let silence: Vec<u8> = (0..16000i16)
                .map(|_| 0i16)
                .flat_map(|s| s.to_le_bytes())
                .collect();
            for chunk in silence.chunks(3200) {
                asr.consume_pcm_chunk(chunk);
            }
            println!("[3/4] 发尾帧（WAV 封装 + POST）…");
            asr.send_last_frame().await.expect("send_last_frame 失败");
            println!("[4/4] 等最终结果…");
            let text = asr
                .await_final_result()
                .await
                .expect("await_final_result 失败");
            println!("RESULT_TEXT: [{text}]");
        })
        .await;

        asr.close();
        if outcome.is_err() {
            panic!("engine test 整体超时(60s)");
        }
    }
}
