//! 共享 HTTP 客户端。
//!
//! 背景：原先每个网络命令各自 `reqwest::Client::new()`，连接池互不复用 —— 一次
//! 成功的 TLS 连接用完即弃，下一个命令又得重新握手。在握手不稳定的网络下（代理
//! 分流等）首次握手经常被重置，用户得反复重试才能用。
//!
//! `http()`：进程级共享客户端。一次握手成功后的连接进连接池，后续命令直接复用，
//!   不再付握手成本。代理开关切换后清空缓存按新策略重建。
//!
//!   可能发生在服务端已收到之后（重试 POST / DELETE 会重复执行）；`is_request()`
//!   类错误多为确定性失败（如 endpoint 配置错误），重试只是徒增数秒延迟。HTTP
//!   4xx/5xx 同样不重试 —— 服务端已应答，状态码交给调用方判断。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use once_cell::sync::Lazy;
use parking_lot::Mutex;

/// 用户是否允许 app 使用系统代理（issue #869）。默认 true = 跟随系统代理，
/// 与历史行为一致；关闭后所有 reqwest 客户端 `.no_proxy()` 直连。
/// 启动时由 coordinator 用持久化设置初始化，`set_settings` 变更时同步。
static USE_SYSTEM_PROXY: AtomicBool = AtomicBool::new(true);

/// 共享 / provider 客户端的构建缓存。key = `(discriminator, no_proxy 决策)`。
/// 代理开关变化时整表清空重建，保证「存盘即生效」。
static CACHE: Lazy<Mutex<HashMap<(u64, bool), reqwest::Client>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// 当前是否使用系统代理（false = 所有请求直连）。
pub(crate) fn use_system_proxy() -> bool {
    USE_SYSTEM_PROXY.load(Ordering::Relaxed)
}

/// 更新系统代理开关并清空客户端缓存，让后续请求立即按新策略重建连接池。
/// 在启动初始化与 `set_settings` 中设置值变化时调用。
pub(crate) fn set_use_system_proxy(enabled: bool) {
    USE_SYSTEM_PROXY.store(enabled, Ordering::Relaxed);
    CACHE.lock().clear();
}

/// 判定某 base_url 是否应绕过系统代理：回环地址恒绕过（localhost 走代理没有
/// 意义且可能自环）；全局关闭系统代理时所有地址绕过（issue #869）。
/// 共享客户端的基础 builder：握手限时 + 连接池 + UA；按需禁用系统代理。
fn base_client_builder(no_proxy: bool) -> reqwest::ClientBuilder {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        // 连接池：一条握手成功的连接保留 90s 供后续命令复用。
        .pool_idle_timeout(Duration::from_secs(90))
        .pool_max_idle_per_host(8)
        .tcp_keepalive(Duration::from_secs(30))
        .user_agent(concat!("zhunji/", env!("CARGO_PKG_VERSION")));
    if no_proxy {
        builder = builder.no_proxy();
    }
    builder
}

/// 进程级共享 HTTP 客户端。带连接池 —— 一次握手成功后的连接被后续请求复用；
/// 代理开关切换后经 CACHE 清空自动按新策略重建。
pub fn http() -> reqwest::Client {
    let no_proxy = !use_system_proxy();
    cached_client((0, no_proxy), || {
        base_client_builder(no_proxy)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// 按 `(key, no_proxy)` 缓存并复用 `reqwest::Client`。相同配置的后续调用直接
/// `clone()` 复用同一连接池（`reqwest::Client` 内部是 `Arc`）。
/// `build` 只在首次 miss 时调用，必须产出与该 `key` 语义一致的客户端。
pub fn cached_client<F>(key: (u64, bool), build: F) -> reqwest::Client
where
    F: FnOnce() -> reqwest::Client,
{
    CACHE.lock().entry(key).or_insert_with(build).clone()
}

#[cfg(test)]
mod tests {
    use super::{http, set_use_system_proxy, use_system_proxy, CACHE};

    #[test]
    fn system_proxy_toggle_updates_flag_and_rebuilds_shared_client() {
        set_use_system_proxy(true);
        CACHE.lock().clear();
        let _ = http();
        assert!(!CACHE.lock().is_empty());
        set_use_system_proxy(false);
        assert!(!use_system_proxy());
        // 下一次 http() 按「直连」决策重建（key 的 bool 位 = no_proxy）。
        let _ = http();
        assert!(CACHE.lock().contains_key(&(0, true)));
        set_use_system_proxy(true);
        assert!(use_system_proxy());
    }
}
