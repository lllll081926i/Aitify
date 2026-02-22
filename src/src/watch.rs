use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use tokio::time::interval;

const CLAUDE_DIR: &str = ".claude/projects";
const CODEX_DIR: &str = ".codex/sessions";
const GEMINI_DIR: &str = ".gemini/tmp/chats";

struct FileState {
    position: u64,
    last_user_at: Option<i64>,
    last_assistant_at: Option<i64>,
    last_notified_at: Option<i64>,
    notified_for_turn: bool,
    last_notified_turn_id: Option<Box<str>>,
}

fn get_home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE").ok().map(PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}

fn find_latest_jsonl(dir: &Path) -> Option<PathBuf> {
    fn walk_dir(dir: &Path, latest: &mut Option<(PathBuf, SystemTime)>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk_dir(&path, latest);
                } else if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                    if let Ok(metadata) = entry.metadata() {
                        if let Ok(modified) = metadata.modified() {
                            if latest.is_none() || modified > latest.as_ref().unwrap().1 {
                                *latest = Some((path, modified));
                            }
                        }
                    }
                }
            }
        }
    }

    let mut latest: Option<(PathBuf, SystemTime)> = None;
    walk_dir(dir, &mut latest);
    latest.map(|(path, _)| path)
}

fn find_latest_json(dir: &Path, prefix: &str) -> Option<PathBuf> {
    fn walk_dir(dir: &Path, prefix: &str, latest: &mut Option<(PathBuf, SystemTime)>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk_dir(&path, prefix, latest);
                } else if path.is_file() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.to_lowercase().starts_with(prefix) && path.extension().and_then(|s| s.to_str()) == Some("json") {
                            if let Ok(metadata) = entry.metadata() {
                                if let Ok(modified) = metadata.modified() {
                                    if latest.is_none() || modified > latest.as_ref().unwrap().1 {
                                        *latest = Some((path, modified));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut latest: Option<(PathBuf, SystemTime)> = None;
    walk_dir(dir, prefix, &mut latest);
    latest.map(|(path, _)| path)
}

fn parse_json_line(line: &str) -> Option<Value> {
    serde_json::from_str(line.trim()).ok()
}

fn parse_timestamp(value: &Value) -> Option<i64> {
    if let Some(s) = value.as_str() {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
            return Some(dt.timestamp_millis());
        }
    }
    value.as_i64()
}

fn read_new_content(file_path: &Path, position: u64) -> std::io::Result<(String, u64)> {
    let mut file = File::open(file_path)?;
    let metadata = file.metadata()?;
    let file_size = metadata.len();

    if file_size < position {
        return Ok((String::new(), 0));
    }

    if file_size == position {
        return Ok((String::new(), position));
    }

    file.seek(SeekFrom::Start(position))?;
    let to_read = (file_size - position).min(1024 * 1024) as usize; // 最多读1MB
    let mut content = String::with_capacity(to_read);
    file.read_to_string(&mut content)?;

    Ok((content, file_size))
}

async fn process_claude_file<F>(
    latest_file: &PathBuf,
    states: &Arc<Mutex<HashMap<PathBuf, FileState>>>,
    pending_timers: &Arc<Mutex<HashMap<PathBuf, tokio::task::JoinHandle<()>>>>,
    quiet_ms: u64,
    log_callback: &mut F,
) where
    F: FnMut(String) + Send + 'static,
{
    let mut states_lock = states.lock().unwrap();
    let state = states_lock.entry(latest_file.clone()).or_insert_with(|| {
        let size = std::fs::metadata(latest_file).map(|m| m.len()).unwrap_or(0);
        log_callback(format!("[watch][claude] following {:?}", latest_file));
        FileState {
            position: size,
            last_user_at: None,
            last_assistant_at: None,
            last_notified_at: None,
            notified_for_turn: false,
            last_notified_turn_id: None,
        }
    });

    let (new_content, new_position) = match read_new_content(latest_file, state.position) {
        Ok(result) => result,
        Err(_) => {
            drop(states_lock);
            return;
        }
    };

    if new_content.is_empty() {
        drop(states_lock);
        return;
    }

    state.position = new_position;

    for line in new_content.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let obj = match parse_json_line(line) {
            Some(o) => o,
            None => continue,
        };

        if obj.get("isSidechain").and_then(|v| v.as_bool()) == Some(true) {
            continue;
        }

        let ts = obj.get("timestamp").and_then(|v| parse_timestamp(v));

        if obj.get("type").and_then(|v| v.as_str()) == Some("user") {
            state.last_user_at = ts;
            state.notified_for_turn = false;

            let mut timers = pending_timers.lock().unwrap();
            if let Some(timer) = timers.remove(latest_file) {
                timer.abort();
            }
            continue;
        }

        if obj.get("type").and_then(|v| v.as_str()) == Some("assistant") {
            if let Some(msg) = obj.get("message").and_then(|v| v.as_object()) {
                state.last_assistant_at = ts;

                if state.last_user_at.is_none() {
                    state.last_user_at = ts;
                    state.notified_for_turn = false;
                }

                let has_tool_use = msg
                    .get("content")
                    .and_then(|c| c.as_array())
                    .map(|arr| arr.iter().any(|item| {
                        item.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                    }))
                    .unwrap_or(false);

                let adaptive_quiet_ms = if has_tool_use { quiet_ms } else { quiet_ms.min(15000) };

                let mut timers = pending_timers.lock().unwrap();
                if let Some(timer) = timers.remove(latest_file) {
                    timer.abort();
                }

                let file_path = latest_file.clone();
                let states_clone = Arc::clone(&states);
                let timer = tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(adaptive_quiet_ms)).await;

                    let (should_notify, duration_ms) = {
                        let states = states_clone.lock().unwrap();
                        if let Some(state) = states.get(&file_path) {
                            if state.notified_for_turn || state.last_notified_at == state.last_assistant_at {
                                return;
                            }

                            let duration = if let (Some(start), Some(end)) = (state.last_user_at, state.last_assistant_at) {
                                if end >= start { Some(end - start) } else { None }
                            } else {
                                None
                            };

                            (true, duration)
                        } else {
                            return;
                        }
                    };

                    if should_notify {
                        if let Err(e) = crate::notify::send_notifications(
                            "claude",
                            "Claude 完成",
                            duration_ms,
                            String::new(),
                            false,
                        ).await {
                            eprintln!("Notification error: {}", e);
                        } else {
                            let mut states = states_clone.lock().unwrap();
                            if let Some(state) = states.get_mut(&file_path) {
                                state.last_notified_at = state.last_assistant_at;
                                state.notified_for_turn = true;
                            }
                        }
                    }
                });

                timers.insert(latest_file.clone(), timer);
            }
        }
    }

    drop(states_lock);
}

async fn process_codex_file<F>(
    latest_file: &PathBuf,
    states: &Arc<Mutex<HashMap<PathBuf, FileState>>>,
    pending_timers: &Arc<Mutex<HashMap<PathBuf, tokio::task::JoinHandle<()>>>>,
    log_callback: &mut F,
) where
    F: FnMut(String) + Send + 'static,
{
    let (new_content, _new_position) = {
        let mut states_lock = states.lock().unwrap();
        let state = states_lock.entry(latest_file.clone()).or_insert_with(|| {
            let size = std::fs::metadata(latest_file).map(|m| m.len()).unwrap_or(0);
            log_callback(format!("[watch][codex] following {:?}", latest_file));
            FileState {
                position: size,
                last_user_at: None,
                last_assistant_at: None,
                last_notified_at: None,
                notified_for_turn: false,
                last_notified_turn_id: None,
            }
        });

        let result = match read_new_content(latest_file, state.position) {
            Ok(result) => result,
            Err(_) => return,
        };

        if result.0.is_empty() {
            return;
        }

        state.position = result.1;
        result
    };

    for line in new_content.lines() {
        if line.trim().is_empty() {
            continue;
        }

        let obj = match parse_json_line(line) {
            Some(o) => o,
            None => continue,
        };

        let ts = obj.get("timestamp").and_then(|v| parse_timestamp(v));

        if obj.get("type").and_then(|v| v.as_str()) == Some("response_item") {
            if let Some(payload) = obj.get("payload").and_then(|v| v.as_object()) {
                if payload.get("type").and_then(|v| v.as_str()) == Some("message")
                    && payload.get("role").and_then(|v| v.as_str()) == Some("user") {
                    let mut states_lock = states.lock().unwrap();
                    if let Some(state) = states_lock.get_mut(latest_file) {
                        state.last_user_at = ts;
                        state.notified_for_turn = false;
                    }
                    drop(states_lock);

                    let mut timers = pending_timers.lock().unwrap();
                    if let Some(timer) = timers.remove(latest_file) {
                        timer.abort();
                    }
                    continue;
                }
            }
        }

        if obj.get("type").and_then(|v| v.as_str()) == Some("event_msg") {
            if let Some(payload) = obj.get("payload").and_then(|v| v.as_object()) {
                if payload.get("type").and_then(|v| v.as_str()) == Some("task_complete") {
                    let turn_id: Option<Box<str>> = payload.get("turn_id").and_then(|v| v.as_str()).map(|s| s.into());

                    let should_skip = {
                        let states_lock = states.lock().unwrap();
                        if let Some(state) = states_lock.get(latest_file) {
                            if let Some(ref tid) = turn_id {
                                state.last_notified_turn_id.as_deref() == Some(tid.as_ref())
                            } else {
                                false
                            }
                        } else {
                            true
                        }
                    };

                    if should_skip {
                        continue;
                    }

                    {
                        let mut timers = pending_timers.lock().unwrap();
                        if let Some(timer) = timers.remove(latest_file) {
                            timer.abort();
                        }
                    }

                    let completion_at = ts.unwrap_or_else(|| chrono::Utc::now().timestamp_millis());

                    let duration_ms = {
                        let mut states_lock = states.lock().unwrap();
                        if let Some(state) = states_lock.get_mut(latest_file) {
                            state.last_assistant_at = Some(completion_at);

                            if let Some(start) = state.last_user_at {
                                if completion_at >= start {
                                    Some(completion_at - start)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    };

                    if let Err(e) = crate::notify::send_notifications(
                        "codex",
                        "Codex 完成",
                        duration_ms,
                        String::new(),
                        false,
                    ).await {
                        eprintln!("Notification error: {}", e);
                    } else {
                        let mut states_lock = states.lock().unwrap();
                        if let Some(state) = states_lock.get_mut(latest_file) {
                            state.last_notified_at = Some(completion_at);
                            state.notified_for_turn = true;
                            state.last_notified_turn_id = turn_id;
                        }
                    }

                    return;
                }
            }
        }
    }
}


pub fn start_watch<F>(
    _sources: &str,
    interval_ms: i32,
    gemini_quiet_ms: i32,
    claude_quiet_ms: i32,
    mut log_callback: F,
) -> Result<Box<dyn FnOnce() + Send>, Box<dyn std::error::Error>>
where
    F: FnMut(String) + Send + 'static,
{
    let home = get_home_dir().ok_or("Cannot find home directory")?;
    let claude_root = home.join(CLAUDE_DIR);
    let codex_root = home.join(CODEX_DIR);
    let gemini_root = home.join(GEMINI_DIR);

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    let quiet_ms = claude_quiet_ms.max(500) as u64;
    let gemini_quiet = gemini_quiet_ms.max(500) as u64;

    tokio::spawn(async move {
        let states: Arc<Mutex<HashMap<PathBuf, FileState>>> = Arc::new(Mutex::new(HashMap::with_capacity(3)));
        let pending_timers: Arc<Mutex<HashMap<PathBuf, tokio::task::JoinHandle<()>>>> = Arc::new(Mutex::new(HashMap::with_capacity(3)));
        let mut tick_interval = interval(Duration::from_millis(interval_ms.max(500) as u64));
        let mut cleanup_counter = 0u32;

        while running_clone.load(Ordering::Relaxed) {
            tick_interval.tick().await;

            let mut current_files = Vec::with_capacity(3);

            // Monitor Claude
            if claude_root.exists() {
                if let Some(latest_file) = find_latest_jsonl(&claude_root) {
                    current_files.push(latest_file.clone());
                    process_claude_file(&latest_file, &states, &pending_timers, quiet_ms, &mut log_callback).await;
                }
            }

            // Monitor Codex
            if codex_root.exists() {
                if let Some(latest_file) = find_latest_jsonl(&codex_root) {
                    current_files.push(latest_file.clone());
                    process_codex_file(&latest_file, &states, &pending_timers, &mut log_callback).await;
                }
            }

            // Monitor Gemini
            if gemini_root.exists() {
                if let Some(latest_file) = find_latest_json(&gemini_root, "session-") {
                    current_files.push(latest_file.clone());
                    process_gemini_file(&latest_file, &states, &pending_timers, gemini_quiet, &mut log_callback).await;
                }
            }

            // 每60次循环清理一次旧状态 (约1分钟)
            cleanup_counter += 1;
            if cleanup_counter >= 60 {
                cleanup_counter = 0;
                let mut states_lock = states.lock().unwrap();
                states_lock.retain(|path, _| current_files.contains(path));
            }
        }
    });

    Ok(Box::new(move || {
        running.store(false, Ordering::Relaxed);
    }))
}

async fn process_gemini_file<F>(
    latest_file: &PathBuf,
    states: &Arc<Mutex<HashMap<PathBuf, FileState>>>,
    pending_timers: &Arc<Mutex<HashMap<PathBuf, tokio::task::JoinHandle<()>>>>,
    quiet_ms: u64,
    log_callback: &mut F,
) where
    F: FnMut(String) + Send + 'static,
{
    let metadata = match std::fs::metadata(latest_file) {
        Ok(m) => m,
        Err(_) => return,
    };

    let modified = match metadata.modified() {
        Ok(m) => m,
        Err(_) => return,
    };

    let mut states_lock = states.lock().unwrap();
    let state = states_lock.entry(latest_file.clone()).or_insert_with(|| {
        log_callback(format!("[watch][gemini] following {:?}", latest_file));
        FileState {
            position: 0,
            last_user_at: None,
            last_assistant_at: None,
            last_notified_at: None,
            notified_for_turn: false,
            last_notified_turn_id: None,
        }
    });

    let last_modified_ms = modified.duration_since(SystemTime::UNIX_EPOCH).ok().map(|d| d.as_millis() as u64).unwrap_or(0);

    if last_modified_ms <= state.position {
        drop(states_lock);
        return;
    }

    let content = match std::fs::read_to_string(latest_file) {
        Ok(c) => c,
        Err(_) => {
            drop(states_lock);
            return;
        }
    };

    let json: Value = match serde_json::from_str(&content) {
        Ok(j) => j,
        Err(_) => {
            drop(states_lock);
            return;
        }
    };

    let messages = match json.get("messages").and_then(|m| m.as_array()) {
        Some(m) => m,
        None => {
            drop(states_lock);
            return;
        }
    };

    let last_count = state.position as usize;
    if messages.len() <= last_count {
        state.position = last_modified_ms;
        drop(states_lock);
        return;
    }

    let new_messages = &messages[last_count..];

    for msg in new_messages {
        let ts = msg.get("timestamp").and_then(|v| parse_timestamp(v));
        let msg_type = msg.get("type").and_then(|v| v.as_str());

        if msg_type == Some("user") {
            state.last_user_at = ts;
            state.notified_for_turn = false;

            let mut timers = pending_timers.lock().unwrap();
            if let Some(timer) = timers.remove(latest_file) {
                timer.abort();
            }
            continue;
        }

        if msg_type == Some("gemini") {
            state.last_assistant_at = ts;

            if state.last_user_at.is_none() {
                state.last_user_at = ts;
                state.notified_for_turn = false;
            }

            let mut timers = pending_timers.lock().unwrap();
            if let Some(timer) = timers.remove(latest_file) {
                timer.abort();
            }

            let file_path = latest_file.clone();
            let states_clone = Arc::clone(&states);
            let timer = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(quiet_ms)).await;

                let (should_notify, duration_ms) = {
                    let states = states_clone.lock().unwrap();
                    if let Some(state) = states.get(&file_path) {
                        if state.notified_for_turn || state.last_notified_at == state.last_assistant_at {
                            return;
                        }

                        let duration = if let (Some(start), Some(end)) = (state.last_user_at, state.last_assistant_at) {
                            if end >= start { Some(end - start) } else { None }
                        } else {
                            None
                        };

                        (true, duration)
                    } else {
                        return;
                    }
                };

                if should_notify {
                    if let Err(e) = crate::notify::send_notifications(
                        "gemini",
                        "Gemini 完成",
                        duration_ms,
                        String::new(),
                        false,
                    ).await {
                        eprintln!("Notification error: {}", e);
                    } else {
                        let mut states = states_clone.lock().unwrap();
                        if let Some(state) = states.get_mut(&file_path) {
                            state.last_notified_at = state.last_assistant_at;
                            state.notified_for_turn = true;
                        }
                    }
                }
            });

            timers.insert(latest_file.clone(), timer);
        }
    }

    state.position = last_modified_ms;
    drop(states_lock);
}
