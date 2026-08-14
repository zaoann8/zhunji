//! 热词词典 — 从 zhunlu/src-tauri/src/commands/dictionary.rs 迁移（纯文件逻辑）。

use std::path::PathBuf;

use parking_lot::Mutex;

use crate::persistence;

#[allow(dead_code)] // P2 词典页接回
const MAX_TERMS: usize = 100;
#[allow(dead_code)] // P2 词典页接回
const MAX_TERM_LEN: usize = 50;

#[allow(dead_code)] // P2 词典页接回
static LOCK: Mutex<()> = Mutex::new(());

fn path() -> PathBuf {
    persistence::data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("dictionary.json")
}

fn read() -> Vec<String> {
    let p = path();
    if !p.exists() {
        return vec![];
    }
    std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

#[allow(dead_code)] // P2 词典页接回
fn write(list: &[String]) {
    let p = path();
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec_pretty(list) {
        let _ = std::fs::write(&p, json);
    }
}

#[allow(dead_code)] // P2 词典页接回
pub fn list_terms() -> Result<Vec<String>, String> {
    let _lock = LOCK.lock();
    Ok(read())
}

#[allow(dead_code)] // P2 词典页接回
pub fn add_term(term: String) -> Result<(), String> {
    let t = term.trim().to_string();
    if t.is_empty() {
        return Err("热词不能为空".into());
    }
    if t.chars().count() > MAX_TERM_LEN {
        return Err(format!("热词不超过 {} 个字符", MAX_TERM_LEN));
    }
    let _lock = LOCK.lock();
    let mut list = read();
    if list.iter().any(|x| x == &t) {
        return Err("热词已存在".into());
    }
    if list.len() >= MAX_TERMS {
        return Err(format!("最多 {} 条热词", MAX_TERMS));
    }
    list.push(t);
    write(&list);
    Ok(())
}

#[allow(dead_code)] // P2 词典页接回
pub fn remove_term(term: String) -> Result<(), String> {
    let _lock = LOCK.lock();
    let mut list = read();
    list.retain(|x| x != &term);
    write(&list);
    Ok(())
}

/// 获取当前词典，供 coordinator 读取后透传 keyterm。
pub fn get_terms() -> Vec<String> {
    read()
}
