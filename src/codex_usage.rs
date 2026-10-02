use serde::Deserialize;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const TRANSCRIPT_TAIL_BYTES: u64 = 1024 * 1024;
const CODEX_BASELINE_TOKENS: u64 = 12_000;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReplyLink {
    pub url: String,
    pub label: Option<String>,
}

impl From<&str> for ReplyLink {
    fn from(url: &str) -> Self {
        Self {
            url: url.to_owned(),
            label: None,
        }
    }
}

impl std::ops::Deref for ReplyLink {
    type Target = str;
    fn deref(&self) -> &str {
        &self.url
    }
}

fn latest_reply_link(text: &str) -> Option<ReplyLink> {
    let url = crate::app::actions::latest_app_url(text)?;
    let start = url.as_ptr() as usize - text.as_ptr() as usize;
    let prefix = text.get(..start)?;
    let suffix = text.get(start + url.len()..)?;
    let label = if suffix.starts_with(')') {
        prefix.strip_suffix("](").and_then(|before| {
            let opening = before.rfind('[')?;
            if before[..opening].ends_with('!') {
                return None;
            }
            let label = before[opening + 1..].trim();
            (!label.is_empty()).then(|| label.to_owned())
        })
    } else {
        None
    };
    Some(ReplyLink {
        url: url.to_owned(),
        label,
    })
}

#[derive(Clone)]
struct ReplyUrlCacheEntry {
    len: u64,
    modified: Option<std::time::SystemTime>,
    url: Option<ReplyLink>,
}

static REPLY_URLS: OnceLock<Mutex<HashMap<PathBuf, ReplyUrlCacheEntry>>> = OnceLock::new();
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

/// Derive a link for the focused session's status display from assistant replies.
pub(crate) fn load_latest_reply_url(session_id: &str) -> Option<ReplyLink> {
    crate::codex_checkout::reply_session_lineage(session_id)
        .iter()
        .find_map(|id| load_session_reply_url(id))
}

fn load_session_reply_url(session_id: &str) -> Option<ReplyLink> {
    if !valid_session_id(session_id) {
        return None;
    }
    let path = cached_session_path(session_id)?;
    let metadata = std::fs::metadata(&path).ok()?;
    let modified = metadata.modified().ok();
    let cache = REPLY_URLS.get_or_init(|| Mutex::new(HashMap::new()));
    let previous = cache
        .lock()
        .ok()
        .and_then(|entries| entries.get(&path).cloned());
    if let Some(entry) = previous.as_ref() {
        if entry.len == metadata.len() && entry.modified == modified {
            return entry.url.clone();
        }
    }
    // Transcripts append. Scan new output with overlap for a previously partial record;
    // scan from the beginning on the first read or after a truncation/rewrite.
    let previous = previous.filter(|entry| entry.len < metadata.len());
    let lower_bound = previous
        .as_ref()
        .map_or(0, |entry| entry.len.saturating_sub(TRANSCRIPT_TAIL_BYTES));
    let url = read_latest_reply_url_since(&path, lower_bound)
        .or_else(|| previous.and_then(|entry| entry.url));
    if let Ok(mut entries) = cache.lock() {
        entries.insert(
            path,
            ReplyUrlCacheEntry {
                len: metadata.len(),
                modified,
                url: url.clone(),
            },
        );
    }
    url
}

/// Recognizable local hosts and common preview hostnames. No worktree lookup is needed.
pub(crate) fn is_app_url(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return false;
    }
    let host = if authority.starts_with('[') {
        authority
            .split(']')
            .next()
            .unwrap_or_default()
            .trim_start_matches('[')
    } else {
        authority.split(':').next().unwrap_or_default()
    }
    .to_ascii_lowercase();
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(ip) => ip.is_loopback() || ip.is_private(),
            std::net::IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local(),
        };
    }
    host == "localhost"
        || [
            ".localhost",
            ".local",
            ".test",
            ".vercel.app",
            ".netlify.app",
            ".pages.dev",
            ".trycloudflare.com",
        ]
        .iter()
        .any(|suffix| host.ends_with(suffix))
        || host
            .split(['.', '-'])
            .any(|part| matches!(part, "preview" | "sandbox" | "staging" | "dev"))
}

fn reply_url(line: &[u8]) -> Option<ReplyLink> {
    let record: serde_json::Value = serde_json::from_slice(line).ok()?;
    let payload = &record["payload"];
    if record["type"] == "response_item"
        && payload["type"] == "message"
        && payload["role"] == "assistant"
    {
        return payload["content"]
            .as_array()?
            .iter()
            .rev()
            .find_map(|part| {
                if part["type"] != "output_text" {
                    return None;
                }
                latest_reply_link(part["text"].as_str()?)
            });
    }
    None
}

#[cfg(test)]
fn read_latest_reply_url(path: &Path) -> Option<ReplyLink> {
    read_latest_reply_url_since(path, 0)
}

fn read_latest_reply_url_since(path: &Path, lower_bound: u64) -> Option<ReplyLink> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut end = file.metadata().ok()?.len();
    let mut remainder = Vec::new();
    while end > lower_bound {
        let start = end.saturating_sub(TRANSCRIPT_TAIL_BYTES).max(lower_bound);
        file.seek(SeekFrom::Start(start)).ok()?;
        let mut bytes = vec![0; (end - start) as usize];
        file.read_exact(&mut bytes).ok()?;
        bytes.extend_from_slice(&remainder);
        let first_line = if start == 0 {
            0
        } else {
            bytes
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|idx| idx + 1)
                .unwrap_or(bytes.len())
        };
        if let Some(url) = bytes[first_line..]
            .split(|byte| *byte == b'\n')
            .rev()
            .find_map(reply_url)
        {
            return Some(url);
        }
        remainder = bytes[..first_line].to_vec();
        end = start;
    }
    None
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
        if record.record_type == "event_msg" && record.payload.payload_type == "context_compacted" {
            return Some(None);
        }
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
        Some(Some(100u8.saturating_sub(remaining_percent.min(100) as u8)))
    })?
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
    fn reply_link_preserves_markdown_label_and_click_destination() {
        let link =
            latest_reply_link("[Call Log](http://outbound-dash.localhost:8766/calls)").unwrap();
        assert_eq!(link.label.as_deref(), Some("Call Log"));
        assert_eq!(link.url, "http://outbound-dash.localhost:8766/calls");
        assert_eq!(
            latest_reply_link("http://localhost:3000/calls")
                .unwrap()
                .label,
            None
        );
        assert_eq!(
            latest_reply_link("[old](http://localhost:3000/a) then [new](http://localhost:3000/b)")
                .unwrap()
                .label
                .as_deref(),
            Some("new")
        );
    }

    #[test]
    fn app_urls_include_local_and_preview_hosts_but_exclude_reference_links() {
        for url in [
            "http://localhost:8766/a",
            "http://outbound-dash.localhost:8766/a",
            "http://127.0.0.1:3000",
            "http://[::1]:3000",
            "http://192.168.1.5:3000",
            "https://sandbox.example.com/a",
            "https://branch.preview.example.com",
            "https://branch.vercel.app",
        ] {
            assert!(is_app_url(url), "{url}");
        }
        for url in [
            "https://github.com/a",
            "https://docs.example.com/preview",
            "https://localhost.evil.com",
            "https://myvercel.app",
            "https://localhost@github.com",
            "file:///tmp/test",
        ] {
            assert!(!is_app_url(url), "{url}");
        }
    }

    #[test]
    fn reply_urls_ignore_user_tool_and_reference_links_and_keep_last_app_link() {
        let record = |role: &str, text: &str| {
            serde_json::json!({
                "type": "response_item", "payload": {"type": "message", "role": role,
                "content": [{"type": "output_text", "text": text}]}
            })
            .to_string()
        };
        assert_eq!(
            reply_url(record("user", "http://localhost:3000").as_bytes()),
            None
        );
        assert_eq!(reply_url(br#"{"type":"response_item","payload":{"type":"function_call_output","output":"http://localhost:3000"}}"#), None);
        assert_eq!(reply_url(record("assistant", "Open [app](http://outbound-dash.localhost:8766/clients/a?tab=one). See https://github.com/a.").as_bytes()), Some(ReplyLink { url: "http://outbound-dash.localhost:8766/clients/a?tab=one".into(), label: Some("app".into()) }));
        assert_eq!(
            reply_url(
                record(
                    "assistant",
                    "http://localhost:3000/old then https://sandbox.example.com/new."
                )
                .as_bytes()
            ),
            Some("https://sandbox.example.com/new".into())
        );
        let path =
            std::env::temp_dir().join(format!("codex-reply-url-{}.jsonl", std::process::id()));
        let mut data = record("assistant", "[App](http://localhost:3000/a)");
        data.push('\n');
        data.push_str(&record("assistant", "See https://github.com/example/repo"));
        data.push('\n');
        // Make sure a later large tool result does not evict the last app link.
        data.push_str(&serde_json::json!({"type":"response_item", "payload":{"type":"function_call_output", "output":"x".repeat(TRANSCRIPT_TAIL_BYTES as usize + 100)}}).to_string());
        std::fs::write(&path, data).unwrap();
        assert_eq!(
            read_latest_reply_url(&path),
            Some(ReplyLink {
                url: "http://localhost:3000/a".into(),
                label: Some("App".into())
            })
        );
        let _ = std::fs::remove_file(path);
    }

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

    #[test]
    fn hides_usage_after_compaction_until_a_fresh_token_count() {
        let path =
            std::env::temp_dir().join(format!("codex-usage-compact-{}.jsonl", std::process::id()));
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"total_tokens\":89684},\"model_context_window\":258400}}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"context_compacted\"}}\n"
            ),
        )
        .unwrap();
        assert_eq!(read_latest_context_used(&path), None);

        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"context_compacted\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"total_tokens\":42000},\"model_context_window\":258400}}}\n"
            ),
        )
        .unwrap();
        assert!(read_latest_context_used(&path).is_some());
        let _ = std::fs::remove_file(path);
    }
}
