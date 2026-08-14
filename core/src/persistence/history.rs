#![cfg_attr(target_os = "linux", allow(dead_code, unused_variables))]
//! Dictation history store: newest-first JSON list with retention + count caps.

use std::path::PathBuf;

use anyhow::{Context, Result};
use parking_lot::Mutex;

use super::{
    atomic_write, data_dir, ensure_dir, paths::recordings_root, read_or_default, HISTORY_CAP,
};
use crate::types::DictationSession;

const HISTORY_FILE: &str = "history.json";

pub struct HistoryStore {
    path: PathBuf,
    lock: Mutex<()>,
}

#[allow(dead_code)] // P2 历史页接回
impl HistoryStore {
    pub fn new() -> Result<Self> {
        let dir = data_dir()?;
        ensure_dir(&dir)?;
        Ok(Self {
            path: dir.join(HISTORY_FILE),
            lock: Mutex::new(()),
        })
    }

    /// 在 data_dir 不可用时构造一个降级实例。
    /// Android 使用空 path（内存态），禁止落 `/data/local/tmp`。
    pub(crate) fn new_fallback() -> Self {
        Self {
            path: super::fallback_store_path("openless_history_fallback.json"),
            lock: Mutex::new(()),
        }
    }

    pub fn list(&self) -> Result<Vec<DictationSession>> {
        let _guard = self.lock.lock();
        self.read_locked()
    }

    /// `retention_days == 0` 跟旧 append 行为一致（不按时间清理）。
    /// `> 0` 时在写入新条目后顺手把超过 N 天的会话裁掉，写入时就完成清理，
    /// 不需要后台轮询。最后再受条数上限约束：
    /// - `max_entries == None` → HISTORY_CAP (200)
    /// - `max_entries == Some(n)` → clamp 到 5..=HISTORY_CAP，避免用户填 0 / 极大值。
    pub fn append_with_retention(
        &self,
        session: DictationSession,
        retention_days: u32,
        max_entries: Option<u32>,
    ) -> Result<()> {
        let _guard = self.lock.lock();
        let mut sessions = self.read_locked()?;
        // Prepend so the newest session is at index 0, matching the Swift impl.
        sessions.insert(0, session);
        if retention_days > 0 {
            let cutoff = chrono::Utc::now() - chrono::Duration::days(i64::from(retention_days));
            sessions.retain(|s| {
                chrono::DateTime::parse_from_rfc3339(&s.created_at)
                    .map(|t| t.with_timezone(&chrono::Utc) >= cutoff)
                    // 解析失败时保守保留——避免错误的时间戳让用户丢历史。
                    .unwrap_or(true)
            });
        }
        let cap = max_entries
            .map(|n| (n as usize).clamp(5, HISTORY_CAP))
            .unwrap_or(HISTORY_CAP);
        if sessions.len() > cap {
            sessions.truncate(cap);
        }
        self.write_locked(&sessions)
    }

    /// 用户修改保留策略（天数 / 条数上限）后立即裁剪现有历史。
    /// 与 append_with_retention 的裁剪规则一致，只对存量生效、不追加新条目。
    pub fn apply_retention(&self, retention_days: u32, max_entries: Option<u32>) -> Result<()> {
        let _guard = self.lock.lock();
        let mut sessions = self.read_locked()?;
        if retention_days > 0 {
            let cutoff = chrono::Utc::now() - chrono::Duration::days(i64::from(retention_days));
            sessions.retain(|s| {
                chrono::DateTime::parse_from_rfc3339(&s.created_at)
                    .map(|t| t.with_timezone(&chrono::Utc) >= cutoff)
                    .unwrap_or(true)
            });
        }
        let cap = max_entries
            .map(|n| (n as usize).clamp(5, HISTORY_CAP))
            .unwrap_or(HISTORY_CAP);
        if sessions.len() > cap {
            sessions.truncate(cap);
        }
        self.write_locked(&sessions)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let _guard = self.lock.lock();
        let mut sessions = self.read_locked()?;
        let original_len = sessions.len();
        sessions.retain(|s| s.id != id);
        if sessions.len() == original_len {
            return Ok(());
        }
        self.write_locked(&sessions)?;
        // 联动删除录音文件（<data_dir>/recordings/<id>.wav），避免历史删了录音还占磁盘。
        if let Ok(root) = recordings_root() {
            let wav = root.join(format!("{id}.wav"));
            if wav.exists() {
                if let Err(e) = std::fs::remove_file(&wav) {
                    log::warn!("[history] delete recording failed for {id}: {e}");
                }
            }
        }
        Ok(())
    }

    pub fn update_entry(&self, updated: DictationSession) -> Result<bool> {
        let _guard = self.lock.lock();
        let mut sessions = self.read_locked()?;
        let Some(slot) = sessions.iter_mut().find(|s| s.id == updated.id) else {
            return Ok(false);
        };
        *slot = updated;
        self.write_locked(&sessions)?;
        Ok(true)
    }

    pub fn clear(&self) -> Result<()> {
        let _guard = self.lock.lock();
        self.write_locked(&Vec::<DictationSession>::new())?;
        // 清空历史同样联动清空录音归档目录。失败仅告警不阻断（录音是次要资产）。
        if let Ok(root) = recordings_root() {
            if let Ok(entries) = std::fs::read_dir(&root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|e| e == "wav") {
                        if let Err(e) = std::fs::remove_file(&path) {
                            log::warn!("[history] clear recording failed for {path:?}: {e}");
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn read_locked(&self) -> Result<Vec<DictationSession>> {
        read_or_default::<Vec<DictationSession>>(&self.path)
    }

    fn write_locked(&self, sessions: &[DictationSession]) -> Result<()> {
        let json = serde_json::to_vec_pretty(sessions).context("encode history failed")?;
        atomic_write(&self.path, &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{HistorySource, InsertStatus, PolishMode};

    fn session(id: &str, created_at: &str) -> DictationSession {
        DictationSession {
            id: id.into(),
            created_at: created_at.into(),
            source: HistorySource::Voice,
            raw_transcript: "x".into(),
            final_text: "x".into(),
            mode: PolishMode::Raw,
            style_pack_id: None,
            translation_active: false,
            polish_source: None,
            app_bundle_id: None,
            app_name: None,
            insert_status: InsertStatus::Inserted,
            error_code: None,
            duration_ms: None,
            dictionary_entry_count: None,
            has_audio_recording: None,
            asr_provider: None,
            asr_model: None,
            llm_provider: None,
            llm_model: None,
            asr_ms: None,
            polish_ms: None,
        }
    }

    // 唯一 temp 文件，避免并行测试互相踩同一个 fallback 路径。
    fn store() -> HistoryStore {
        let path = std::env::temp_dir().join(format!(
            "openless-history-test-{}-{:?}.json",
            std::process::id(),
            std::thread::current().id()
        ));
        HistoryStore {
            path,
            lock: Mutex::new(()),
        }
    }

    #[test]
    fn apply_retention_truncates_to_max_entries_keeping_newest() {
        let store = store();
        for i in 0..8 {
            store
                .append_with_retention(
                    session(&format!("s{i}"), &format!("2026-08-0{i}T10:00:00Z")),
                    0,
                    None,
                )
                .unwrap();
        }
        // 上限 5：只留最新的 5 条（newest-first，index 0 最新）。
        store.apply_retention(0, Some(5)).unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 5);
        assert_eq!(list[0].id, "s7");
        assert_eq!(list[4].id, "s3");
    }

    #[test]
    fn apply_retention_truncates_by_days() {
        let store = store();
        store
            .append_with_retention(session("old", "2020-01-01T00:00:00Z"), 0, None)
            .unwrap();
        store
            .append_with_retention(session("new", "2026-08-01T00:00:00Z"), 0, None)
            .unwrap();
        store.apply_retention(30, None).unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "new");
    }
}
