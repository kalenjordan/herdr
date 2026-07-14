use serde::Deserialize;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const TRANSCRIPT_TAIL_BYTES: u64 = 1024 * 1024;
const CODEX_BASELINE_TOKENS: u64 = 12_000;
static SESSION_PATHS: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();

#[derive(Deserialize)]
struct TranscriptRecord {
    #[serde(rename = "type")]
    record_type: String,
    payload: TranscriptPayload,
}

#[derive(Deserialize)]
struct TranscriptPayload {
    #[serde(rename = "type")]
    payload_type: String,
    info: Option<TokenInfo>,
}

#[derive(Deserialize)]
struct TokenInfo {
    last_token_usage: Option<TokenUsage>,
    model_context_window: Option<u64>,
}

#[derive(Deserialize)]
struct TokenUsage {
    total_tokens: u64,
}

pub(crate) fn load_context_used_percent(session_id: &str) -> Option<u8> {
    if !valid_session_id(session_id) {
        return None;
    }
    let path = cached_session_path(session_id)?;
    read_latest_context_used(&path)
}

fn cached_session_path(session_id: &str) -> Option<PathBuf> {
    let cache = SESSION_PATHS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(paths) = cache.lock() {
        if let Some(path) = paths.get(session_id).filter(|path| path.is_file()) {
            return Some(path.clone());
        }
    }
    let root = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))?
        .join("sessions");
    let path = find_session_path(&root, session_id)?;
    if let Ok(mut paths) = cache.lock() {
        paths.insert(session_id.to_string(), path.clone());
    }
    Some(path)
}

fn find_session_path(root: &Path, session_id: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_session_path(&path, session_id) {
                return Some(found);
            }
        } else if path.extension().is_some_and(|ext| ext == "jsonl")
            && path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().contains(session_id))
        {
            return Some(path);
        }
    }
    None
}

fn read_latest_context_used(path: &Path) -> Option<u8> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(TRANSCRIPT_TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut tail = String::new();
    file.read_to_string(&mut tail).ok()?;
    tail.lines().rev().find_map(|line| {
        let record = serde_json::from_str::<TranscriptRecord>(line).ok()?;
        if record.record_type != "event_msg" || record.payload.payload_type != "token_count" {
            return None;
        }
        let info = record.payload.info?;
        let used = info.last_token_usage?.total_tokens;
        let window = info
            .model_context_window
            .filter(|window| *window > CODEX_BASELINE_TOKENS)?;
        let effective_window = window - CODEX_BASELINE_TOKENS;
        let effective_used = used.saturating_sub(CODEX_BASELINE_TOKENS);
        let remaining = effective_window.saturating_sub(effective_used);
        let remaining_percent =
            (remaining.saturating_mul(100) + effective_window / 2) / effective_window;
        Some(100u8.saturating_sub(remaining_percent.min(100) as u8))
    })
}

fn valid_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_are_safe_path_components() {
        assert!(valid_session_id("019f5c38-dc0e-7311-b581-be3ebefa8509"));
        assert!(!valid_session_id("../session"));
        assert!(!valid_session_id("session/id"));
    }

    #[test]
    fn reads_latest_context_usage_from_transcript_tail() {
        let path = std::env::temp_dir().join(format!("codex-usage-{}.jsonl", std::process::id()));
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"total_tokens\":20},\"model_context_window\":100}}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"total_tokens\":89684},\"model_context_window\":258400}}}\n"
            ),
        )
        .unwrap();
        assert_eq!(read_latest_context_used(&path), Some(32));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn matches_codex_baseline_and_rounding() {
        let path =
            std::env::temp_dir().join(format!("codex-usage-rounding-{}.jsonl", std::process::id()));
        std::fs::write(
            &path,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"total_tokens\":94462},\"model_context_window\":258400}}}\n",
        )
        .unwrap();
        assert_eq!(read_latest_context_used(&path), Some(33));
        let _ = std::fs::remove_file(path);
    }
}
