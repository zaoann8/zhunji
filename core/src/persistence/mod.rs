#![cfg_attr(target_os = "linux", allow(dead_code, unused_variables))]
//! Local persistence: history JSON, user preferences JSON, and
//! platform-backed credentials vault.
//!
//! Storage roots:
//! - macOS:   `~/Library/Application Support/Zhunji`
//! - Windows: `%APPDATA%\OpenLess`
//! - Linux:   `$XDG_DATA_HOME/OpenLess` or `~/.local/share/OpenLess`
//!
//! Credential storage policy: provider credentials are stored in the OS
//! credential vault (macOS Keychain, Windows Credential Manager, Linux keyring).
//! A legacy plaintext JSON file is read once as a migration source and removed
//! after a successful vault write; new writes never persist plaintext secrets.
//!
//! This module is split into focused submodules; everything that was previously
//! reachable as `crate::persistence::*` stays reachable via the glob re-exports
//! below. The shared filesystem helpers and the two cross-cutting constants live
//! here so every submodule can reach them through `super::`.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use uuid::Uuid;

mod activity;
mod credentials;
mod history;
mod paths;
mod preferences;

pub use activity::*;
pub use credentials::*;
pub use history::*;
pub use paths::*;
pub use preferences::*;

const HISTORY_CAP: usize = 5000;
const PREFERENCES_FILE: &str = "preferences.json";

pub fn data_dir() -> Result<PathBuf> {
    // 测试隔离：Coordinator::new() 等大量测试构造真实组件，若走生产路径，测试里
    // prefs.set()（很多用 `..Default::default()` 构造）会把默认值整盘覆盖用户配置
    // （曾发生：跑完 cargo test 后用户的 hold 热键/2000 条上限/provider 全被重置）。
    // cfg(test) 时改用进程级 temp 目录，测试互相同享但绝不触碰真实数据。
    #[cfg(test)]
    {
        static TEST_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        return Ok(TEST_DIR
            .get_or_init(|| {
                std::env::temp_dir().join(format!("openless-test-data-{}", std::process::id()))
            })
            .clone());
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").context("HOME not set")?;
        Ok(PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("Zhunji"))
    }

    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").context("APPDATA not set")?;
        Ok(PathBuf::from(appdata).join("OpenLess"))
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            if !xdg.is_empty() {
                return Ok(PathBuf::from(xdg).join("OpenLess"));
            }
        }
        let home = std::env::var("HOME").context("HOME not set")?;
        Ok(PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("OpenLess"))
    }
}

/// Fallback store path when `data_dir()` is unavailable.
fn fallback_store_path(file_name: &str) -> PathBuf {
    std::env::temp_dir().join(file_name)
}

fn ensure_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("create dir failed: {}", dir.display()))?;
    Ok(())
}

/// Atomic write: write to a unique `*.tmp-<uuid>` first, then rename onto the
/// target path. The unique suffix lets concurrent writers each own their own
/// tmp file, so a parallel rename never finds its source already taken.
fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    if path.as_os_str().is_empty() {
        bail!("atomic write refused: empty path (memory-only store)");
    }
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp_path = path.with_file_name(format!("{file_name}.tmp-{}", Uuid::new_v4().simple()));
    fs::write(&tmp_path, contents)
        .with_context(|| format!("write tmp failed: {}", tmp_path.display()))?;
    if let Err(err) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(err).with_context(|| format!("rename failed: {}", path.display()));
    }
    Ok(())
}

fn read_or_default<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T> {
    if !path.exists() {
        return Ok(T::default());
    }
    let bytes = fs::read(path).with_context(|| format!("read failed: {}", path.display()))?;
    if bytes.is_empty() {
        return Ok(T::default());
    }
    serde_json::from_slice::<T>(&bytes)
        .with_context(|| format!("decode failed: {}", path.display()))
}
