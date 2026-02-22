//! Watch module - Monitors AI CLI log files and detects task completions
//!
//! This module implements file monitoring for:
//! - Claude: ~/.claude/projects/*.jsonl
//! - Codex: ~/.codex/sessions/*.jsonl
//! - Gemini: ~/.gemini/tmp/chats/session-*.json

use serde_json::Value;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::interval;

use crate::config::ConfirmAlertConfig;
use crate::notify;

/// Confirm detection keywords (Chinese and English)
const CONFIRM_KEYWORDS_CN: &[&str] = &[
    "是否",
    "要不要",
    "能否",
    "可否",
    "可以吗",
    "可以么",
    "请确认",
    "确认一下",
    "是否确认",
    "是否继续",
    "同意",
    "允许",
    "授权",
    "批准",
];

const CONFIRM_KEYWORDS_EN: &[&str] = &[
    "confirm",
    "confirmation",
    "approve",
    "approval",
    "okay to",
    "is it ok",
    "is it okay",
    "shall i",
    "should i",
    "would you like",
    "do you want me",
    "may i",
    "permission",
    "allow",
    "authorize",
    "await your",
    "waiting for your",
];

/// Dedupe time for confirm notifications (15 seconds)
const CONFIRM_DEDUPE_MS: i64 = 15000;

/// Seed bytes for JSONL follower (256KB)
const SEED_BYTES: usize = 256 * 1024;

/// Metadata for seed events
#[derive(Debug, Clone)]
pub struct SeedMeta {
    pub seed: bool,
}

/// JSONL file follower for streaming line-by-line parsing
pub struct JsonlFollower {
    seed_bytes: usize,
    file_path: Option<PathBuf>,
    position: u64,
    partial: String,
}

impl JsonlFollower {
    /// Create a new JSONL follower
    pub fn new(seed_bytes: usize) -> Self {
        Self {
            seed_bytes,
            file_path: None,
            position: 0,
            partial: String::new(),
        }
    }

    /// Attach to a file and optionally emit seed events
    pub fn attach<F>(&mut self, file_path: PathBuf, mut on_object: F)
    where
        F: FnMut(Value, SeedMeta),
    {
        let stat = match safe_stat(&file_path) {
            Some(s) => s,
            None => return,
        };

        self.file_path = Some(file_path.clone());
        self.position = stat.size;
        self.partial = String::new();

        // Read seed content from the end of the file
        let start = if stat.size > self.seed_bytes {
            stat.size - self.seed_bytes
        } else {
            0
        };

        if let Ok(seed_text) = read_file_slice_utf8(&file_path, start, stat.size - start) {
            let mut lines: Vec<&str> = seed_text.split('\n').collect();
            if start > 0 {
                lines = lines.into_iter().skip(1).collect();
            }

            for line in lines {
                if line.is_empty() {
                    continue;
                }
                if let Some(obj) = safe_json_parse(line) {
                    on_object(obj, SeedMeta { seed: true });
                }
            }
        }
    }

    /// Poll for new content
    pub fn poll<F>(&mut self, mut on_object: F)
    where
        F: FnMut(Value, SeedMeta),
    {
        let file_path = match &self.file_path {
            Some(p) => p,
            None => return,
        };

        let stat = match safe_stat(file_path) {
            Some(s) => s,
            None => return,
        };

        // File was truncated, reset position
        if stat.size < self.position {
            self.position = 0;
            self.partial = String::new();
        }

        // No new content
        if stat.size == self.position {
            return;
        }

        // Read new content
        let chunk = match read_file_slice_utf8(file_path, self.position, stat.size - self.position) {
            Ok(c) => c,
            Err(_) => return,
        };
        self.position = stat.size;

        // Split into lines, keep incomplete last line
        let text = format!("{}{}", self.partial, chunk);
        let mut lines: Vec<&str> = text.split('\n').collect();

        // Keep last incomplete line as partial
        if let Some(last) = lines.pop() {
            self.partial = last.to_string();
        }

        // Process complete lines
        for line in lines {
            if line.is_empty() {
                continue;
            }
            if let Some(obj) = safe_json_parse(line) {
                on_object(obj, SeedMeta { seed: false });
            }
        }
    }

    /// Get current file path
    pub fn file_path(&self) -> Option<&PathBuf> {
        self.file_path.as_ref()
    }

    /// Set file position
    pub fn set_position(&mut self, position: u64) {
        self.position = position;
    }
}

/// File stat information
struct FileStat {
    size: u64,
    mtime_ms: i64,
}

/// Safe file stat
fn safe_stat(path: &Path) -> Option<FileStat> {
    fs::metadata(path).ok().map(|m| {
        let mtime = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        FileStat {
            size: m.len(),
            mtime_ms: mtime,
        }
    })
}

/// Read a slice of a file as UTF-8
fn read_file_slice_utf8(path: &Path, start: u64, length: u64) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;

    let max_bytes = usize::max(0, length as usize);
    let mut buffer = vec![0u8; max_bytes];
    let bytes_read = file.read(&mut buffer)?;
    buffer.truncate(bytes_read);

    Ok(String::from_utf8_lossy(&buffer).to_string())
}

/// Safe JSON parse
pub fn safe_json_parse(line: &str) -> Option<Value> {
    let normalized = line.replace('\u{feff}', "");
    serde_json::from_str(&normalized).ok()
}

/// Parse timestamp from JSON value (supports Unix seconds/milliseconds and ISO 8601)
pub fn parse_timestamp(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64().map(|v| {
            if v < 1_000_000_000_000i64 {
                v * 1000
            } else {
                v
            }
        }),
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return None;
            }

            // Try numeric string
            if let Ok(num) = trimmed.parse::<i64>() {
                return Some(if num < 1_000_000_000_000i64 {
                    num * 1000
                } else {
                    num
                });
            }

            // Try float string
            if let Ok(num) = trimmed.parse::<f64>() {
                return Some(if num < 1_000_000_000_000.0 {
                    (num * 1000.0) as i64
                } else {
                    num as i64
                });
            }

            // Try ISO 8601
            chrono::DateTime::parse_from_rfc3339(trimmed)
                .ok()
                .map(|dt| dt.timestamp_millis())
        }
        _ => None,
    }
}

/// Check if message has content of specific type
pub fn has_content_type(message: &Value, expected_type: &str) -> bool {
    message
        .get("content")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter().any(|item| {
                item.get("type")
                    .and_then(|t| t.as_str())
                    .map(|t| t == expected_type)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

/// Extract text from a value (handles various formats)
fn extract_text_from_any(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr
            .iter()
            .map(extract_text_from_any)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(obj) => {
            // Try common text fields
            for field in ["text", "content", "message", "value", "data"] {
                if let Some(text) = obj.get(field).and_then(|v| v.as_str()) {
                    return text.to_string();
                }
                // Recursively check content arrays
                if let Some(content) = obj.get(field).and_then(|v| v.as_array()) {
                    let result = Value::Array(content.clone());
                    let extracted = extract_text_from_any(&result);
                    if !extracted.is_empty() {
                        return extracted;
                    }
                }
            }
            String::new()
        }
        _ => String::new(),
    }
}

/// Extract message text from a message object
pub fn extract_message_text(message: &Value) -> String {
    match message {
        Value::Object(obj) => {
            // Check content array first
            if let Some(content) = obj.get("content").and_then(|c| c.as_array()) {
                let result = Value::Array(content.clone());
                return extract_text_from_any(&result);
            }

            // Check content string
            if let Some(content) = obj.get("content").and_then(|c| c.as_str()) {
                return content.to_string();
            }

            // Fallback to general extraction
            extract_text_from_any(message)
        }
        _ => extract_text_from_any(message),
    }
}

/// Confirm detector for interactive prompts
pub struct ConfirmDetector {
    enabled: bool,
}

impl ConfirmDetector {
    /// Create a new confirm detector
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Detect if text contains a confirmation prompt
    pub fn detect(&self, text: &str) -> Option<String> {
        if !self.enabled {
            return None;
        }

        let text_lower = text.to_lowercase();

        // Check Chinese keywords
        for keyword in CONFIRM_KEYWORDS_CN {
            if text.contains(keyword) {
                return Some(truncate_text(text, 600));
            }
        }

        // Check English keywords
        for keyword in CONFIRM_KEYWORDS_EN {
            if text_lower.contains(keyword) {
                return Some(truncate_text(text, 600));
            }
        }

        None
    }

    /// Check if detector is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Truncate text to max length
fn truncate_text(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        format!("{}...", &text[..max_len.saturating_sub(3)])
    }
}

/// Normalize confirm text for deduplication
fn normalize_confirm_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Stop handle for watch operations
pub struct StopHandle {
    stop_tx: Option<oneshot::Sender<()>>,
}

impl StopHandle {
    pub fn new(stop_tx: oneshot::Sender<()>) -> Self {
        Self {
            stop_tx: Some(stop_tx),
        }
    }

    pub fn stop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Claude state
#[derive(Clone)]
struct ClaudeState {
    current_file: Option<PathBuf>,
    last_user_text_at: Option<i64>,
    last_assistant_at: Option<i64>,
    last_notified_at: Option<i64>,
    notified_for_turn: bool,
    confirm_notified_for_turn: bool,
    last_cwd: Option<String>,
    last_assistant_content: Option<String>,
    last_assistant_had_tool_use: bool,
    last_user_text: String,
    last_assistant_text: String,
    last_confirm_key: String,
    last_confirm_at: i64,
}

impl ClaudeState {
    fn new() -> Self {
        Self {
            current_file: None,
            last_user_text_at: None,
            last_assistant_at: None,
            last_notified_at: None,
            notified_for_turn: false,
            confirm_notified_for_turn: false,
            last_cwd: None,
            last_assistant_content: None,
            last_assistant_had_tool_use: false,
            last_user_text: String::new(),
            last_assistant_text: String::new(),
            last_confirm_key: String::new(),
            last_confirm_at: 0,
        }
    }
}

/// Find latest file matching predicate
fn find_latest_file<F>(root_dir: &Path, is_candidate: F) -> Option<PathBuf>
where
    F: Fn(&Path, &str) -> bool,
{
    let mut latest: Option<(PathBuf, i64)> = None;

    fn walk<F>(dir: &Path, is_candidate: &F, latest: &mut Option<(PathBuf, i64)>)
    where
        F: Fn(&Path, &str) -> bool,
    {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, is_candidate, latest);
                continue;
            }

            if !path.is_file() {
                continue;
            }

            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !is_candidate(&path, name) {
                continue;
            }

            if let Ok(meta) = fs::metadata(&path) {
                if let Some(mtime) = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                {
                    let mtime_ms = mtime.as_millis() as i64;
                    if latest.is_none() || mtime_ms > latest.as_ref().unwrap().1 {
                        *latest = Some((path.clone(), mtime_ms));
                    }
                }
            }
        }
    }

    walk(root_dir, &is_candidate, &mut latest);
    latest.map(|(path, _)| path)
}

/// Find latest N files matching predicate
fn find_latest_files<F>(root_dir: &Path, is_candidate: F, limit: usize) -> Vec<PathBuf>
where
    F: Fn(&Path, &str) -> bool,
{
    let mut files: Vec<(PathBuf, i64)> = Vec::new();

    fn walk<F>(dir: &Path, is_candidate: &F, files: &mut Vec<(PathBuf, i64)>)
    where
        F: Fn(&Path, &str) -> bool,
    {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, is_candidate, files);
                continue;
            }

            if !path.is_file() {
                continue;
            }

            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !is_candidate(&path, name) {
                continue;
            }

            if let Ok(meta) = fs::metadata(&path) {
                if let Some(mtime) = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                {
                    let mtime_ms = mtime.as_millis() as i64;
                    files.push((path.clone(), mtime_ms));
                }
            }
        }
    }

    walk(root_dir, &is_candidate, &mut files);

    // Sort by mtime descending and take limit
    files.sort_by(|a, b| b.1.cmp(&a.1));
    files.truncate(limit);
    files.into_iter().map(|(path, _)| path).collect()
}

/// Summarize notification result
fn summarize_result(result: &Value) -> String {
    if result.get("skipped").and_then(|v| v.as_bool()).unwrap_or(false) {
        return format!(
            "skipped: {}",
            result
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("")
        );
    }

    let results = result
        .get("results")
        .and_then(|r| r.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);
    let ok = result
        .get("results")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|r| r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false))
                .count()
        })
        .unwrap_or(0);

    format!("sent: {}/{}", ok, results)
}

/// Get home directory
fn get_home_dir() -> Option<PathBuf> {
    // Try environment variable first
    if let Ok(dir) = std::env::var("AI_CLI_COMPLETE_NOTIFY_HOME") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            return Some(PathBuf::from(userprofile));
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return Some(PathBuf::from(home));
        }
    }

    None
}

/// Send confirm notification
async fn send_confirm_notification(source: &str, log_cb: &dyn Fn(String)) -> bool {
    match notify::send_notifications(
        source,
        "确认提醒",
        None,
        std::env::current_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        true,
    )
    .await
    {
        Ok(result) => {
            log_cb(format!(
                "[watch][confirm:{}] {}",
                source,
                summarize_result(&result)
            ));
            true
        }
        Err(e) => {
            log_cb(format!("[watch][confirm:{}] error: {}", source, e));
            false
        }
    }
}

/// Send completion notification
async fn send_completion_notification(
    source: &str,
    task_info: &str,
    duration_ms: Option<i64>,
    cwd: String,
    log_cb: &dyn Fn(String),
) -> bool {
    match notify::send_notifications(source, task_info, duration_ms, cwd, true).await {
        Ok(result) => {
            log_cb(format!(
                "[watch][complete:{}] {}",
                source,
                summarize_result(&result)
            ));
            true
        }
        Err(e) => {
            log_cb(format!("[watch][complete:{}] error: {}", source, e));
            false
        }
    }
}

/// Start watching Claude logs
fn start_claude_watch<F>(
    interval_ms: u64,
    _quiet_period_ms: u64,
    claude_quiet_ms: u64,
    log: F,
    confirm_detector: ConfirmDetector,
) -> Result<StopHandle, String>
where
    F: Fn(String) + Send + 'static,
{
    let home_dir = get_home_dir().ok_or_else(|| "Could not determine home directory".to_string())?;
    let root = home_dir.join(".claude").join("projects");

    let state = Arc::new(Mutex::new(ClaudeState::new()));
    let log_arc = Arc::new(Mutex::new(log));
    let confirm_detector = Arc::new(confirm_detector);
    let quiet_ms = std::cmp::max(500, claude_quiet_ms);

    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_clone = stop_flag.clone();

    tokio::spawn(async move {
        let mut int = interval(Duration::from_millis(std::cmp::max(500, interval_ms)));
        let mut follower = JsonlFollower::new(SEED_BYTES);

        loop {
            tokio::select! {
                _ = int.tick() => {
                    if stop_flag_clone.load(Ordering::Relaxed) {
                        break;
                    }

                    let state_clone = state.clone();
                    let log_clone = log_arc.clone();
                    let confirm_clone = confirm_detector.clone();
                    let root_clone = root.clone();

                    tokio::task::block_in_place(|| {
                        let mut state_guard = state_clone.lock().unwrap();
                        let log_guard = log_clone.lock().unwrap();

                        if !root_clone.exists() {
                            return;
                        }

                        let latest = match find_latest_file(&root_clone, |_, name| name.to_lowercase().ends_with(".jsonl")) {
                            Some(p) => p,
                            None => return,
                        };

                        // File changed, reset state
                        if Some(&latest) != state_guard.current_file.as_ref() {
                            state_guard.current_file = Some(latest.clone());
                            state_guard.last_user_text_at = None;
                            state_guard.last_assistant_at = None;
                            state_guard.last_notified_at = None;
                            state_guard.notified_for_turn = false;
                            state_guard.last_confirm_key = String::new();
                            state_guard.confirm_notified_for_turn = false;
                            state_guard.last_user_text = String::new();
                            state_guard.last_assistant_text = String::new();
                            state_guard.last_assistant_content = None;
                            state_guard.last_assistant_had_tool_use = false;

                            // Attach follower to new file
                            follower = JsonlFollower::new(SEED_BYTES);
                            let state_for_callback = state_clone.clone();
                            let confirm_for_callback = confirm_clone.clone();
                            let log_for_callback = log_clone.clone();
                            let quiet = quiet_ms;

                            follower.attach(latest.clone(), move |obj, meta| {
                                process_claude_object(
                                    &obj,
                                    meta.seed,
                                    &state_for_callback,
                                    &confirm_for_callback,
                                    &log_for_callback,
                                    quiet,
                                );
                            });

                            log_guard(format!("[watch][claude] following {}", latest.display()));
                            return;
                        }

                        // Poll for new content
                        let state_for_callback = state_clone.clone();
                        let confirm_for_callback = confirm_clone.clone();
                        let log_for_callback = log_clone.clone();
                        let quiet = quiet_ms;

                        follower.poll(move |obj, meta| {
                            process_claude_object(
                                &obj,
                                meta.seed,
                                &state_for_callback,
                                &confirm_for_callback,
                                &log_for_callback,
                                quiet,
                            );
                        });
                    });
                }
                _ = &mut stop_rx => {
                    break;
                }
            }
        }
    });

    Ok(StopHandle::new(stop_tx))
}

/// Process Claude JSON object
fn process_claude_object(
    obj: &Value,
    is_seed: bool,
    state: &Arc<Mutex<ClaudeState>>,
    confirm_detector: &Arc<ConfirmDetector>,
    log: &Arc<Mutex<dyn Fn(String) + Send>>,
    quiet_ms: u64,
) {
    if !obj.is_object() {
        return;
    }

    if obj.get("isSidechain").and_then(|v| v.as_bool()).unwrap_or(false) {
        return;
    }

    let ts = obj.get("timestamp").and_then(parse_timestamp);
    let obj_type = obj.get("type").and_then(|v| v.as_str());

    let mut state_guard = state.lock().unwrap();
    let log_guard = log.lock().unwrap();

    if let Some(cwd) = obj.get("cwd").and_then(|v| v.as_str()) {
        state_guard.last_cwd = Some(cwd.to_string());
    }

    match obj_type {
        Some("user") => {
            let user_text = extract_message_text(obj.get("message").unwrap_or(&Value::Null));
            state_guard.last_user_text = user_text;
            state_guard.last_assistant_text = String::new();
            state_guard.last_assistant_content = None;
            state_guard.last_assistant_had_tool_use = false;
            state_guard.last_confirm_key = String::new();
            state_guard.confirm_notified_for_turn = false;

            if !is_seed {
                state_guard.last_user_text_at = ts.or_else(|| Some(chrono::Utc::now().timestamp_millis()));
                state_guard.notified_for_turn = false;
            } else if let Some(timestamp) = ts {
                state_guard.last_user_text_at = Some(timestamp);
                state_guard.notified_for_turn = false;
            }
        }
        Some("assistant") => {
            let assistant_text = extract_message_text(obj.get("message").unwrap_or(&Value::Null));
            if !assistant_text.is_empty() {
                state_guard.last_assistant_text = assistant_text.clone();
            }

            let has_tool_use = has_content_type(
                obj.get("message").unwrap_or(&Value::Null),
                "tool_use",
            );
            state_guard.last_assistant_had_tool_use = has_tool_use;

            // Extract content
            let mut content = String::new();
            if let Some(message) = obj.get("message") {
                if let Some(content_arr) = message.get("content").and_then(|c| c.as_array()) {
                    let text_parts: Vec<String> = content_arr
                        .iter()
                        .filter(|item| item.get("type").and_then(|t| t.as_str()) == Some("text"))
                        .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                        .collect();
                    content = text_parts.join("\n\n");
                } else if let Some(content_str) = message.get("content").and_then(|c| c.as_str()) {
                    content = content_str.to_string();
                } else if let Some(text) = message.get("text").and_then(|t| t.as_str()) {
                    content = text.to_string();
                }
            }

            if !content.trim().is_empty() {
                state_guard.last_assistant_content = Some(content);
            }

            let assistant_ts = ts.or_else(|| Some(chrono::Utc::now().timestamp_millis()));
            state_guard.last_assistant_at = assistant_ts;

            if is_seed {
                return;
            }

            // Check for confirm prompt
            if confirm_detector.is_enabled() && !state_guard.confirm_notified_for_turn {
                if let Some(_prompt) = confirm_detector.detect(&assistant_text) {
                    state_guard.confirm_notified_for_turn = true;
                    state_guard.last_confirm_at = chrono::Utc::now().timestamp_millis();

                    // Send confirm notification
                    drop(state_guard);
                    drop(log_guard);

                    let state_clone = state.clone();
                    let log_clone = log.clone();
                    let source = "claude".to_string();

                    tokio::spawn(async move {
                        send_confirm_notification(&source, &|msg| {
                            let _ = log_clone.lock().map(|g| g(msg));
                        })
                        .await;

                        // Reset notified flag so completion can still be sent
                        if let Ok(mut s) = state_clone.lock() {
                            s.confirm_notified_for_turn = false;
                        }
                    });
                    return;
                }
            }

            // Schedule completion notification
            if state_guard.last_user_text_at.is_some()
                && !state_guard.notified_for_turn
                && !state_guard.confirm_notified_for_turn
            {
                state_guard.notified_for_turn = true;
                state_guard.confirm_notified_for_turn = true;

                let assistant_at = state_guard.last_assistant_at.unwrap_or(0);
                let user_at = state_guard.last_user_text_at.unwrap_or(assistant_at);
                let duration_ms = if assistant_at >= user_at {
                    Some(assistant_at - user_at)
                } else {
                    None
                };
                let cwd = state_guard
                    .last_cwd
                    .clone()
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().to_string_lossy().to_string());
                let last_content = state_guard.last_assistant_content.clone();

                drop(state_guard);
                drop(log_guard);

                let log_clone = log.clone();
                let adaptive_quiet = if has_tool_use {
                    quiet_ms
                } else {
                    std::cmp::min(15000, quiet_ms)
                };

                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(adaptive_quiet)).await;

                    send_completion_notification(
                        "claude",
                        "Claude 完成",
                        duration_ms,
                        cwd,
                        &|msg| {
                            let _ = log_clone.lock().map(|g| g(msg));
                        },
                    )
                    .await;
                });
            }
        }
        _ => {}
    }
}

/// Codex turn-end confirmation cues
const CODEX_TURN_END_CONFIRM_CUES: &[&str] = &[
    "请确认",
    "是否继续",
    "是否开始",
    "是否开始执行",
    "是否执行",
    "是否同意",
    "是否允许",
    "是否授权",
    "请选择",
    "请选",
    "你希望",
    "你想",
    "你要",
    "要不要",
    "可以吗",
    "可以么",
    "能否",
    "可否",
    "please confirm",
    "confirm",
    "approve",
    "approval",
    "proceed",
    "continue",
    "should i",
    "shall i",
    "do you want me",
    "would you like",
    "may i",
];

/// Codex state
struct CodexState {
    last_task_started_at: Option<i64>,
    last_user_at: Option<i64>,
    last_assistant_at: Option<i64>,
    last_notified_assistant_at: Option<i64>,
    current_turn_id: String,
    collaboration_mode_kind: String,
    last_notified_turn_id: String,
    last_cwd: Option<String>,
    last_agent_content: Option<String>,
    last_user_text: String,
    last_assistant_text: String,
    last_confirm_key: String,
    last_confirm_at: i64,
    confirm_notified_for_turn: bool,
    interaction_required_for_turn: bool,
    last_interaction_resolved_at: Option<i64>,
}

impl CodexState {
    fn new() -> Self {
        Self {
            last_task_started_at: None,
            last_user_at: None,
            last_assistant_at: None,
            last_notified_assistant_at: None,
            current_turn_id: String::new(),
            collaboration_mode_kind: String::new(),
            last_notified_turn_id: String::new(),
            last_cwd: None,
            last_agent_content: None,
            last_user_text: String::new(),
            last_assistant_text: String::new(),
            last_confirm_key: String::new(),
            last_confirm_at: 0,
            confirm_notified_for_turn: false,
            interaction_required_for_turn: false,
            last_interaction_resolved_at: None,
        }
    }
}

/// Check if text contains turn-end confirmation prompt
fn detect_turn_end_confirm_prompt(text: &str) -> Option<String> {
    let text_lower = text.to_lowercase();
    let lines: Vec<&str> = text.split('\n').collect();
    let tail_lines = lines.iter().rev().take(6).copied().collect::<Vec<_>>();
    let tail_text = tail_lines.join("\n");

    for cue in CODEX_TURN_END_CONFIRM_CUES {
        if tail_text.to_lowercase().contains(&cue.to_lowercase()) {
            return Some(truncate_text(&tail_text, 600));
        }
    }

    // Check for action words + question mark
    let action_words = ["开始", "继续", "执行", "确认", "选择", "proceed", "execute", "run"];
    let last_line = lines.last().map(|l| l.trim()).unwrap_or("");

    if last_line.ends_with('?') || last_line.ends_with('?') {
        for action in action_words {
            if text_lower.contains(&action.to_lowercase()) {
                return Some(truncate_text(&tail_text, 600));
            }
        }
    }

    None
}

/// Start watching Codex logs
fn start_codex_watch<F>(
    interval_ms: u64,
    log: F,
    confirm_detector: ConfirmDetector,
) -> Result<StopHandle, String>
where
    F: Fn(String) + Send + 'static,
{
    let home_dir = get_home_dir().ok_or_else(|| "Could not determine home directory".to_string())?;
    let root = home_dir.join(".codex").join("sessions");

    let state = Arc::new(Mutex::new(CodexState::new()));
    let log_arc = Arc::new(Mutex::new(log));
    let confirm_detector = Arc::new(confirm_detector);

    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_clone = stop_flag.clone();

    tokio::spawn(async move {
        let mut int = interval(Duration::from_millis(std::cmp::max(500, interval_ms)));
        let mut follower = JsonlFollower::new(SEED_BYTES);

        loop {
            tokio::select! {
                _ = int.tick() => {
                    if stop_flag_clone.load(Ordering::Relaxed) {
                        break;
                    }

                    let state_clone = state.clone();
                    let log_clone = log_arc.clone();
                    let confirm_clone = confirm_detector.clone();
                    let root_clone = root.clone();

                    tokio::task::block_in_place(|| {
                        let mut state_guard = state_clone.lock().unwrap();
                        let log_guard = log_clone.lock().unwrap();

                        if !root_clone.exists() {
                            return;
                        }

                        let latest = match find_latest_file(&root_clone, |_, name| name.to_lowercase().ends_with(".jsonl")) {
                            Some(p) => p,
                            None => return,
                        };

                        // File changed, reset state
                        if follower.file_path().map(|p| p != &latest).unwrap_or(true) {
                            state_guard.current_turn_id = String::new();
                            state_guard.collaboration_mode_kind = String::new();
                            state_guard.last_notified_turn_id = String::new();
                            state_guard.last_cwd = None;
                            state_guard.last_agent_content = None;
                            state_guard.last_user_text = String::new();
                            state_guard.last_assistant_text = String::new();
                            state_guard.last_confirm_key = String::new();
                            state_guard.confirm_notified_for_turn = false;
                            state_guard.interaction_required_for_turn = false;
                            state_guard.last_interaction_resolved_at = None;

                            // Attach follower to new file
                            follower = JsonlFollower::new(SEED_BYTES);
                            let state_for_callback = state_clone.clone();
                            let confirm_for_callback = confirm_clone.clone();
                            let log_for_callback = log_clone.clone();

                            follower.attach(latest.clone(), move |obj, meta| {
                                process_codex_object(
                                    &obj,
                                    meta.seed,
                                    &state_for_callback,
                                    &confirm_for_callback,
                                    &log_for_callback,
                                );
                            });

                            log_guard(format!("[watch][codex] following {}", latest.display()));
                            return;
                        }

                        // Poll for new content
                        let state_for_callback = state_clone.clone();
                        let confirm_for_callback = confirm_clone.clone();
                        let log_for_callback = log_clone.clone();

                        follower.poll(move |obj, meta| {
                            process_codex_object(
                                &obj,
                                meta.seed,
                                &state_for_callback,
                                &confirm_for_callback,
                                &log_for_callback,
                            );
                        });
                    });
                }
                _ = &mut stop_rx => {
                    break;
                }
            }
        }
    });

    Ok(StopHandle::new(stop_tx))
}

/// Process Codex JSON object
fn process_codex_object(
    obj: &Value,
    is_seed: bool,
    state: &Arc<Mutex<CodexState>>,
    confirm_detector: &Arc<ConfirmDetector>,
    log: &Arc<Mutex<dyn Fn(String) + Send>>,
) {
    if !obj.is_object() {
        return;
    }

    let ts = obj.get("timestamp").and_then(parse_timestamp);
    let obj_type = obj.get("type").and_then(|v| v.as_str());

    let mut state_guard = state.lock().unwrap();

    // Handle turn_context
    if obj_type == Some("turn_context") {
        if let Some(payload) = obj.get("payload") {
            if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
                state_guard.last_cwd = Some(cwd.to_string());
            }
            if let Some(turn_id) = payload.get("turn_id").and_then(|v| v.as_str()) {
                if state_guard.current_turn_id != turn_id {
                    state_guard.current_turn_id = turn_id.to_string();
                    state_guard.last_confirm_key = String::new();
                    state_guard.confirm_notified_for_turn = false;
                    state_guard.interaction_required_for_turn = false;
                    state_guard.last_interaction_resolved_at = None;
                }
            }
            if let Some(mode) = payload.get("collaboration_mode").and_then(|m| m.get("mode")).and_then(|v| v.as_str()) {
                state_guard.collaboration_mode_kind = mode.to_string();
            }
        }
        return;
    }

    // Handle user message
    if obj_type == Some("response_item") {
        if let Some(payload) = obj.get("payload") {
            if payload.get("type").and_then(|v| v.as_str()) == Some("message")
                && payload.get("role").and_then(|v| v.as_str()) == Some("user")
            {
                if !is_seed {
                    state_guard.last_task_started_at = None;
                }
                state_guard.last_user_at = ts;
                state_guard.last_user_text = extract_text_from_any(payload);
                state_guard.last_confirm_key = String::new();
                state_guard.confirm_notified_for_turn = false;
                state_guard.interaction_required_for_turn = false;
                return;
            }

            // Handle assistant message
            if payload.get("type").and_then(|v| v.as_str()) == Some("message")
                && payload.get("role").and_then(|v| v.as_str()) == Some("assistant")
            {
                if !is_seed {
                    let assistant_text = extract_text_from_any(payload);
                    if !assistant_text.is_empty() {
                        state_guard.last_assistant_text = assistant_text.clone();
                        state_guard.last_agent_content = Some(assistant_text);
                    }
                    state_guard.last_assistant_at = ts.or_else(|| Some(chrono::Utc::now().timestamp_millis()));
                }
                return;
            }
        }
    }

    // Handle event_msg
    if obj_type == Some("event_msg") {
        if let Some(payload) = obj.get("payload") {
            if let Some(kind) = payload.get("type").and_then(|v| v.as_str()) {
                match kind {
                    "task_started" => {
                        if !is_seed {
                            state_guard.last_task_started_at = None;
                        }
                        if let Some(turn_id) = payload.get("turn_id").and_then(|v| v.as_str()) {
                            state_guard.current_turn_id = turn_id.to_string();
                        }
                        if let Some(mode) = payload.get("collaboration_mode_kind").and_then(|v| v.as_str()) {
                            state_guard.collaboration_mode_kind = mode.to_string();
                        }
                        state_guard.last_task_started_at = ts;
                        state_guard.last_confirm_key = String::new();
                        state_guard.confirm_notified_for_turn = false;
                        state_guard.interaction_required_for_turn = false;
                        return;
                    }
                    "task_complete" => {
                        if is_seed {
                            return;
                        }

                        let turn_id = payload.get("turn_id").and_then(|v| v.as_str()).unwrap_or("");
                        if state_guard.last_notified_turn_id == turn_id {
                            return;
                        }

                        let completion_at = ts.or_else(|| Some(chrono::Utc::now().timestamp_millis())).unwrap();

                        if let Some(last_msg) = payload.get("last_agent_message").and_then(|v| v.as_str()) {
                            if !last_msg.is_empty() {
                                state_guard.last_assistant_text = last_msg.to_string();
                                state_guard.last_agent_content = Some(last_msg.to_string());
                                state_guard.last_assistant_at = Some(completion_at);
                            }
                        }

                        if state_guard.confirm_notified_for_turn {
                            let _ = log.lock().map(|g| g(format!("[watch][codex] skipped completion (already notified)")));
                            return;
                        }

                        // Check for confirm prompt
                        if confirm_detector.is_enabled() {
                            if let Some(content) = &state_guard.last_agent_content {
                                if let Some(prompt) = detect_turn_end_confirm_prompt(content) {
                                    state_guard.confirm_notified_for_turn = true;
                                    state_guard.last_confirm_at = chrono::Utc::now().timestamp_millis();

                                    drop(state_guard);

                                    let log_clone = log.clone();
                                    tokio::spawn(async move {
                                        send_confirm_notification("codex", &|msg| {
                                            let _ = log_clone.lock().map(|g| g(msg));
                                        })
                                        .await;
                                    });
                                    return;
                                }
                            }
                        }

                        // Send completion notification
                        state_guard.last_notified_assistant_at = Some(completion_at);
                        state_guard.last_notified_turn_id = turn_id.to_string();
                        state_guard.confirm_notified_for_turn = true;

                        let start_at = state_guard.last_user_at.or(state_guard.last_task_started_at);
                        let duration_ms = start_at.map(|s| completion_at - s).filter(|d| *d >= 0);
                        let cwd = state_guard
                            .last_cwd
                            .clone()
                            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default().to_string_lossy().to_string());

                        drop(state_guard);

                        let log_clone = log.clone();
                        tokio::spawn(async move {
                            send_completion_notification(
                                "codex",
                                "Codex 完成",
                                duration_ms,
                                cwd,
                                &|msg| {
                                    let _ = log_clone.lock().map(|g| g(msg));
                                },
                            )
                            .await;
                        });
                        return;
                    }
                    "agent_message" => {
                        if !is_seed {
                            if let Some(text) = payload.get("content").and_then(|v| v.as_str()) {
                                state_guard.last_assistant_text = text.to_string();
                            }
                            state_guard.last_assistant_at = ts.or_else(|| Some(chrono::Utc::now().timestamp_millis()));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Gemini state
struct GeminiState {
    current_file: Option<PathBuf>,
    current_mtime_ms: i64,
    last_count: usize,
    last_user_at: Option<i64>,
    last_gemini_at: Option<i64>,
    last_notified_gemini_at: Option<i64>,
    last_gemini_content: Option<String>,
    last_user_text: String,
    last_gemini_text: String,
    last_confirm_key: String,
    last_confirm_at: i64,
    confirm_notified_for_turn: bool,
}

impl GeminiState {
    fn new() -> Self {
        Self {
            current_file: None,
            current_mtime_ms: 0,
            last_count: 0,
            last_user_at: None,
            last_gemini_at: None,
            last_notified_gemini_at: None,
            last_gemini_content: None,
            last_user_text: String::new(),
            last_gemini_text: String::new(),
            last_confirm_key: String::new(),
            last_confirm_at: 0,
            confirm_notified_for_turn: false,
        }
    }
}

/// Start watching Gemini logs
fn start_gemini_watch<F>(
    interval_ms: u64,
    quiet_period_ms: u64,
    log: F,
    confirm_detector: ConfirmDetector,
) -> Result<StopHandle, String>
where
    F: Fn(String) + Send + 'static,
{
    let home_dir = get_home_dir().ok_or_else(|| "Could not determine home directory".to_string())?;
    let root = home_dir.join(".gemini").join("tmp");

    let state = Arc::new(Mutex::new(GeminiState::new()));
    let log_arc = Arc::new(Mutex::new(log));
    let confirm_detector = Arc::new(confirm_detector);
    let quiet_ms = std::cmp::max(500, quiet_period_ms);

    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_clone = stop_flag.clone();

    tokio::spawn(async move {
        let mut int = interval(Duration::from_millis(std::cmp::max(500, interval_ms)));

        loop {
            tokio::select! {
                _ = int.tick() => {
                    if stop_flag_clone.load(Ordering::Relaxed) {
                        break;
                    }

                    let state_clone = state.clone();
                    let log_clone = log_arc.clone();
                    let confirm_clone = confirm_detector.clone();
                    let root_clone = root.clone();
                    let quiet = quiet_ms;

                    tokio::task::block_in_place(|| {
                        let mut state_guard = state_clone.lock().unwrap();
                        let log_guard = log_clone.lock().unwrap();

                        if !root_clone.exists() {
                            return;
                        }

                        let latest = match find_latest_file(&root_clone, |path, name| {
                            name.to_lowercase().ends_with(".json")
                                && name.to_lowercase().starts_with("session-")
                                && path.to_string_lossy().contains("/chats/")
                        }) {
                            Some(p) => p,
                            None => return,
                        };

                        let stat = match safe_stat(&latest) {
                            Some(s) => s,
                            None => return,
                        };

                        // New file
                        if state_guard.current_file.as_ref() != Some(&latest) {
                            // Read and parse
                            if let Ok(content) = fs::read_to_string(&latest) {
                                let content = content.trim_start_matches('\u{feff}');
                                if let Ok(parsed) = serde_json::from_str::<Value>(content) {
                                    if let Some(messages) = parsed.get("messages").and_then(|m| m.as_array()) {
                                        // Reset state
                                        state_guard.last_user_at = None;
                                        state_guard.last_gemini_at = None;
                                        state_guard.last_user_text = String::new();
                                        state_guard.last_gemini_text = String::new();
                                        state_guard.last_confirm_key = String::new();
                                        state_guard.confirm_notified_for_turn = false;

                                        // Process existing messages
                                        for msg in messages {
                                            if let Some(ts) = msg.get("timestamp").and_then(parse_timestamp) {
                                                if let Some(msg_type) = msg.get("type").and_then(|t| t.as_str()) {
                                                    match msg_type {
                                                        "user" => {
                                                            state_guard.last_user_at = Some(ts);
                                                            state_guard.last_user_text = extract_message_text(msg);
                                                        }
                                                        "gemini" => {
                                                            state_guard.last_gemini_at = Some(ts);
                                                            state_guard.last_gemini_text = extract_message_text(msg);
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        }

                                        state_guard.last_count = messages.len();
                                        state_guard.current_file = Some(latest.clone());
                                        state_guard.current_mtime_ms = stat.mtime_ms;
                                        state_guard.last_notified_gemini_at = state_guard.last_gemini_at;

                                        log_guard(format!("[watch][gemini] following {}", latest.display()));
                                        return;
                                    }
                                }
                            }
                            return;
                        }

                        // No change
                        if stat.mtime_ms <= state_guard.current_mtime_ms {
                            return;
                        }

                        // File changed, re-read
                        let content = match fs::read_to_string(&latest) {
                            Ok(c) => c.trim_start_matches('\u{feff}').to_string(),
                            Err(_) => return,
                        };

                        let parsed: Value = match serde_json::from_str(&content) {
                            Ok(p) => p,
                            Err(_) => return,
                        };

                        let messages = parsed
                            .get("messages")
                            .and_then(|m| m.as_array())
                            .cloned()
                            .unwrap_or_default();

                        if messages.len() <= state_guard.last_count {
                            state_guard.current_mtime_ms = stat.mtime_ms;
                            state_guard.last_count = messages.len();
                            return;
                        }

                        // Process new messages
                        let new_messages = messages.into_iter().skip(state_guard.last_count);
                        let state_for_callback = state_clone.clone();
                        let confirm_for_callback = confirm_clone.clone();
                        let log_for_callback = log_clone.clone();
                        let quiet_inner = quiet;

                        for msg in new_messages {
                            process_gemini_message(
                                &msg,
                                &state_for_callback,
                                &confirm_for_callback,
                                &log_for_callback,
                                quiet_inner,
                            );
                        }

                        state_guard.current_mtime_ms = stat.mtime_ms;
                        state_guard.last_count = messages.len();
                    });
                }
                _ = &mut stop_rx => {
                    break;
                }
            }
        }
    });

    Ok(StopHandle::new(stop_tx))
}

/// Process Gemini message
fn process_gemini_message(
    msg: &Value,
    state: &Arc<Mutex<GeminiState>>,
    confirm_detector: &Arc<ConfirmDetector>,
    log: &Arc<Mutex<dyn Fn(String) + Send>>,
    quiet_ms: u64,
) {
    let ts = msg.get("timestamp").and_then(parse_timestamp);
    let msg_type = msg.get("type").and_then(|v| v.as_str());

    let mut state_guard = state.lock().unwrap();

    match msg_type {
        Some("user") => {
            state_guard.last_user_at = ts;
            state_guard.last_user_text = extract_message_text(msg);
            state_guard.last_gemini_at = None;
            state_guard.last_notified_gemini_at = None;
            state_guard.last_gemini_text = String::new();
            state_guard.last_confirm_key = String::new();
            state_guard.confirm_notified_for_turn = false;
        }
        Some("gemini") => {
            state_guard.last_gemini_at = ts;

            // Extract content
            let mut content_text = String::new();

            if let Some(content) = msg.get("content") {
                if let Some(arr) = content.as_array() {
                    let parts: Vec<String> = arr.iter().filter_map(|i| i.as_str()).collect();
                    content_text = parts.join("\n\n");
                } else if let Some(text) = content.as_str() {
                    content_text = text.to_string();
                }
            }

            if let Some(parts) = msg.get("parts").and_then(|p| p.as_array()) {
                let text_parts: Vec<String> = parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect();
                if content_text.is_empty() {
                    content_text = text_parts.join("\n\n");
                }
            }

            if let Some(text) = msg.get("text").and_then(|t| t.as_str()) {
                if content_text.is_empty() {
                    content_text = text.to_string();
                }
            }

            if !content_text.trim().is_empty() {
                state_guard.last_gemini_content = Some(content_text);
            }

            let gemini_text = extract_message_text(msg);
            if !gemini_text.is_empty() {
                state_guard.last_gemini_text = gemini_text.clone();
            }

            // Check for confirm
            if confirm_detector.is_enabled() && !state_guard.confirm_notified_for_turn {
                if let Some(_prompt) = confirm_detector.detect(&gemini_text) {
                    state_guard.confirm_notified_for_turn = true;
                    state_guard.last_confirm_at = chrono::Utc::now().timestamp_millis();

                    drop(state_guard);

                    let log_clone = log.clone();
                    tokio::spawn(async move {
                        send_confirm_notification("gemini", &|msg| {
                            let _ = log_clone.lock().map(|g| g(msg));
                        })
                        .await;
                    });
                    return;
                }
            }

            // Schedule debounced completion notification
            if !state_guard.confirm_notified_for_turn {
                let target_at = state_guard.last_gemini_at;
                let content = state_guard.last_gemini_content.clone();
                let cwd = std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                drop(state_guard);

                let log_clone = log.clone();
                let state_clone = state.clone();

                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(quiet_ms)).await;

                    let mut guard = state_clone.lock().unwrap();
                    if guard.last_gemini_at == target_at && guard.last_notified_gemini_at != target_at {
                        guard.last_notified_gemini_at = target_at;
                        guard.confirm_notified_for_turn = true;

                        let start_at = guard.last_user_at;
                        let duration_ms = start_at.and_then(|s| target_at.map(|t| t - s)).filter(|d| *d >= 0);

                        drop(guard);

                        send_completion_notification(
                            "gemini",
                            "Gemini 完成",
                            duration_ms,
                            cwd,
                            &|msg| {
                                let _ = log_clone.lock().map(|g| g(msg));
                            },
                        )
                        .await;
                    }
                });
            }
        }
        _ => {}
    }
}

/// Normalize sources string
fn normalize_sources(input: &str) -> Vec<String> {
    if input.is_empty() {
        return vec!["claude".to_string(), "codex".to_string(), "gemini".to_string()];
    }

    let parts: Vec<&str> = input
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    if parts.contains(&"all".to_string()) {
        return vec!["claude".to_string(), "codex".to_string(), "gemini".to_string()];
    }

    let mut seen = HashSet::new();
    parts
        .into_iter()
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

/// Start watching AI CLI logs
///
/// # Arguments
/// * `sources` - Comma-separated list of sources to watch (claude, codex, gemini, or all)
/// * `interval_ms` - Polling interval in milliseconds
/// * `gemini_quiet_ms` - Debounce time for Gemini notifications
/// * `claude_quiet_ms` - Debounce time for Claude notifications
/// * `on_log` - Callback for log messages
///
/// # Returns
/// * `Ok(Box<dyn FnOnce() + Send>)` - Function to stop watching
/// * `Err(String)` - Error message
pub fn start_watch<F>(
    sources: &str,
    interval_ms: u64,
    gemini_quiet_ms: u64,
    claude_quiet_ms: u64,
    on_log: F,
) -> Result<Box<dyn FnOnce() + Send>, String>
where
    F: Fn(String) + Send + 'static,
{
    let normalized = normalize_sources(sources);

    // Load confirm alert config
    let confirm_config = crate::config::load_config()
        .map(|c| c.ui.confirm_alert)
        .unwrap_or(ConfirmAlertConfig { enabled: false });

    let confirm_detector = ConfirmDetector::new(confirm_config.enabled);

    let mut stop_handles: Vec<StopHandle> = Vec::new();

    if normalized.contains(&"claude".to_string()) {
        match start_claude_watch(
            interval_ms,
            gemini_quiet_ms,
            claude_quiet_ms,
            on_log.clone(),
            confirm_detector.clone(),
        ) {
            Ok(handle) => stop_handles.push(handle),
            Err(e) => on_log(format!("[watch] failed to start claude watch: {}", e)),
        }
    }

    if normalized.contains(&"codex".to_string()) {
        match start_codex_watch(interval_ms, on_log.clone(), confirm_detector.clone()) {
            Ok(handle) => stop_handles.push(handle),
            Err(e) => on_log(format!("[watch] failed to start codex watch: {}", e)),
        }
    }

    if normalized.contains(&"gemini".to_string()) {
        match start_gemini_watch(
            interval_ms,
            gemini_quiet_ms,
            on_log.clone(),
            confirm_detector,
        ) {
            Ok(handle) => stop_handles.push(handle),
            Err(e) => on_log(format!("[watch] failed to start gemini watch: {}", e)),
        }
    }

    // Return combined stop function
    let stop_function = move || {
        for mut handle in stop_handles {
            handle.stop();
        }
    };

    Ok(Box::new(stop_function))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_timestamp_number() {
        // Unix seconds
        let val = Value::Number(serde_json::Number::from(1700000000));
        assert_eq!(parse_timestamp(&val), Some(1700000000000));

        // Unix milliseconds
        let val = Value::Number(serde_json::Number::from(1700000000000i64));
        assert_eq!(parse_timestamp(&val), Some(1700000000000));
    }

    #[test]
    fn test_parse_timestamp_string() {
        // Numeric string
        let val = Value::String("1700000000".to_string());
        assert_eq!(parse_timestamp(&val), Some(1700000000000));

        // ISO 8601
        let val = Value::String("2024-01-01T00:00:00Z".to_string());
        assert!(parse_timestamp(&val).is_some());
    }

    #[test]
    fn test_extract_message_text() {
        // Simple text
        let msg = json!({"content": "Hello world"});
        assert_eq!(extract_message_text(&msg), "Hello world");

        // Content array
        let msg = json!({
            "content": [
                {"type": "text", "text": "Hello"}
            ]
        });
        assert_eq!(extract_message_text(&msg), "Hello");
    }

    #[test]
    fn test_confirm_detector() {
        let detector = ConfirmDetector::new(true);

        // Chinese keyword
        assert!(detector.detect("是否继续？").is_some());

        // English keyword
        assert!(detector.detect("Please confirm").is_some());

        // No keyword
        assert!(detector.detect("Task completed").is_none());

        // Disabled detector
        let disabled = ConfirmDetector::new(false);
        assert!(disabled.detect("是否继续？").is_none());
    }

    #[test]
    fn test_normalize_sources() {
        assert_eq!(
            normalize_sources(""),
            vec!["claude", "codex", "gemini"]
        );

        assert_eq!(
            normalize_sources("all"),
            vec!["claude", "codex", "gemini"]
        );

        assert_eq!(
            normalize_sources("claude,gemini"),
            vec!["claude", "gemini"]
        );

        assert_eq!(
            normalize_sources("claude, claude , gemini"),
            vec!["claude", "gemini"]
        );
    }
}
