#![cfg_attr(target_os = "linux", allow(dead_code, unused_variables))]
//! Credentials vault.
//!
//! 正常读写走系统凭据库；旧 plaintext JSON 只作为迁移来源。为保持多 provider
//! schema 与 active provider 状态，凭据库里保存一个 v1 JSON payload；payload 会按平台
//! 凭据库限制拆成多个条目，避免 Windows 单条凭据 2560 bytes 限制。
//!
//! v1 schema：
//!   {
//!     "version": 1,
//!     "active": { "asr": "<id>", "llm": "<id>" },
//!     "providers": {
//!       "asr": { "<id>": { "appKey", "accessKey", "resourceId", "apiKey", "baseURL", "model", "vocabularyId" } },
//!       "llm": { "<id>": { "displayName", "apiKey", "baseURL", "model", "temperature", "extraHeaders" } }
//!     },
//!     "marketplace": { "githubAccessToken": "<desktop-only secret>" }
//!   }
//!
//! "ark.api_key"/"volcengine.app_key" 等账户名按 Swift 语义路由到 active provider。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// 旧版 plaintext JSON 凭据路径。仅作为迁移来源；成功写入系统凭据库后会删除。
const LEGACY_CREDS_DIR: &str = ".openless";
const LEGACY_CREDS_FILE: &str = "credentials.json";

const KEYRING_CREDENTIALS_ACCOUNT: &str = "credentials.v1";
const KEYRING_CREDENTIALS_CHUNK_PREFIX: &str = "credentials.v1.chunk.";
// Windows Credential Manager caps one credential blob at 2560 bytes. keyring stores
// passwords as UTF-16 on Windows, so keep each JSON chunk comfortably below that.
const KEYRING_CHUNK_MAX_UTF16_UNITS: usize = 1000;

static CREDENTIALS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn credentials_lock() -> &'static Mutex<()> {
    CREDENTIALS_LOCK.get_or_init(|| Mutex::new(()))
}

/// Process-wide credentials cache.
///
/// Without this cache every `CredentialsVault::get_*` / `snapshot` call hits
/// `load_credentials()` → `load_keyring_credentials()` which reads the
/// manifest entry plus every chunk entry from the OS keyring. On macOS each
/// distinct keychain entry has its own ACL — so an ad-hoc-signed binary (or
/// any binary whose ACL grants haven't been set up yet) prompts on every read
/// of every entry. A single dictation cycle reads credentials 5–10 times,
/// times (1 manifest + N chunks) entries → tens of "OpenLess wants to use
/// the keychain" prompts per recording.
///
/// With this cache the first read populates `Some(CredsRoot)` and every
/// subsequent read in the same process is silent. `save_credentials` keeps
/// the cache in sync after writes so Settings → Recording credential edits
/// take effect immediately.
///
/// Cross-process changes (e.g. user edits via `security` CLI, or another
/// instance of the app — single-instance is enforced but defense in depth)
/// will be invisible until the next process launch. Acceptable trade-off
/// per the credential vault contract: the keyring is owned by this app.
static CREDENTIALS_CACHE: OnceLock<Mutex<Option<CredsRoot>>> = OnceLock::new();

fn credentials_cache() -> &'static Mutex<Option<CredsRoot>> {
    CREDENTIALS_CACHE.get_or_init(|| Mutex::new(None))
}

fn store_credentials_cache(root: &CredsRoot) {
    *credentials_cache().lock() = Some(root.clone());
}

#[cfg(test)]
fn reset_credentials_cache_for_tests() {
    *credentials_cache().lock() = None;
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[allow(non_snake_case)]
struct CredsRoot {
    #[serde(default = "credsroot_default_version")]
    version: u32,
    #[serde(default)]
    active: CredsActive,
    #[serde(default)]
    providers: CredsProviders,
    #[serde(default, skip_serializing_if = "CredsMarketplace::is_empty")]
    marketplace: CredsMarketplace,
}

fn credsroot_default_version() -> u32 {
    1
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct CredsActive {
    #[serde(default = "creds_default_asr")]
    asr: String,
    #[serde(default = "creds_default_llm")]
    llm: String,
}

impl Default for CredsActive {
    fn default() -> Self {
        Self {
            asr: creds_default_asr(),
            llm: creds_default_llm(),
        }
    }
}

fn creds_default_asr() -> String {
    // 豆包引擎版：默认 active ASR 就是豆包（钥匙串跳过后的内存态默认值）。
    crate::asr::doubao::PROVIDER_ID.into()
}
fn creds_default_llm() -> String {
    "ark".into()
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
struct CredsProviders {
    #[serde(default)]
    asr: HashMap<String, CredsAsrEntry>,
    #[serde(default)]
    llm: HashMap<String, CredsLlmEntry>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[allow(non_snake_case)]
struct CredsMarketplace {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    githubAccessToken: Option<MarketplaceGithubToken>,
}

impl CredsMarketplace {
    fn is_empty(&self) -> bool {
        self.githubAccessToken.is_none()
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(transparent)]
struct MarketplaceGithubToken(String);

impl std::fmt::Debug for MarketplaceGithubToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[allow(non_snake_case)]
struct CredsAsrEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    apiKey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseURL: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    appKey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    accessKey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resourceId: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    authMode: Option<String>,
    /// 方舟（Ark）API Key —— 仅 `api_key` 鉴权模式使用，与旧版 Access Token 槽位
    /// (`accessKey`) 隔离，避免两模式切换时残留凭据互相污染。
    #[serde(skip_serializing_if = "Option::is_none")]
    volcengineApiKey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vocabularyId: Option<String>,
    /// 通用 OpenAI 兼容 ASR(openai-compatible)的高级配置 JSON:
    /// `{"verboseJson": bool, "chunkDurationMs": number|null}`。
    /// 仅该预设读取;命名厂商的怪癖开关保持硬编码,不受此字段影响。
    #[serde(skip_serializing_if = "Option::is_none")]
    advancedConfig: Option<String>,
    /// 讯飞开放平台应用 ID（RTASR/IFASR 鉴权用）。
    #[serde(skip_serializing_if = "Option::is_none")]
    xfyunAppId: Option<String>,
    /// 讯飞实时语音转写 APIKey（接口密钥）。
    #[serde(skip_serializing_if = "Option::is_none")]
    xfyunApiKey: Option<String>,
}

impl CredsAsrEntry {
    fn is_empty(&self) -> bool {
        self.apiKey.as_deref().unwrap_or("").is_empty()
            && self.baseURL.as_deref().unwrap_or("").is_empty()
            && self.model.as_deref().unwrap_or("").is_empty()
            && self.appKey.as_deref().unwrap_or("").is_empty()
            && self.accessKey.as_deref().unwrap_or("").is_empty()
            && self.resourceId.as_deref().unwrap_or("").is_empty()
            && self.authMode.as_deref().unwrap_or("").is_empty()
            && self.volcengineApiKey.as_deref().unwrap_or("").is_empty()
            && self.vocabularyId.as_deref().unwrap_or("").is_empty()
            && self.advancedConfig.as_deref().unwrap_or("").is_empty()
            && self.xfyunAppId.as_deref().unwrap_or("").is_empty()
            && self.xfyunApiKey.as_deref().unwrap_or("").is_empty()
    }
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[allow(non_snake_case)]
struct CredsLlmEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    displayName: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    apiKey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    baseURL: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extraHeaders: Option<HashMap<String, String>>,
}

impl CredsLlmEntry {
    fn is_empty(&self) -> bool {
        self.displayName.as_deref().unwrap_or("").is_empty()
            && self.apiKey.as_deref().unwrap_or("").is_empty()
            && self.baseURL.as_deref().unwrap_or("").is_empty()
            && self.model.as_deref().unwrap_or("").is_empty()
            && self.temperature.is_none()
            && self
                .extraHeaders
                .as_ref()
                .map(|h| h.is_empty())
                .unwrap_or(true)
    }
}

fn credentials_path() -> Result<PathBuf> {
    // macOS / Linux: ~/.openless/credentials.json (与 Swift 同源)
    // Windows: %APPDATA%\OpenLess\credentials.json (Windows 没有标准 HOME 环境变量)
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").context("APPDATA not set")?;
        return Ok(PathBuf::from(appdata)
            .join("OpenLess")
            .join(LEGACY_CREDS_FILE));
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = std::env::var("HOME").context("HOME not set")?;
        Ok(PathBuf::from(home)
            .join(LEGACY_CREDS_DIR)
            .join(LEGACY_CREDS_FILE))
    }
}

fn keyring_entry() -> Result<keyring::Entry> {
    keyring_entry_for(KEYRING_CREDENTIALS_ACCOUNT)
}

fn keyring_entry_for(account: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(CredentialsVault::SERVICE_NAME, account)
        .context("open system credential vault")
}

fn clean_credentials(root: &CredsRoot) -> CredsRoot {
    let mut cleaned = root.clone();
    cleaned.providers.asr.retain(|_, v| !v.is_empty());
    cleaned.providers.llm.retain(|_, v| !v.is_empty());
    cleaned
}

#[cfg(test)]
fn lookup_marketplace_github_token(root: &CredsRoot) -> Option<String> {
    root.marketplace
        .githubAccessToken
        .as_ref()
        .map(|token| token.0.as_str())
        .filter(|token| !token.trim().is_empty())
        .map(str::to_string)
}

#[cfg(test)]
fn write_marketplace_github_token(root: &mut CredsRoot, value: Option<String>) {
    root.marketplace.githubAccessToken = value.and_then(|token| {
        if token.trim().is_empty() {
            None
        } else {
            Some(MarketplaceGithubToken(token))
        }
    });
}

fn remove_legacy_credentials_file() -> Result<()> {
    let Ok(path) = credentials_path() else {
        return Ok(());
    };
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("remove legacy credentials file {}", path.display()))?;
    }
    Ok(())
}

fn remove_legacy_credentials_file_best_effort() {
    if let Err(e) = remove_legacy_credentials_file() {
        log::warn!("[vault] remove legacy credentials file failed: {e}");
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct CredsChunkManifest {
    openless_credentials_storage: String,
    version: u32,
    /// 旧版本（v1 早期）每次 save 都生成新 UUID 作为 chunk account 命名前缀，
    /// 这让 macOS Keychain 的「始终允许」每次保存后失效 → 反复弹 ACL 弹窗。
    /// 现在 save 总用稳定 chunk.{index} 名，此字段仅向后兼容旧 manifest 读取。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generation: Option<String>,
    chunks: usize,
}

/// 旧版（generation=Some）：`credentials.v1.chunk.<UUID>.{index}`
/// 新版（generation=None）：`credentials.v1.chunk.{index}` —— 稳定名，ACL 长期有效
fn chunk_account(generation: Option<&str>, index: usize) -> String {
    match generation {
        Some(gen) => format!("{KEYRING_CREDENTIALS_CHUNK_PREFIX}{gen}.{index}"),
        None => format!("{KEYRING_CREDENTIALS_CHUNK_PREFIX}{index}"),
    }
}

fn chunk_json_payload(json: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_units = 0usize;
    for ch in json.chars() {
        let units = ch.len_utf16();
        if !current.is_empty() && current_units + units > KEYRING_CHUNK_MAX_UTF16_UNITS {
            chunks.push(std::mem::take(&mut current));
            current_units = 0;
        }
        current.push(ch);
        current_units += units;
    }
    if !current.is_empty() || json.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn read_chunk_manifest(json: &str) -> Option<CredsChunkManifest> {
    let manifest = serde_json::from_str::<CredsChunkManifest>(json).ok()?;
    if manifest.openless_credentials_storage == "chunked" && manifest.version == 1 {
        Some(manifest)
    } else {
        None
    }
}

/// Windows Credential Manager (`CredReadW`) can transiently fail right after
/// login / under contention when we read the manifest entry plus every chunk
/// entry in quick succession. A single failed read makes the whole credential
/// set look empty → `load_keyring_credentials` returns `Err` → `load_credentials`
/// falls back to an empty default → Overview shows「火山引擎未配置」even though the
/// secrets are present (the next dictation re-reads and succeeds, which is why the
/// bug is *probabilistic* and the app "实际可以正常使用"). The more chunks a
/// credential set spans, the more reads per load, the higher the odds at least
/// one trips. Retry transient errors a few times with short backoff.
///
/// macOS / Linux keep the original single-shot behavior on purpose: their read
/// errors are ACL denials that won't heal on retry, and the un-cached error path
/// already retries on the next call — adding sleeps there would only slow the
/// macOS first-launch Keychain authorization flow.
#[cfg(target_os = "windows")]
const KEYRING_READ_RETRY_ATTEMPTS: usize = 4;
#[cfg(target_os = "windows")]
const KEYRING_READ_RETRY_BACKOFF_MS: u64 = 60;

fn get_keyring_password(account: &str) -> Result<Option<String>> {
    #[cfg(target_os = "windows")]
    {
        let mut attempt = 0usize;
        loop {
            match keyring_entry_for(account)?.get_password() {
                Ok(value) => return Ok(Some(value)),
                // NoEntry is a definitive "not stored" answer, never a transient
                // failure — return immediately so genuinely-unconfigured providers
                // don't pay the retry latency.
                Err(keyring::Error::NoEntry) => return Ok(None),
                Err(e) => {
                    attempt += 1;
                    if attempt >= KEYRING_READ_RETRY_ATTEMPTS {
                        return Err(anyhow!(e))
                            .with_context(|| format!("read system credential vault {account}"));
                    }
                    log::warn!(
                        "[vault] transient credential read for {account} failed \
                         (attempt {attempt}/{KEYRING_READ_RETRY_ATTEMPTS}): {e}; retrying"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(
                        KEYRING_READ_RETRY_BACKOFF_MS * attempt as u64,
                    ));
                }
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        match keyring_entry_for(account)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => {
                Err(anyhow!(e)).with_context(|| format!("read system credential vault {account}"))
            }
        }
    }
}

fn delete_keyring_password(account: &str) {
    match keyring_entry_for(account).and_then(|entry| {
        entry
            .delete_credential()
            .with_context(|| format!("delete system credential vault {account}"))
    }) {
        Ok(()) | Err(_) => {}
    }
}

fn load_credentials() -> CredsRoot {
    if let Some(cached) = credentials_cache().lock().as_ref().cloned() {
        return cached;
    }

    // 豆包引擎版：无凭据需要存储，跳过系统钥匙串（避免 macOS Keychain 弹窗）。
    let root = CredsRoot::default();
    store_credentials_cache(&root);
    root
}

fn load_credentials_for_update() -> Result<CredsRoot> {
    if let Some(cached) = credentials_cache().lock().as_ref().cloned() {
        return Ok(cached);
    }

    // 豆包引擎版：凭据仅内存态，不访问系统钥匙串。
    let root = CredsRoot::default();
    store_credentials_cache(&root);
    Ok(root)
}

fn save_credentials(root: &CredsRoot) -> Result<()> {
    let cleaned = clean_credentials(root);

    // 豆包引擎版：凭据仅内存态，不写系统钥匙串。
    store_credentials_cache(&cleaned);
    return Ok(());
    #[allow(unreachable_code)]
        let json = serde_json::to_string(&cleaned).context("encode credentials failed")?;
        let previous_manifest = get_keyring_password(KEYRING_CREDENTIALS_ACCOUNT)
            .ok()
            .flatten()
            .and_then(|value| read_chunk_manifest(&value));
        let chunks = chunk_json_payload(&json);

        // 先写所有 chunks（稳定名），再写 manifest —— 保证 partial-write 不会让
        // manifest 指向不完整 chunks。stable name 让 macOS Keychain ACL 一次允许后
        // 长期有效，不再因 UUID 轮换反复弹窗（这是 PR #277 早期 UUID-rotation
        // 设计的回退）。
        for (index, chunk) in chunks.iter().enumerate() {
            let account = chunk_account(None, index);
            keyring_entry_for(&account)?
                .set_password(chunk)
                .with_context(|| format!("write system credential vault chunk {index}"))?;
        }

        let manifest = CredsChunkManifest {
            openless_credentials_storage: "chunked".to_string(),
            version: 1,
            generation: None,
            chunks: chunks.len(),
        };
        let manifest_json =
            serde_json::to_string(&manifest).context("encode credential manifest failed")?;
        keyring_entry()?
            .set_password(&manifest_json)
            .context("write system credential vault manifest")?;

        // 清理旧 chunks：
        // 1) 旧 manifest 用 UUID generation → 那一代 chunks 全删（迁移到 stable name）
        // 2) 旧 manifest 也是 stable name，但 chunks 数量比这次多 → 删多余的 idx
        if let Some(previous) = previous_manifest {
            match previous.generation.as_deref() {
                Some(prev_gen) => {
                    for index in 0..previous.chunks {
                        delete_keyring_password(&chunk_account(Some(prev_gen), index));
                    }
                }
                None => {
                    for index in chunks.len()..previous.chunks {
                        delete_keyring_password(&chunk_account(None, index));
                    }
                }
            }
        }

        remove_legacy_credentials_file_best_effort();
        // 写完成功后立刻刷新 process cache —— 同进程后续读不再回 Keychain。
        // 见 CREDENTIALS_CACHE 的 doc。
        store_credentials_cache(&cleaned);
        Ok(())
}

/// 凭据存储——系统凭据库；旧 JSON 文件只作为迁移来源。
pub struct CredentialsVault;

impl CredentialsVault {
    /// 系统凭据库 service name；macOS 下对应 Keychain service。
    pub const SERVICE_NAME: &'static str = "com.openless.app";

    pub fn get_active_asr() -> String {
        let _guard = credentials_lock().lock();
        load_credentials().active.asr
    }

    pub fn set_active_asr_provider(id: &str) -> Result<()> {
        let _guard = credentials_lock().lock();
        let mut root = load_credentials_for_update()?;
        root.active.asr = id.to_string();
        save_credentials(&root)
    }

}

#[cfg(test)]
mod tests {
    use super::{
        chunk_json_payload, lookup_marketplace_github_token, write_marketplace_github_token,
        CredsRoot, KEYRING_CHUNK_MAX_UTF16_UNITS,
    };

    #[test]
    fn credential_payload_chunks_stay_under_windows_blob_limit() {
        let payload = format!(
            "{}{}{}",
            "a".repeat(KEYRING_CHUNK_MAX_UTF16_UNITS + 25),
            "😀".repeat(20),
            "b".repeat(KEYRING_CHUNK_MAX_UTF16_UNITS + 25)
        );
        let chunks = chunk_json_payload(&payload);
        assert!(chunks.len() > 1);
        assert_eq!(chunks.concat(), payload);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.encode_utf16().count() <= KEYRING_CHUNK_MAX_UTF16_UNITS));
    }

    #[test]
    fn marketplace_github_token_uses_the_credentials_payload_not_provider_accounts() {
        let mut root = CredsRoot::default();
        assert_eq!(lookup_marketplace_github_token(&root), None);

        write_marketplace_github_token(&mut root, Some("gho_vault_only".to_string()));

        assert_eq!(
            lookup_marketplace_github_token(&root).as_deref(),
            Some("gho_vault_only")
        );
        assert!(root.providers.asr.is_empty());
        assert!(root.providers.llm.is_empty());
    }

    #[test]
    #[test]
    fn legacy_credentials_payload_without_marketplace_token_remains_readable() {
        let root: CredsRoot = serde_json::from_str(r#"{"version":1}"#)
            .expect("pre-marketplace credentials should remain compatible");

        assert_eq!(lookup_marketplace_github_token(&root), None);
    }

    #[test]
    fn marketplace_logout_removes_only_the_marketplace_token() {
        let mut root = CredsRoot::default();
        root.active.llm = "configured-provider".to_string();
        write_marketplace_github_token(&mut root, Some("gho_remove_me".to_string()));

        write_marketplace_github_token(&mut root, None);

        assert_eq!(lookup_marketplace_github_token(&root), None);
        assert_eq!(root.active.llm, "configured-provider");
    }

    #[test]
    fn marketplace_token_is_absent_from_serialized_preferences() {
        let token = "gho_must_not_enter_preferences";
        let mut root = CredsRoot::default();
        write_marketplace_github_token(&mut root, Some(token.to_string()));

        let credentials_json = serde_json::to_string(&root).expect("credentials should serialize");
        let preferences_json = serde_json::to_string(&crate::types::UserPreferences::default())
            .expect("preferences should serialize");

        assert!(credentials_json.contains(token));
        assert!(!preferences_json.contains(token));
        assert!(!preferences_json.contains("githubAccessToken"));
        assert!(!format!("{root:?}").contains(token));
    }
}
