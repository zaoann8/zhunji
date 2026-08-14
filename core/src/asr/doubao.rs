//! 豆包 IME 免费通道 ASR provider（非官方协议，从已验证的 Swift Demo 移植）。
//!
//! 流程：设备注册(log.snssdk.com) → 取 token(is.snssdk.com, x-ss-stub=MD5) →
//! WebSocket(frontier-audio-ime-ws.doubao.com) → StartTask/StartSession →
//! TaskRequest(Opus 帧) → FinishSession。
//!
//! 凭证自动注册并缓存到 `~/.doudou_mac_doubao.json`，用户零配置。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use super::doubao_proto;
use crate::recorder::AudioConsumer;

// MARK: - 常量（照搬 Swift Demo，已实测可用）

const REGISTER_HOST: &str = "log.snssdk.com";
const SETTINGS_HOST: &str = "is.snssdk.com";
const WS_HOST: &str = "frontier-audio-ime-ws.doubao.com";
const WS_PATH: &str = "/ocean/api/v1/ws";
const AID: u32 = 401734;
const VERSION_CODE: u32 = 100102018;
const USER_AGENT: &str = "com.bytedance.android.doubaoime/100102018 (Linux; U; Android 16; en_US; Pixel 7 Pro; Build/BP2A.250605.031.A2; Cronet/TTNetVersion:94cf429a 2025-11-17 QuicVersion:1f89f732 2025-05-08)";
const FRAME_SAMPLES: usize = 320; // 20ms @16k
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
const FINAL_TIMEOUT: Duration = Duration::from_secs(10);

// MARK: - 错误

/// doudou provider 注册 id（对应 prefs.asrProvider 存的值）。
pub const PROVIDER_ID: &str = "doubao";

pub fn is_doubao(id: &str) -> bool {
    id == PROVIDER_ID
}

/// 服务端并发配额错误（40200011，免费通道 5 路并发上限）。这类错误下连接本身是
/// 健康的，重连无意义；识别出来快速失败并给用户可操作的提示。
fn is_quota_error(e: &DoubaoError) -> bool {
    e.to_string().contains("40200011") || e.to_string().contains("concurrency quota")
}

#[derive(Debug, Error)]
pub enum DoubaoError {
    #[error("设备注册失败: {0}")]
    Register(String),
    #[error("获取 token 失败: {0}")]
    Token(String),
    #[error("WebSocket 连接失败: {0}")]
    Connect(String),
    #[error("会话失败: {0}")]
    Session(String),
    #[error("Opus 编码失败: {0}")]
    Opus(String),
    #[error("传输中断: {0}")]
    Transport(String),
    #[error("等待 {0} 超时")]
    Timeout(String),
}

// MARK: - 凭据

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DoubaoImeCredentials {
    pub device_id: String,
    pub cdid: String,
    pub token: String,
}

impl DoubaoImeCredentials {
    pub fn auth_ok(&self) -> bool {
        !self.device_id.is_empty() && !self.cdid.is_empty() && !self.token.is_empty()
    }
}

// MARK: - 会话内部状态

struct SyncState {
    pending: Vec<i16>,
    frame_index: u64,
    timestamp_base_ms: u64,
    request_id: String,
    encoder: Option<opus::Encoder>,
    final_accumulator: String,
    last_final_segment: String,
}

impl Default for SyncState {
    fn default() -> Self {
        Self {
            pending: Vec::with_capacity(FRAME_SAMPLES * 4),
            frame_index: 0,
            timestamp_base_ms: 0,
            request_id: String::new(),
            encoder: None,
            final_accumulator: String::new(),
            last_final_segment: String::new(),
        }
    }
}

// MARK: - Provider

pub struct DoubaoImeASR {
    credentials: Mutex<DoubaoImeCredentials>,
    credentials_path: PathBuf,
    state: Mutex<SyncState>,
    connected: AtomicBool,
    /// 连接进行中标志（single-flight：warmup 与 open_session 并发时只连一次）。
    connecting: AtomicBool,
    send_tx: Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>,
    writer_task: Mutex<Option<JoinHandle<()>>>,
    receive_task: Mutex<Option<JoinHandle<()>>>,
    // 消息等待表：key 是 messageType，握手 / 最终结果都走这里
    waiters: Mutex<HashMap<String, oneshot::Sender<Result<String, String>>>>,
    on_partial: Option<Arc<dyn Fn(String) + Send + Sync>>,
}

impl DoubaoImeASR {
    pub fn new(on_partial: Option<Arc<dyn Fn(String) + Send + Sync>>) -> Arc<Self> {
        let credentials_path = std::env::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".doudou_mac_doubao.json");
        Arc::new(Self {
            credentials: Mutex::new(DoubaoImeCredentials::default()),
            credentials_path,
            state: Mutex::new(SyncState::default()),
            connected: AtomicBool::new(false),
            connecting: AtomicBool::new(false),
            send_tx: Mutex::new(None),
            writer_task: Mutex::new(None),
            receive_task: Mutex::new(None),
            waiters: Mutex::new(HashMap::new()),
            on_partial,
        })
    }

    /// 打开会话：凭据（注册/token）→ WebSocket → StartTask → StartSession。
    /// 后台预热：注册/token/WS 连接提前建立，热键按下时 open_session 只做握手（快）。
    pub async fn warmup(self: &Arc<Self>) {
        if self.connected.load(Ordering::SeqCst) {
            return;
        }
        if let Err(e) = self.ensure_credentials().await {
            log::warn!("[doubao] warmup credentials failed: {e}");
            return;
        }
        if let Err(e) = self.connect_ws().await {
            log::warn!("[doubao] warmup connect failed: {e}");
        }
    }

    pub async fn open_session(self: &Arc<Self>) -> Result<(), DoubaoError> {
        self.ensure_credentials().await?;
        if !self.connected.load(Ordering::SeqCst) {
            self.connect_ws().await?;
        }
        // 连接复用（预热/上一轮残留）：握手失败说明连接已失效，断开重连完整握手一次。
        // 并发配额（40200011）除外——连接本身正常，是服务端并发上限，重连只会再失败
        // 一次让用户多等一轮；直接快速失败，把可操作的提示交回上层。
        if let Err(e) = self.handshake().await {
            if is_quota_error(&e) {
                log::warn!("[doubao] handshake rejected (quota): {e}; not reconnecting");
                return Err(e);
            }
            log::warn!("[doubao] handshake on reused connection failed: {e}; reconnecting");
            self.disconnect();
            self.connect_ws().await?;
            self.handshake().await?;
        }

        let mut st = self.state.lock();
        if st.encoder.is_none() {
            let encoder = opus::Encoder::new(16_000, opus::Channels::Mono, opus::Application::Voip)
                .map_err(|e| DoubaoError::Opus(e.to_string()))?;
            st.encoder = Some(encoder);
        }
        Ok(())
    }

    /// 发送最后一帧（补零到 20ms）+ frameState=Last。
    pub async fn send_last_frame(&self) -> Result<(), DoubaoError> {
        let mut st = self.state.lock();
        if st.pending.is_empty() && st.frame_index == 0 {
            return Ok(()); // 没有录到任何音频
        }
        let frame = self.build_frame(&mut st, true)?;
        if !frame.is_empty() {
            self.send_payload(&mut st, &frame, true);
        }
        Ok(())
    }

    /// 发送 FinishSession → 等待 SessionFinished → 返回累积最终文本。
    pub async fn await_final_result(&self) -> Result<String, DoubaoError> {
        let (tx, rx) = oneshot::channel();
        self.waiters.lock().insert("SessionFinished".into(), tx);

        let token = self.credentials.lock().token.clone();
        let request_id = self.state.lock().request_id.clone();
        let msg = doubao_proto::encode_request(&token, "FinishSession", "", &[], &request_id, 0);
        self.send(msg);

        tokio::time::timeout(FINAL_TIMEOUT, rx)
            .await
            .map_err(|_| DoubaoError::Timeout("SessionFinished".into()))?
            .map_err(|_| DoubaoError::Transport("等待通道中断".into()))?
            .map_err(DoubaoError::Session)
    }

    /// 取消会话：断开连接。
    pub fn cancel(self: &Arc<Self>) {
        self.disconnect();
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

    /// 会话结束后断开连接（下一轮重新走注册/握手，与已验证的 Swift Demo 行为一致）。
    pub fn close(self: &Arc<Self>) {
        self.disconnect();
    }
}

// asr/mod.rs 的 AudioConsumer（doudou 双 trait 约定：recorder 推流 / asr 灌入）。
impl crate::asr::AudioConsumer for DoubaoImeASR {
    fn consume_pcm_chunk(&self, pcm: &[u8]) {
        AudioConsumer::consume_pcm_chunk(self, pcm);
    }
}

impl AudioConsumer for DoubaoImeASR {
    /// recorder 推来的 16k/16bit/mono PCM 字节流 → 每 640 字节编码一帧 Opus 发送。
    fn consume_pcm_chunk(&self, pcm: &[u8]) {
        if !self.connected.load(Ordering::SeqCst) {
            return;
        }
        let mut st = self.state.lock();
        let samples = pcm
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]));
        st.pending.extend(samples);
        while st.pending.len() >= FRAME_SAMPLES {
            let frame = match self.build_frame(&mut st, false) {
                Ok(f) => f,
                Err(_) => break,
            };
            self.send_payload(&mut st, &frame, false);
        }
    }
}

// MARK: - 内部实现

impl DoubaoImeASR {
    /// 发送一帧。`st` 由调用方持有（避免二次加锁死锁）。
    fn send_payload(&self, st: &mut SyncState, payload: &[u8], is_last: bool) {
        let state_flag: u64 = if is_last {
            9
        } else if st.frame_index == 0 {
            1
        } else {
            3
        };
        let timestamp = st.timestamp_base_ms + st.frame_index * 20;
        let json = format!(r#"{{"extra":{{}},"timestamp_ms":{}}}"#, timestamp);
        let msg = doubao_proto::encode_request(
            "",
            "TaskRequest",
            &json,
            payload,
            &st.request_id,
            state_flag,
        );
        st.frame_index += 1;
        self.send(msg);
    }

    /// 编码一帧 Opus（补零到 320 samples），返回 Opus 字节。
    fn build_frame(&self, st: &mut SyncState, is_last: bool) -> Result<Vec<u8>, DoubaoError> {
        let encoder = st
            .encoder
            .as_mut()
            .ok_or_else(|| DoubaoError::Opus("编码器未初始化".into()))?;
        let take = FRAME_SAMPLES.min(st.pending.len());
        let mut frame: Vec<i16> = st.pending.drain(..take).collect();
        if is_last && frame.len() < FRAME_SAMPLES {
            frame.resize(FRAME_SAMPLES, 0);
        }
        if frame.is_empty() {
            return Ok(Vec::new());
        }
        encoder
            .encode_vec(&frame, FRAME_SAMPLES)
            .map_err(|e| DoubaoError::Opus(e.to_string()))
    }

    fn send(&self, msg: Vec<u8>) {
        if let Some(tx) = self.send_tx.lock().as_ref() {
            let _ = tx.send(msg);
        }
    }

    fn register_waiter(
        &self,
        key: &str,
    ) -> Result<oneshot::Receiver<Result<String, String>>, DoubaoError> {
        let (tx, rx) = oneshot::channel();
        self.waiters.lock().insert(key.into(), tx);
        Ok(rx)
    }

    fn complete_waiter(&self, key: &str, value: Result<String, String>) {
        if let Some(tx) = self.waiters.lock().remove(key) {
            let _ = tx.send(value);
        }
    }

    // MARK: 凭据

    async fn ensure_credentials(&self) -> Result<(), DoubaoError> {
        if self.credentials.lock().auth_ok() {
            return Ok(());
        }
        // 尝试读缓存
        if let Ok(data) = std::fs::read(&self.credentials_path) {
            if let Ok(creds) = serde_json::from_slice::<DoubaoImeCredentials>(&data) {
                if creds.auth_ok() {
                    *self.credentials.lock() = creds;
                    return Ok(());
                }
            }
        }
        // 注册 + token
        let (device_id, cdid) = self.register_device().await?;
        let token = self.fetch_token(&device_id, &cdid).await?;
        let creds = DoubaoImeCredentials {
            device_id,
            cdid,
            token,
        };
        if let Ok(data) = serde_json::to_vec(&creds) {
            let _ = std::fs::write(&self.credentials_path, data);
        }
        *self.credentials.lock() = creds;
        Ok(())
    }

    async fn register_device(&self) -> Result<(String, String), DoubaoError> {
        let cdid = uuid::Uuid::new_v4().simple().to_string();
        let openudid = uuid::Uuid::new_v4().simple().to_string()[..16].to_string();
        let clientudid = uuid::Uuid::new_v4().simple().to_string();
        let query = format!(
            "/service/2/device_register/?device_platform=android&os=android&ssmix=a\
             &_rticket={}&cdid={}&channel=official&aid={}&app_name=oime&version_code={}\
             &version_name=1.1.2&manifest_version_code={}&update_version_code={}\
             &resolution=1080*2400&dpi=420&device_type=Pixel%207%20Pro&device_brand=google\
             &language=zh&os_api=34&os_version=16&ac=wifi",
            unix_ms(),
            cdid,
            AID,
            VERSION_CODE,
            VERSION_CODE,
            VERSION_CODE
        );

        let header = serde_json::json!({
            "device_id": 0, "install_id": 0, "aid": AID,
            "app_name": "oime", "version_code": VERSION_CODE, "version_name": "1.1.2",
            "manifest_version_code": VERSION_CODE, "update_version_code": VERSION_CODE,
            "channel": "official", "package": "com.bytedance.android.doubaoime",
            "device_platform": "android", "os": "android", "os_api": "34", "os_version": "16",
            "device_type": "Pixel 7 Pro", "device_brand": "google", "device_model": "Pixel 7 Pro",
            "resolution": "1080*2400", "dpi": "420", "language": "zh", "timezone": 8,
            "access": "wifi", "rom": "UP1A.231005.007", "rom_version": "UP1A.231005.007",
            "openudid": openudid, "clientudid": clientudid, "cdid": cdid,
            "region": "CN", "tz_name": "Asia/Shanghai", "tz_offset": 28800,
            "sim_region": "cn", "carrier_region": "cn", "cpu_abi": "arm64-v8a",
            "build_serial": "unknown", "not_request_sender": 0,
            "sig_hash": "", "google_aid": "", "mc": "", "serial_number": "",
        });
        let body = serde_json::json!({
            "magic_tag": "ss_app_log",
            "header": header,
            "_gen_time": unix_ms(),
        });

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| DoubaoError::Register(e.to_string()))?;
        let resp = client
            .post(format!("https://{REGISTER_HOST}{query}"))
            .header("User-Agent", USER_AGENT)
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| DoubaoError::Register(e.to_string()))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(DoubaoError::Register(format!("HTTP {status} {text}")));
        }
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| DoubaoError::Register(format!("响应解析失败: {e}")))?;
        let device_id = json
            .get("device_id")
            .and_then(|v| v.as_i64())
            .filter(|&id| id != 0)
            .ok_or_else(|| DoubaoError::Register("响应缺少 device_id".into()))?;
        Ok((device_id.to_string(), cdid))
    }

    async fn fetch_token(&self, device_id: &str, cdid: &str) -> Result<String, DoubaoError> {
        let query = format!(
            "/service/settings/v3/?device_platform=android&os=android&ssmix=a\
             &channel=official&aid={}&app_name=oime&version_code={}\
             &version_name=1.1.2&device_id={}&cdid={}",
            AID, VERSION_CODE, device_id, cdid
        );
        let body = "body=null";
        use md5::Digest;
        let digest = md5::Md5::digest(body.as_bytes());
        let stub: String = digest.iter().map(|b| format!("{b:02X}")).collect();

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| DoubaoError::Token(e.to_string()))?;
        let resp = client
            .post(format!("https://{SETTINGS_HOST}{query}"))
            .header("User-Agent", USER_AGENT)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("x-ss-stub", stub)
            .body(body)
            .send()
            .await
            .map_err(|e| DoubaoError::Token(e.to_string()))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(DoubaoError::Token(format!("HTTP {status} {text}")));
        }
        let json: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| DoubaoError::Token(format!("响应解析失败: {e}: {text}")))?;
        // app_key 嵌套在 data.settings.settings.asr_config 里（Windows 版同款递归查找）
        find_string_recursive(&json, "app_key")
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .ok_or_else(|| DoubaoError::Token(format!("响应缺少 app_key: {text}")))
    }

    // MARK: WebSocket

    async fn connect_ws(self: &Arc<Self>) -> Result<(), DoubaoError> {
        // single-flight：warmup 与 open_session 可能并发调用；已有连接直接复用，
        // 另一个连接进行中则等它完成（最多 3s），避免双连接互相覆盖。
        if self.connected.load(Ordering::SeqCst) {
            return Ok(());
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while self
            .connecting
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            if self.connected.load(Ordering::SeqCst) {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(DoubaoError::Connect("连接建立超时(3s)".into()));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let result = self.connect_ws_inner().await;
        self.connecting.store(false, Ordering::SeqCst);
        result
    }

    async fn connect_ws_inner(self: &Arc<Self>) -> Result<(), DoubaoError> {
        let device_id = self.credentials.lock().device_id.clone();
        let url = format!("wss://{WS_HOST}{WS_PATH}?aid={AID}&device_id={device_id}");
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        use tokio_tungstenite::tungstenite::http::Request;
        let mut request: Request<()> = IntoClientRequest::into_client_request(url)
            .map_err(|e| DoubaoError::Connect(e.to_string()))?;
        {
            let headers = request.headers_mut();
            headers.insert("User-Agent", USER_AGENT.parse().unwrap());
            headers.insert("proto-version", "v2".parse().unwrap());
            headers.insert("x-custom-keepalive", "true".parse().unwrap());
        }
        let (ws, _) = tokio::time::timeout(Duration::from_secs(10), connect_async(request))
            .await
            .map_err(|_| DoubaoError::Connect("连接超时(10s)".into()))?
            .map_err(|e| DoubaoError::Connect(e.to_string()))?;
        let (mut sink, mut stream) = ws.split();

        // 发送队列：保序发送（音频帧 / FinishSession 全走这一个 channel）
        let (send_tx, mut send_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let writer_task = tokio::spawn(async move {
            while let Some(data) = send_rx.recv().await {
                if let Err(e) = sink.send(Message::Binary(data.into())).await {
                    log::warn!("[doubao] ws send: {e}");
                    break;
                }
            }
            let _ = sink.close().await;
        });

        // 接收循环：解析 protobuf 响应，分发到 waiter / 文本累积。
        // 用 Weak 打破 Arc 环：孤儿 receive task（双连接竞态等异常路径）不会让实例永驻。
        let weak_self = Arc::downgrade(self);
        let receive_task = tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                let Some(this_self) = weak_self.upgrade() else {
                    break; // 实例已释放
                };
                match msg {
                    Ok(Message::Binary(data)) => {
                        this_self.handle_binary(data);
                    }
                    Ok(Message::Close(_)) => {
                        this_self.on_disconnect();
                        break;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        log::warn!("[doubao] ws recv: {e}");
                        this_self.on_disconnect();
                        break;
                    }
                }
            }
            if let Some(this_self) = weak_self.upgrade() {
                this_self.on_disconnect();
            }
        });

        *self.send_tx.lock() = Some(send_tx);
        *self.writer_task.lock() = Some(writer_task);
        *self.receive_task.lock() = Some(receive_task);
        self.connected.store(true, Ordering::SeqCst);
        Ok(())
    }

    // MARK: 握手

    async fn handshake(&self) -> Result<(), DoubaoError> {
        let token = self.credentials.lock().token.clone();
        let request_id = uuid::Uuid::new_v4().simple().to_string();

        {
            let mut st = self.state.lock();
            st.request_id = request_id.clone();
            st.frame_index = 0;
            st.timestamp_base_ms = unix_ms();
            st.final_accumulator.clear();
            st.last_final_segment.clear();
            st.pending.clear();
        }

        // StartTask → TaskStarted
        let rx = self.register_waiter("TaskStarted")?;
        let msg = doubao_proto::encode_request(&token, "StartTask", "", &[], &request_id, 0);
        self.send(msg);
        tokio::time::timeout(HANDSHAKE_TIMEOUT, rx)
            .await
            .map_err(|_| DoubaoError::Timeout("TaskStarted".into()))?
            .map_err(|_| DoubaoError::Transport("等待通道中断".into()))?
            .map_err(DoubaoError::Session)?;

        // StartSession → SessionStarted
        let rx = self.register_waiter("SessionStarted")?;
        let device_id = self.credentials.lock().device_id.clone();
        let payload = start_session_payload(&device_id);
        let msg =
            doubao_proto::encode_request(&token, "StartSession", &payload, &[], &request_id, 0);
        self.send(msg);
        tokio::time::timeout(HANDSHAKE_TIMEOUT, rx)
            .await
            .map_err(|_| DoubaoError::Timeout("SessionStarted".into()))?
            .map_err(|_| DoubaoError::Transport("等待通道中断".into()))?
            .map_err(DoubaoError::Session)?;

        Ok(())
    }

    // MARK: 消息处理

    fn handle_binary(&self, data: Vec<u8>) {
        let Some(resp) = doubao_proto::decode_response(&data) else {
            return;
        };
        // 先处理结果文本：SessionFinished 的 waiter 需要合并后的最终文本，
        // 不能在 complete 之后才合并（否则随 FinishSession 下发的定稿会丢失）。
        if !resp.result_json.is_empty() {
            if let Some((text, is_final)) = extract_text_candidate(&resp.result_json) {
                if is_final {
                    let mut st = self.state.lock();
                    if text != st.last_final_segment {
                        st.final_accumulator = merge_recognized_text(&st.final_accumulator, &text);
                        st.last_final_segment = text;
                    }
                } else if let Some(cb) = &self.on_partial {
                    cb(text);
                }
            }
        }
        match resp.message_type.as_str() {
            "TaskStarted" | "SessionStarted" => {
                self.complete_waiter(&resp.message_type, Ok(String::new()));
            }
            "TaskFailed" | "SessionFailed" => {
                let raw = if resp.status_message.is_empty() {
                    resp.message_type.clone()
                } else {
                    resp.status_message.clone()
                };
                log::warn!("[doubao] 服务端错误: {raw} (status={})", resp.status_code);
                // 并发配额（40200011）：会话创建被拒，计数随已开会话关闭而回落。
                // 给用户可操作的提示而非内部错误原文（日志已保留原始信息），并带上
                // 状态码供上层识别配额类错误（重连 / 自动重试都无意义）。
                let user_msg = if resp.status_code == 40200011 {
                    "豆包免费通道并发已达上限，请稍后再试 (status=40200011)".to_string()
                } else {
                    raw.clone()
                };
                // 失败时同时唤醒握手 waiter（否则 handshake 干等 8s 超时且丢失真实错误）。
                self.complete_waiter("TaskStarted", Err(user_msg.clone()));
                self.complete_waiter("SessionStarted", Err(user_msg.clone()));
                self.complete_waiter(&resp.message_type, Err(user_msg.clone()));
                self.complete_waiter("SessionFinished", Err(user_msg));
            }
            "SessionFinished" => {
                let text = self.state.lock().final_accumulator.clone();
                self.complete_waiter("SessionFinished", Ok(text));
            }
            _ => {}
        }
    }

    fn on_disconnect(&self) {
        self.connected.store(false, Ordering::SeqCst);
        // 唤醒可能在等待的 waiter，避免挂死
        self.complete_waiter("TaskStarted", Err("连接中断".into()));
        self.complete_waiter("SessionStarted", Err("连接中断".into()));
        self.complete_waiter("SessionFinished", Err("连接中断".into()));
    }

    fn disconnect(&self) {
        self.connected.store(false, Ordering::SeqCst);
        self.send_tx.lock().take();
        if let Some(task) = self.writer_task.lock().take() {
            task.abort();
        }
        if let Some(task) = self.receive_task.lock().take() {
            task.abort();
        }
        // abort 的 receive task 不会走 on_disconnect——这里显式唤醒 waiters，避免取消后干等超时。
        self.complete_waiter("TaskStarted", Err("连接中断".into()));
        self.complete_waiter("SessionStarted", Err("连接中断".into()));
        self.complete_waiter("SessionFinished", Err("连接中断".into()));
    }
}

/// 递归搜索 JSON 中第一个名为 `key` 的字符串值（照搬 Windows 版 ExtractJsonStringValue 语义）。
fn find_string_recursive<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(s) = map.get(key).and_then(|v| v.as_str()) {
                return Some(s);
            }
            for v in map.values() {
                if let Some(s) = find_string_recursive(v, key) {
                    return Some(s);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => arr.iter().find_map(|v| find_string_recursive(v, key)),
        _ => None,
    }
}

fn start_session_payload(device_id: &str) -> String {
    format!(
        r#"{{"audio_info":{{"channel":1,"format":"speech_opus","sample_rate":16000}},"enable_punctuation":true,"enable_speech_rejection":false,"extra":{{"app_name":"com.android.chrome","cell_compress_rate":8,"did":"{device_id}","enable_asr_threepass":true,"enable_asr_twopass":true,"input_mode":"tool"}}}}"#
    )
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// MARK: - 文本提取与合并（照搬 Windows 版 ExtractTextCandidate / MergeRecognizedText）

fn extract_text_candidate(result_json: &str) -> Option<(String, bool)> {
    let json: serde_json::Value = serde_json::from_str(result_json).ok()?;
    let results = json.get("results")?.as_array()?;
    let mut aggregate = String::new();
    let mut last_final = false;
    for obj in results {
        let Some(text) = obj.get("text").and_then(|v| v.as_str()) else {
            continue; // 纯 VAD/状态标记对象无 text，跳过而不是丢弃整条消息
        };
        if text.is_empty() {
            continue;
        }
        let is_interim = obj
            .get("is_interim")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let is_vad_finished = obj
            .get("is_vad_finished")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let nonstream = obj
            .get("extra")
            .and_then(|e| e.get("nonstream_result"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        aggregate = append_segment(&aggregate, text);
        last_final = nonstream || (!is_interim && is_vad_finished);
    }
    if aggregate.is_empty() {
        None
    } else {
        Some((aggregate, last_final))
    }
}

fn append_segment(target: &str, text: &str) -> String {
    if text.is_empty() {
        return target.to_string();
    }
    if target.is_empty() {
        return text.to_string();
    }
    let prev = target.chars().next_back().unwrap();
    let next = text.chars().next().unwrap();
    if !prev.is_whitespace() && !next.is_whitespace() {
        if next.is_alphanumeric() && (prev.is_alphanumeric() || ".,;:!?".contains(prev)) {
            return format!("{target} {text}");
        }
    }
    format!("{target}{text}")
}

fn merge_recognized_text(base: &str, incoming_raw: &str) -> String {
    let incoming = incoming_raw.trim();
    let base = base.trim();
    if incoming.is_empty() {
        return base.to_string();
    }
    if base.is_empty() {
        return incoming.to_string();
    }
    if base == incoming {
        return base.to_string();
    }
    if incoming.starts_with(base) {
        return incoming.to_string();
    }
    if base.starts_with(incoming) {
        return base.to_string();
    }
    // 后缀-前缀重叠合并
    let max_overlap = base.chars().count().min(incoming.chars().count());
    if max_overlap > 1 {
        for len in (2..=max_overlap).rev() {
            let suffix: String = base.chars().skip(base.chars().count() - len).collect();
            if incoming.starts_with(&suffix) {
                let rest: String = incoming.chars().skip(len).collect();
                return format!("{base}{rest}");
            }
        }
    }
    append_segment(base, incoming)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 端到端引擎验证（真实网络）：注册 → token → WS 握手 → 发 1 秒静音 → 等最终结果。
    /// 手动运行：cargo test --lib -- --ignored --nocapture asr::doubao::tests::engine
    #[tokio::test]
    #[ignore]
    async fn engine_end_to_end() {
        let asr = DoubaoImeASR::new(None);

        // 整体超时保护：任何一步意外挂住，最多 60s 后失败退出
        let outcome = tokio::time::timeout(Duration::from_secs(60), async {
            println!("[1/5] 设备注册 + 取 token…");
            asr.open_session().await.expect("open_session 失败");
            println!("[2/5] ✅ open_session 成功（注册/token/WS/握手全通）");

            // 1 秒静音（16000 samples），按 640 字节帧喂给 consumer
            let silence: Vec<u8> = (0..16000i16)
                .map(|_| 0i16)
                .flat_map(|s| s.to_le_bytes())
                .collect();
            for chunk in silence.chunks(640) {
                asr.consume_pcm_chunk(chunk);
            }
            println!("[3/5] 静音帧已发送，发尾帧…");
            asr.send_last_frame().await.expect("send_last_frame 失败");
            println!("[4/5] 等最终结果…");
            let text = asr
                .await_final_result()
                .await
                .expect("await_final_result 失败");
            println!("[5/5] RESULT_TEXT: [{text}]");
        })
        .await;

        asr.close();
        if let Err(_) = outcome {
            panic!("engine test 整体超时(60s)");
        }
    }
}
