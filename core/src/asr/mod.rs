//! Streaming ASR providers.
//!
//! doudou 保留两个免费通道：豆包 IME（无凭据、自动注册，协议实现见
//! `doubao_proto.rs`）与 Grok STT（经 worker-search /v1/audio/transcriptions
//! 中转，非流式，见 `grok_stt.rs`）。

pub mod doubao;
pub mod doubao_proto;
pub mod grok_stt;

pub use doubao::DoubaoImeASR;
pub use grok_stt::GrokSttASR;

/// Sink for raw 16 kHz / 16-bit / mono PCM bytes coming off the recorder.
///
/// The Recorder pushes chunks here as soon as it has them; the ASR session
/// is free to batch internally before flushing to the network.
pub trait AudioConsumer: Send + Sync {
    fn consume_pcm_chunk(&self, pcm: &[u8]);
}

/// What the ASR session yielded once the stream closed.
#[derive(Debug, Clone)]
pub struct RawTranscript {
    pub text: String,
    pub duration_ms: u64,
}
