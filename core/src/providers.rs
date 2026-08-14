//! ASR 供应商 CRUD — 从 zhunlu/src-tauri/src/commands/providers.rs 迁移（纯文件逻辑）。

use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::persistence;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    pub id: String,
    pub name: String,
    #[serde(alias = "endpoint")]
    pub url: String,
    #[serde(alias = "api_key")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub default: bool,
}

#[allow(dead_code)] // P2 供应商管理页接回
static LOCK: Mutex<()> = Mutex::new(());

fn providers_path() -> PathBuf {
    persistence::data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("providers.json")
}

fn read() -> Vec<Provider> {
    let path = providers_path();
    let mut list: Vec<Provider> = if !path.exists() {
        vec![]
    } else {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<Provider>>(&s).ok())
            .unwrap_or_default()
    };
    // 确保内置豆包始终在列表里
    if !list.iter().any(|p| p.id == "builtin-doubao") {
        let is_first = list.is_empty();
        list.insert(
            0,
            Provider {
                id: "builtin-doubao".into(),
                name: "豆包 IME".into(),
                url: String::new(),
                api_key: None,
                notes: Some("内置免费引擎，无需配置".into()),
                default: is_first,
            },
        );
    }
    list
}

#[allow(dead_code)] // P2 供应商管理页接回
fn write(list: &[Provider]) {
    let path = providers_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec_pretty(list) {
        let _ = std::fs::write(&path, json);
    }
}

pub fn list_providers() -> Result<Vec<Provider>, String> {
    let _lock = LOCK.lock();
    Ok(read())
}

#[allow(dead_code)] // P2 供应商管理页接回
pub fn add_provider(
    name: String,
    url: String,
    api_key: Option<String>,
    notes: Option<String>,
) -> Result<Provider, String> {
    let _lock = LOCK.lock();
    let mut list = read();
    let p = Provider {
        id: Uuid::new_v4().to_string(),
        name,
        url,
        api_key: api_key.filter(|s| !s.is_empty()),
        notes: notes.filter(|s| !s.is_empty()),
        default: false, // 新增永远不自动设为默认，用户手动选
    };
    list.push(p.clone());
    write(&list);
    Ok(p)
}

#[allow(dead_code)] // P2 供应商管理页接回
pub fn update_provider(
    id: String,
    name: String,
    url: String,
    api_key: Option<String>,
    notes: Option<String>,
) -> Result<(), String> {
    let _lock = LOCK.lock();
    let mut list = read();
    if let Some(p) = list.iter_mut().find(|p| p.id == id) {
        p.name = name;
        p.url = url;
        p.api_key = api_key.filter(|s| !s.is_empty());
        p.notes = notes.filter(|s| !s.is_empty());
        write(&list);
        Ok(())
    } else {
        Err("供应商不存在".into())
    }
}

#[allow(dead_code)] // P2 供应商管理页接回
pub fn remove_provider(id: String) -> Result<(), String> {
    if id == "builtin-doubao" {
        return Err("内置豆包引擎不可删除".into());
    }
    let _lock = LOCK.lock();
    let mut list = read();
    let was_default = list.iter().any(|p| p.id == id && p.default);
    list.retain(|p| p.id != id);
    // 如果删的是默认引擎，切回豆包
    if was_default {
        if let Some(d) = list.iter_mut().find(|p| p.id == "builtin-doubao") {
            d.default = true;
        }
    }
    write(&list);
    Ok(())
}

#[allow(dead_code)] // P2 供应商管理页接回
pub fn set_default_provider(id: String) -> Result<(), String> {
    let _lock = LOCK.lock();
    let mut list = read();
    let mut found = false;
    for p in &mut list {
        p.default = p.id == id;
        if p.id == id {
            found = true;
        }
    }
    if !found {
        return Err("供应商不存在".into());
    }
    write(&list);
    Ok(())
}

#[allow(dead_code)] // P2 供应商管理页接回
pub async fn test_provider(id: String) -> Result<(), String> {
    let provider = {
        let _lock = LOCK.lock();
        read().iter().find(|p| p.id == id).cloned()
    }
    .ok_or("供应商不存在")?;

    if provider.url.is_empty() {
        return Err("请先填写 URL".into());
    }
    let url = format!("{}/v1/models", provider.url.trim_end_matches('/'));
    let client = crate::net::http();
    let mut req = client.get(&url).timeout(std::time::Duration::from_secs(10));
    if let Some(ref key) = provider.api_key {
        req = req.bearer_auth(key);
    }
    let resp = req.send().await.map_err(|e| format!("网络错误: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("返回 {}", resp.status()))
    }
}
