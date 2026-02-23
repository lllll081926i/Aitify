use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::time::interval;

fn get_codex_token_grace_ms() -> u64 {
    std::env::var("CODEX_TOKEN_GRACE_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(1500)
        .max(200)
}

fn get_codex_seed_catchup_ms() -> u64 {
    std::env::var("CODEX_SEED_CATCHUP_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(30000)
}

const CLAUDE_DIR: &str = ".claude/projects";
const CODEX_DIR: &str = ".codex/sessions";
const GEMINI_DIR: &str = ".gemini/tmp";

fn get_codex_follow_top_n() -> usize {
    std::env::var("CODEX_FOLLOW_TOP_N")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(5)
}

// Codex turn-end confirm 提示词（用于检测是否需要用户确认）
const CODEX_TURN_END_CONFIRM_CUES: &[&str] = &[
    "请确认", "是否继续", "是否开始", "是否开始执行", "是否执行", "是否同意", "是否允许", "是否授权",
    "请选择", "请选", "你希望", "你想", "你要", "要不要", "可以吗", "可以么", "能否", "可否",
    "please confirm", "confirm", "approve", "approval", "proceed", "continue",
    "should i", "shall i", "do you want me", "would you like", "may i",
];

const CODEX_TURN_END_ACTION_WORDS: &[&str] = &[
    "开始", "继续", "执行", "确认", "选择", "提交", "授权", "允许", "同意",
    "proceed", "continue", "execute", "run", "confirm", "choose", "select", "approve", "authorize",
];

// Claude 工作类型（用于判断是否需要取消 pending 计时器）
fn is_claude_work_type(type_str: &str) -> bool {
    matches!(type_str, "progress" | "queue-operation" | "tool_use" | "tool_result" | "thinking" | "reasoning")
}

// Codex 工作类型
fn is_codex_work_type(type_str: &str) -> bool {
    matches!(type_str, "reasoning" | "function_call" | "function_call_output" | "custom_tool_call" | "custom_tool_call_output" | "web_search_call" | "tool_use")
}

fn parse_timestamp(value: &Value) -> Option<i64> {
    // 处理字符串时间戳
    if let Some(s) = value.as_str() {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return None;
        }
        // 尝试 RFC3339
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
            return Some(dt.timestamp_millis());
        }
        // 尝试纯数字字符串
        if let Ok(num) = trimmed.parse::<f64>() {
            if num.is_finite() {
                return Some(if num < 1e12 {
                    (num * 1000.0) as i64
                } else {
                    num as i64
                });
            }
        }
        return None;
    }
    // 处理数字时间戳
    if let Some(num) = value.as_f64() {
        if num.is_finite() {
            return Some(if num < 1e12 {
                (num * 1000.0) as i64
            } else {
                num as i64
            });
        }
    }
    value.as_i64()
}

fn safe_json_parse(line: &str) -> Option<Value> {
    let normalized = line.trim_start_matches('\u{feff}');
    if normalized.is_empty() {
        return None;
    }
    serde_json::from_str(normalized).ok()
}

fn safe_stat(path: &Path) -> Option<std::fs::Metadata> {
    fs::metadata(path).ok()
}

fn now_unix_millis_i64() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn file_mtime_millis(path: &Path) -> Option<u128> {
    safe_stat(path).and_then(|stat| {
        stat.modified()
            .ok()
            .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_millis())
    })
}

fn find_latest_file<F>(root_dir: &Path, mut is_candidate: F) -> Option<PathBuf>
where
    F: FnMut(&Path, &str) -> bool,
{
    let mut latest: Option<(PathBuf, u128)> = None;

    fn walk<F>(dir: &Path, is_candidate: &mut F, latest: &mut Option<(PathBuf, u128)>)
    where
        F: FnMut(&Path, &str) -> bool,
    {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, is_candidate, latest);
                } else if path.is_file() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if !is_candidate(&path, name) {
                            continue;
                        }
                        if let Some(mtime) = file_mtime_millis(&path) {
                            if latest.as_ref().map(|(_, ts)| mtime > *ts).unwrap_or(true) {
                                *latest = Some((path.clone(), mtime));
                            }
                        }
                    }
                }
            }
        }
    }

    walk(root_dir, &mut is_candidate, &mut latest);
    latest.map(|(path, _)| path)
}

fn find_latest_files<F>(root_dir: &Path, mut is_candidate: F, limit: usize) -> Vec<PathBuf>
where
    F: FnMut(&Path, &str) -> bool,
{
    let mut results: Vec<(PathBuf, u128)> = Vec::new();

    fn walk<F>(dir: &Path, is_candidate: &mut F, results: &mut Vec<(PathBuf, u128)>, limit: usize)
    where
        F: FnMut(&Path, &str) -> bool,
    {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, is_candidate, results, limit);
                } else if path.is_file() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if !is_candidate(&path, name) {
                            continue;
                        }
                        if let Some(mtime) = file_mtime_millis(&path) {
                            results.push((path.clone(), mtime));
                            results.sort_by(|a, b| b.1.cmp(&a.1));
                            if results.len() > limit {
                                results.truncate(limit);
                            }
                        }
                    }
                }
            }
        }
    }

    walk(root_dir, &mut is_candidate, &mut results, limit);
    results.into_iter().map(|(path, _)| path).collect()
}

fn extract_text_from_any(value: &Value) -> String {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(arr) => arr
            .iter()
            .map(extract_text_from_any)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(obj) => {
            // 先检查常见文本字段
            for key in &["text", "content", "message", "value", "data"] {
                if let Some(v) = obj.get(*key) {
                    if let Some(s) = v.as_str() {
                        if !s.is_empty() {
                            return s.to_string();
                        }
                        // 递归处理
                        let result = extract_text_from_any(v);
                        if !result.is_empty() {
                            return result;
                        }
                    } else {
                        let result = extract_text_from_any(v);
                        if !result.is_empty() {
                            return result;
                        }
                    }
                }
            }
            // 检查数组字段
            for key in &["content", "parts", "messages"] {
                if let Some(arr) = obj.get(*key).and_then(|v| v.as_array()) {
                    let result = arr
                        .iter()
                        .map(extract_text_from_any)
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !result.is_empty() {
                        return result;
                    }
                }
            }
            String::new()
        }
    }
}

// 检测 Codex turn-end 确认提示
fn detect_turn_end_confirm_prompt(text: &str) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }

    // 取最后 6 行，最多 1200 字符
    let raw = text.replace("\r\n", "\n");
    let limited = if raw.len() > 1200 {
        let byte_offset = raw.len() - 1200;
        let char_offset = raw.char_indices().find(|&(i, _)| i >= byte_offset).map(|(i, _)| i).unwrap_or(raw.len());
        &raw[char_offset..]
    } else {
        &raw
    };

    let lines: Vec<&str> = limited
        .split('\n')
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();

    let tail: Vec<&str> = lines.into_iter().rev().take(6).rev().collect();
    let tail_text = tail.join("\n");
    let tail_lower = tail_text.to_lowercase();

    let last_line = tail.last().unwrap_or(&"");
    let ends_with_question = last_line.ends_with('?') || last_line.ends_with('？');

    // 检查确认提示词
    let cue_near_end = CODEX_TURN_END_CONFIRM_CUES
        .iter()
        .any(|k| !k.is_empty() && tail_lower.contains(&k.to_lowercase()));

    if cue_near_end {
        return Some(if tail_text.len() <= 600 {
            tail_text
        } else {
            let truncated: String = tail_text.chars().take(600).collect();
            truncated
        });
    }

    // 检查动作词 + 问号
    let action_near_end = CODEX_TURN_END_ACTION_WORDS
        .iter()
        .any(|k| tail_lower.contains(&k.to_lowercase()));

    if ends_with_question && action_near_end {
        return Some(tail_text);
    }

    None
}

// 检查是否有选项
fn has_options_in_prompt(text: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim().to_lowercase();
        trimmed.starts_with("选项：") || trimmed.starts_with("选项:")
            || trimmed.starts_with("options:") || trimmed.starts_with("options：")
            || trimmed.starts_with("option:") || trimmed.starts_with("option：")
    })
}

// 标准化源配置
fn normalize_sources(input: &str) -> Vec<&'static str> {
    let input = input.to_lowercase();
    let parts: Vec<&str> = input.split(',').map(|s| s.trim()).collect();

    if parts.contains(&"all") || parts.is_empty() {
        vec!["claude", "codex", "gemini"]
    } else {
        let mut result = Vec::new();
        for part in parts {
            match part {
                "claude" => result.push("claude"),
                "codex" => result.push("codex"),
                "gemini" => result.push("gemini"),
                _ => {}
            }
        }
        if result.is_empty() {
            vec!["claude", "codex", "gemini"]
        } else {
            result
        }
    }
}

// ============ Claude Watch ============

struct ClaudeState {
    current_file: Option<PathBuf>,
    last_file_size: u64,
    last_user_at: Option<i64>,
    last_assistant_at: Option<i64>,
    last_notified_at: Option<i64>,
    notified_for_turn: bool,
    confirm_notified_for_turn: bool,
    last_cwd: Option<String>,
    last_assistant_had_tool_use: bool,
    pending_cancel: Option<Arc<AtomicBool>>,
}

impl ClaudeState {
    fn new() -> Self {
        Self {
            current_file: None,
            last_file_size: 0,
            last_user_at: None,
            last_assistant_at: None,
            last_notified_at: None,
            notified_for_turn: false,
            confirm_notified_for_turn: false,
            last_cwd: None,
            last_assistant_had_tool_use: false,
            pending_cancel: None,
        }
    }

    fn cancel_pending(&mut self) {
        if let Some(flag) = self.pending_cancel.take() {
            flag.store(true, Ordering::Relaxed);
        }
    }

    fn reset_for_new_file(&mut self) {
        self.cancel_pending();
        self.last_file_size = 0;
        self.last_user_at = None;
        self.last_assistant_at = None;
        self.last_notified_at = None;
        self.notified_for_turn = false;
        self.confirm_notified_for_turn = false;
        self.last_assistant_had_tool_use = false;
    }
}

fn has_tool_use_content(obj: &Value) -> bool {
    let message = match obj.get("message") {
        Some(m) => m,
        None => return false,
    };
    if let Some(arr) = message.get("content").and_then(|c| c.as_array()) {
        return arr.iter().any(|item| {
            item.get("type").and_then(|t| t.as_str()) == Some("tool_use")
        });
    }
    false
}

fn process_claude_object(
    obj: &Value,
    _seed: bool,
    state: &mut ClaudeState,
) {
    if obj.get("isSidechain").and_then(|v| v.as_bool()) == Some(true) {
        return;
    }

    let ts = obj.get("timestamp").and_then(parse_timestamp);
    let record_type = obj.get("type").and_then(|v| v.as_str());

    if let Some(cwd) = obj.get("cwd").and_then(|v| v.as_str()) {
        state.last_cwd = Some(cwd.to_string());
    }

    match record_type {
        Some("user") => {
            state.cancel_pending();
            state.confirm_notified_for_turn = false;
            state.notified_for_turn = false;
            state.last_assistant_had_tool_use = false;
            state.last_user_at = ts;
        }
        Some("assistant") => {
            state.last_assistant_had_tool_use = has_tool_use_content(obj);
            state.last_assistant_at = ts.or_else(|| Some(now_unix_millis_i64()));

            if state.last_user_at.is_none() {
                state.last_user_at = state.last_assistant_at;
                state.notified_for_turn = false;
            }
        }
        Some(work_type) if is_claude_work_type(work_type) => {
            // work in progress — cancel any pending completion timer
            state.cancel_pending();
        }
        _ => {}
    }
}

// ============ Codex Watch ============

struct CodexSessionState {
    processed_lines: usize,
    last_user_at: Option<i64>,
    last_assistant_at: Option<i64>,
    last_notified_assistant_at: Option<i64>,
    last_task_started_at: Option<i64>,
    current_turn_id: Option<String>,
    last_notified_turn_id: Option<String>,
    last_cwd: Option<String>,
    last_agent_content: Option<String>,
    last_request_user_input_prompt: String,
    confirm_notified_for_turn: bool,
    interaction_required_for_turn: bool,
    pending_request_user_input_call_ids: HashSet<String>,
    pending_request_user_input_without_id: usize,
    last_interaction_resolved_at: Option<i64>,
    collaboration_mode_kind: String,
    // pending completion: (assistant_at, token_seen, cancel_flag)
    pending_completion: Option<(i64, bool, Arc<AtomicBool>)>,
}

impl CodexSessionState {
    fn new() -> Self {
        Self {
            processed_lines: 0,
            last_user_at: None,
            last_assistant_at: None,
            last_notified_assistant_at: None,
            last_task_started_at: None,
            current_turn_id: None,
            last_notified_turn_id: None,
            last_cwd: None,
            last_agent_content: None,
            last_request_user_input_prompt: String::new(),
            confirm_notified_for_turn: false,
            interaction_required_for_turn: false,
            pending_request_user_input_call_ids: HashSet::new(),
            pending_request_user_input_without_id: 0,
            last_interaction_resolved_at: None,
            collaboration_mode_kind: String::new(),
            pending_completion: None,
        }
    }

    fn clear_pending_completion(&mut self) {
        if let Some((_, _, cancel)) = self.pending_completion.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    fn reset_for_new_turn(&mut self) {
        self.clear_pending_completion();
        self.last_agent_content = None;
        self.confirm_notified_for_turn = false;
        self.interaction_required_for_turn = false;
        self.pending_request_user_input_call_ids.clear();
        self.pending_request_user_input_without_id = 0;
        self.last_interaction_resolved_at = None;
        self.last_request_user_input_prompt = String::new();
    }
}

fn extract_request_user_input_text(payload: &Value) -> String {
    let args = payload
        .get("arguments")
        .or_else(|| payload.get("function").and_then(|f| f.get("arguments")));

    // 先解析可能为字符串的 arguments
    let parsed_args: Option<Value> = match args {
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).ok(),
        other => other.cloned(),
    };

    let empty_map = serde_json::Map::new();
    let args_obj = match parsed_args.as_ref() {
        Some(Value::Object(o)) => o,
        _ => &empty_map,
    };

    let mut lines = Vec::new();

    let ensure_question = |value: &str| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        if trimmed.ends_with('?') || trimmed.ends_with('？') {
            trimmed.to_string()
        } else {
            format!("{}？", trimmed)
        }
    };

    if let Some(questions) = args_obj.get("questions").and_then(|v| v.as_array()) {
        for q in questions {
            if let Some(q_obj) = q.as_object() {
                if let Some(header) = q_obj.get("header").and_then(|h| h.as_str()) {
                    if !header.trim().is_empty() {
                        lines.push(header.trim().to_string());
                    }
                }
                if let Some(question) = q_obj.get("question").and_then(|q| q.as_str()) {
                    let q = ensure_question(question);
                    if !q.is_empty() {
                        lines.push(q);
                    }
                }
                if let Some(options) = q_obj.get("options").and_then(|v| v.as_array()) {
                    let labels: Vec<&str> = options.iter()
                        .filter_map(|o| o.get("label").and_then(|l| l.as_str()))
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !labels.is_empty() {
                        lines.push(format!("选项: {}", labels.join(" / ")));
                    }
                }
            }
        }
    } else if let Some(question) = args_obj.get("question").and_then(|v| v.as_str()) {
        let q = ensure_question(question);
        if !q.is_empty() {
            lines.push(q);
        }
        if let Some(options) = args_obj.get("options").and_then(|v| v.as_array()) {
            let labels: Vec<&str> = options.iter()
                .filter_map(|o| o.get("label").and_then(|l| l.as_str()))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            if !labels.is_empty() {
                lines.push(format!("选项: {}", labels.join(" / ")));
            }
        }
    }

    if lines.is_empty() {
        "需要你确认下一步？".to_string()
    } else {
        lines.join("\n")
    }
}

fn process_codex_object(
    obj: &Value,
    seed: bool,
    state: &mut CodexSessionState,
) {
    let ts = obj.get("timestamp").and_then(parse_timestamp);

    // turn_context
    if obj.get("type").and_then(|v| v.as_str()) == Some("turn_context") {
        if let Some(payload) = obj.get("payload").and_then(|v| v.as_object()) {
            if let Some(cwd) = payload.get("cwd").and_then(|v| v.as_str()) {
                state.last_cwd = Some(cwd.to_string());
            }
            if let Some(turn_id) = payload.get("turn_id").and_then(|v| v.as_str()) {
                let next_turn_id = turn_id.to_string();
                if state.current_turn_id.is_some() && state.current_turn_id != Some(next_turn_id.clone()) {
                    state.reset_for_new_turn();
                }
                state.current_turn_id = Some(next_turn_id);
            }
            if let Some(collab_mode) = payload.get("collaboration_mode").and_then(|v| v.as_object()) {
                if let Some(mode) = collab_mode.get("mode").and_then(|v| v.as_str()) {
                    state.collaboration_mode_kind = mode.to_string();
                }
            }
        }
        return;
    }

    // response_item
    if obj.get("type").and_then(|v| v.as_str()) == Some("response_item") {
        if let Some(payload) = obj.get("payload").and_then(|v| v.as_object()) {
            let payload_type = payload.get("type").and_then(|v| v.as_str());
            let payload_role = payload.get("role").and_then(|v| v.as_str());

            // user message
            if payload_type == Some("message") && payload_role == Some("user") {
                state.clear_pending_completion();
                state.last_task_started_at = None;
                state.last_user_at = ts;
                state.reset_for_new_turn();
                return;
            }

            // request_user_input
            let is_request_user_input = matches!(payload_type, Some("function_call") | Some("custom_tool_call") | Some("tool_use"))
                && (payload.get("name").and_then(|v| v.as_str()) == Some("request_user_input")
                    || payload.get("function_name").and_then(|v| v.as_str()) == Some("request_user_input")
                    || payload.get("function").and_then(|f| f.get("name")).and_then(|v| v.as_str()) == Some("request_user_input"));

            if is_request_user_input {
                state.clear_pending_completion();
                state.interaction_required_for_turn = true;

                let call_id = payload.get("call_id").and_then(|v| v.as_str())
                    .or_else(|| payload.get("id").and_then(|v| v.as_str()))
                    .or_else(|| payload.get("tool_call_id").and_then(|v| v.as_str()))
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty());

                if let Some(id) = call_id {
                    state.pending_request_user_input_call_ids.insert(id.to_string());
                } else {
                    state.pending_request_user_input_without_id += 1;
                }

                state.last_request_user_input_prompt = extract_request_user_input_text(&Value::Object(payload.clone()));
                return;
            }

            // function_call_output / custom_tool_call_output
            if matches!(payload_type, Some("function_call_output") | Some("custom_tool_call_output")) {
                let call_id = payload.get("call_id").and_then(|v| v.as_str())
                    .or_else(|| payload.get("id").and_then(|v| v.as_str()))
                    .or_else(|| payload.get("tool_call_id").and_then(|v| v.as_str()))
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty());

                if let Some(id) = call_id {
                    state.pending_request_user_input_call_ids.remove(id);
                } else if state.pending_request_user_input_without_id > 0 {
                    state.pending_request_user_input_without_id -= 1;
                }

                if state.pending_request_user_input_call_ids.is_empty()
                    && state.pending_request_user_input_without_id == 0
                {
                    state.interaction_required_for_turn = false;
                    state.last_interaction_resolved_at = ts.or_else(|| Some(now_unix_millis_i64()));
                    state.last_request_user_input_prompt = String::new();
                }
                return;
            }

            // work type — cancel pending
            if payload_type.map(|t| is_codex_work_type(t)).unwrap_or(false) {
                state.clear_pending_completion();
                return;
            }

            // assistant message
            if payload_type == Some("message") && payload_role == Some("assistant") {
                if seed { return; }
                state.clear_pending_completion();
                let assistant_text = extract_text_from_any(&Value::Object(payload.clone()));
                if !assistant_text.is_empty() {
                    state.last_agent_content = Some(assistant_text);
                }
                state.last_assistant_at = ts.or_else(|| Some(now_unix_millis_i64()));
                return;
            }
        }
    }

    // event_msg
    if obj.get("type").and_then(|v| v.as_str()) == Some("event_msg") {
        if let Some(payload) = obj.get("payload").and_then(|v| v.as_object()) {
            let event_type = payload.get("type").and_then(|v| v.as_str());

            match event_type {
                Some("task_started") => {
                    if !seed {
                        // flush any pending completion before new task
                        state.clear_pending_completion();
                    }
                    if let Some(turn_id) = payload.get("turn_id").and_then(|v| v.as_str()) {
                        state.current_turn_id = Some(turn_id.to_string());
                    }
                    if let Some(kind) = payload.get("collaboration_mode_kind").and_then(|v| v.as_str()) {
                        state.collaboration_mode_kind = kind.to_string();
                    }
                    state.last_task_started_at = ts;
                    state.reset_for_new_turn();
                }

                Some("task_complete") => {
                    if seed {
                        return;
                    }

                    let turn_id = payload.get("turn_id").and_then(|v| v.as_str()).map(|s| s.to_string());
                    if let Some(ref tid) = turn_id {
                        if state.last_notified_turn_id.as_ref() == Some(tid) {
                            return;
                        }
                    }

                    state.clear_pending_completion();

                    let completion_at = ts.unwrap_or_else(now_unix_millis_i64);

                    if let Some(last_agent_msg) = payload.get("last_agent_message").and_then(|v| v.as_str()) {
                        if !last_agent_msg.is_empty() {
                            state.last_agent_content = Some(last_agent_msg.to_string());
                            state.last_assistant_at = Some(completion_at);
                        }
                    }

                    // assistant content stale if it predates the last interaction resolution
                    let assistant_stale = state.last_interaction_resolved_at
                        .zip(state.last_assistant_at)
                        .map(|(resolved, asst)| asst <= resolved)
                        .unwrap_or(false)
                        && payload.get("last_agent_message").and_then(|v| v.as_str()).map(|s| s.is_empty()).unwrap_or(true);

                    if state.interaction_required_for_turn {
                        let request_prompt = state.last_request_user_input_prompt.clone();
                        let request_has_options = has_options_in_prompt(&request_prompt);
                        let agent_content = state.last_agent_content.clone().unwrap_or_default();

                        if request_has_options {
                            let cwd = state.last_cwd.clone().unwrap_or_default();
                            tauri::async_runtime::spawn(async move {
                                let _ = crate::notify::send_notifications("codex", &request_prompt, None, cwd, false, Some("confirm")).await;
                            });
                        } else {
                            let prompt = detect_turn_end_confirm_prompt(&agent_content);
                            let msg = prompt.unwrap_or_else(|| "需要你的确认".to_string());
                            let cwd = state.last_cwd.clone().unwrap_or_default();
                            tauri::async_runtime::spawn(async move {
                                let _ = crate::notify::send_notifications("codex", &msg, None, cwd, false, Some("confirm")).await;
                            });
                        }
                        if let Some(tid) = turn_id {
                            state.last_notified_turn_id = Some(tid);
                        }
                        return;
                    }

                    if state.confirm_notified_for_turn {
                        return;
                    }

                    if !assistant_stale {
                        let agent_content = state.last_agent_content.clone().unwrap_or_default();
                        let prompt = detect_turn_end_confirm_prompt(&agent_content);
                        if let Some(p) = prompt {
                            let cwd = state.last_cwd.clone().unwrap_or_default();
                            tauri::async_runtime::spawn(async move {
                                let _ = crate::notify::send_notifications("codex", &p, None, cwd, false, Some("confirm")).await;
                            });
                            if let Some(tid) = turn_id {
                                state.last_notified_turn_id = Some(tid);
                            }
                            return;
                        }
                    }

                    let start_at = state.last_user_at.or(state.last_task_started_at);
                    let duration_ms = start_at.map(|start| {
                        if completion_at >= start { completion_at - start } else { 0 }
                    });

                    let cwd = state.last_cwd.clone().unwrap_or_default();
                    tauri::async_runtime::spawn(async move {
                        let _ = crate::notify::send_notifications("codex", "Codex 任务已完成", duration_ms, cwd, false, Some("complete")).await;
                    });

                    state.last_notified_assistant_at = Some(completion_at);
                    state.last_notified_turn_id = turn_id;
                    state.confirm_notified_for_turn = true;
                }

                Some("user_message") => {
                    state.clear_pending_completion();
                    state.last_task_started_at = None;
                    state.last_user_at = ts;
                    state.reset_for_new_turn();
                }

                Some("token_count") => {
                    if !seed {
                        // mark token seen for pending completion grace period
                        if let Some((asst_at, ref mut token_seen, ref cancel)) = state.pending_completion {
                            if !*token_seen && !cancel.load(Ordering::Relaxed) {
                                *token_seen = true;
                                let cancel2 = cancel.clone();
                                let grace_ms = get_codex_token_grace_ms();
                                let cwd = state.last_cwd.clone().unwrap_or_default();
                                let start_at = state.last_user_at.or(state.last_task_started_at);
                                let duration_ms = start_at.map(|s| if asst_at >= s { asst_at - s } else { 0 });
                                tauri::async_runtime::spawn(async move {
                                    tokio::time::sleep(Duration::from_millis(grace_ms)).await;
                                    if cancel2.load(Ordering::Relaxed) { return; }
                                    let _ = crate::notify::send_notifications("codex", "Codex 任务已完成", duration_ms, cwd, false, Some("complete")).await;
                                });
                            }
                        }
                    }
                }

                Some("agent_reasoning") => {
                    state.clear_pending_completion();
                }

                Some("agent_message") => {
                    if seed {
                        return;
                    }

                    let payload_val = obj.get("payload").unwrap_or(&Value::Null);
                    state.last_assistant_at = ts.or_else(|| Some(now_unix_millis_i64()));

                    let content = payload_val
                        .get("content").and_then(|c| c.as_str())
                        .or_else(|| payload_val.get("message").and_then(|m| m.as_str()))
                        .or_else(|| payload_val.get("text").and_then(|t| t.as_str()))
                        .or_else(|| payload_val.get("data").and_then(|d| d.as_str()))
                        .map(|s| s.to_string());

                    if let Some(c) = content {
                        if !c.trim().is_empty() {
                            state.last_agent_content = Some(c);
                        }
                    }
                }

                _ => {}
            }
        }
    }
}

// ============ Gemini Watch ============

struct GeminiState {
    current_file: Option<PathBuf>,
    current_mtime_ms: u64,
    last_count: usize,
    last_user_at: Option<i64>,
    last_gemini_at: Option<i64>,
    last_notified_gemini_at: Option<i64>,
    confirm_notified_for_turn: bool,
    // cancel flag for debounced notify timer
    pending_cancel: Option<Arc<AtomicBool>>,
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
            confirm_notified_for_turn: false,
            pending_cancel: None,
        }
    }

    fn cancel_pending(&mut self) {
        if let Some(flag) = self.pending_cancel.take() {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

fn process_gemini_message(
    msg: &Value,
    state: &mut GeminiState,
    quiet_ms: u64,
) {
    let ts = msg.get("timestamp").and_then(parse_timestamp);
    let msg_type = msg.get("type").and_then(|v| v.as_str());

    match msg_type {
        Some("user") => {
            state.cancel_pending();
            state.last_user_at = ts;
            state.last_gemini_at = None;
            state.last_notified_gemini_at = None;
            state.confirm_notified_for_turn = false;
        }
        Some("gemini") => {
            state.last_gemini_at = ts;

            if state.confirm_notified_for_turn {
                state.cancel_pending();
                return;
            }

            // Schedule debounced notification
            state.cancel_pending();
            let cancel = Arc::new(AtomicBool::new(false));
            state.pending_cancel = Some(cancel.clone());
            let target_gemini_at = state.last_gemini_at;
            let user_at = state.last_user_at;
            let last_notified = state.last_notified_gemini_at;

            if last_notified == target_gemini_at {
                return;
            }

            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(quiet_ms)).await;
                if cancel.load(Ordering::Relaxed) { return; }
                let end_at = match target_gemini_at { Some(t) => t, None => return };
                let start_at = match user_at { Some(t) => t, None => return };
                let duration_ms = if end_at >= start_at { Some(end_at - start_at) } else { None };
                let _ = crate::notify::send_notifications("gemini", "Gemini 任务已完成", duration_ms, String::new(), false, Some("complete")).await;
            });
        }
        _ => {}
    }
}

// ============ 主循环 ============

pub fn start_watch<F>(
    sources: &str,
    interval_ms: i32,
    gemini_quiet_ms: i32,
    claude_quiet_ms: i32,
    mut log_callback: F,
) -> Result<Box<dyn FnOnce() + Send>, Box<dyn std::error::Error>>
where
    F: FnMut(String) + Send + 'static,
{
    let home = match get_home_dir() {
        Some(h) => h,
        None => return Err("Cannot find home directory".into()),
    };

    let claude_root = home.join(CLAUDE_DIR);
    let codex_root = home.join(CODEX_DIR);
    let gemini_root = home.join(GEMINI_DIR);

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    let sources = normalize_sources(sources);
    let claude_quiet_ms = (claude_quiet_ms.max(500) as u64).max(3000);
    let gemini_quiet_ms = (gemini_quiet_ms.max(500) as u64).max(3000);

    tauri::async_runtime::spawn(async move {
        let mut claude_state = ClaudeState::new();
        let mut codex_states: std::collections::HashMap<PathBuf, CodexSessionState> = std::collections::HashMap::new();
        let mut gemini_state = GeminiState::new();

        let mut tick_interval = interval(Duration::from_millis((interval_ms.max(500) as u64).max(1000)));
        let mut cleanup_counter = 0u32;

        log_callback(format!("[watch] started with sources: {:?}", sources));

        while running_clone.load(Ordering::Relaxed) {
            tick_interval.tick().await;

            // Monitor Claude
            if sources.contains(&"claude") && claude_root.exists() {
                if let Some(latest_file) = find_latest_file(&claude_root, |_, name| name.to_lowercase().ends_with(".jsonl")) {
                    if claude_state.current_file.as_ref() != Some(&latest_file) {
                        claude_state.current_file = Some(latest_file.clone());
                        claude_state.reset_for_new_file();
                        log_callback(format!("[watch][claude] following {:?}", latest_file));

                        // Seed: read entire file with seed=true
                        if let Ok(content) = fs::read_to_string(&latest_file) {
                            for line in content.lines() {
                                if let Some(obj) = safe_json_parse(line) {
                                    process_claude_object(&obj, true, &mut claude_state);
                                }
                            }
                            claude_state.last_file_size = content.len() as u64;
                        }

                        // scheduleSeedNotifyIfNeeded
                        if let (Some(user_at), Some(assistant_at)) = (claude_state.last_user_at, claude_state.last_assistant_at) {
                            if assistant_at >= user_at && !claude_state.notified_for_turn && !claude_state.confirm_notified_for_turn {
                                let now = now_unix_millis_i64();
                                let window_ms = (claude_quiet_ms * 2).max(15000) as i64;
                                if now - assistant_at <= window_ms {
                                    let had_tool_use = claude_state.last_assistant_had_tool_use;
                                    let adaptive_ms = if had_tool_use { claude_quiet_ms } else { claude_quiet_ms.min(15000) };
                                    let cancel = Arc::new(AtomicBool::new(false));
                                    claude_state.pending_cancel = Some(cancel.clone());
                                    let cwd = claude_state.last_cwd.clone().unwrap_or_default();
                                    let duration_ms = assistant_at - user_at;
                                    tauri::async_runtime::spawn(async move {
                                        tokio::time::sleep(Duration::from_millis(adaptive_ms)).await;
                                        if cancel.load(Ordering::Relaxed) { return; }
                                        let _ = crate::notify::send_notifications("claude", "Claude 任务已完成", Some(duration_ms), cwd, false, Some("complete")).await;
                                    });
                                    claude_state.notified_for_turn = true;
                                    claude_state.confirm_notified_for_turn = true;
                                    claude_state.last_notified_at = Some(assistant_at);
                                }
                            }
                        }
                    } else {
                        // Incremental: only read new bytes since last position
                        let current_size = safe_stat(&latest_file).map(|s| s.len()).unwrap_or(0);
                        if current_size < claude_state.last_file_size {
                            // File truncated/rotated
                            claude_state.last_file_size = 0;
                        }
                        if current_size > claude_state.last_file_size {
                            if let Ok(content) = fs::read_to_string(&latest_file) {
                                let prev_assistant_at = claude_state.last_assistant_at;
                                let new_content = if claude_state.last_file_size == 0 {
                                    content.as_str()
                                } else {
                                    let byte_pos = claude_state.last_file_size as usize;
                                    // Find a valid UTF-8 char boundary at or after byte_pos
                                    let safe_pos = (byte_pos..=content.len())
                                        .find(|&i| content.is_char_boundary(i))
                                        .unwrap_or(content.len());
                                    &content[safe_pos..]
                                };
                                for line in new_content.lines() {
                                    if let Some(obj) = safe_json_parse(line) {
                                        process_claude_object(&obj, false, &mut claude_state);
                                    }
                                }
                                claude_state.last_file_size = content.len() as u64;

                                if claude_state.last_assistant_at != prev_assistant_at {
                                    if let (Some(user_at), Some(assistant_at)) = (claude_state.last_user_at, claude_state.last_assistant_at) {
                                        if assistant_at >= user_at && !claude_state.notified_for_turn && !claude_state.confirm_notified_for_turn {
                                            let had_tool_use = claude_state.last_assistant_had_tool_use;
                                            let adaptive_ms = if had_tool_use { claude_quiet_ms } else { claude_quiet_ms.min(15000) };
                                            let cancel = Arc::new(AtomicBool::new(false));
                                            claude_state.pending_cancel = Some(cancel.clone());
                                            let cwd = claude_state.last_cwd.clone().unwrap_or_default();
                                            let duration_ms = assistant_at - user_at;
                                            tauri::async_runtime::spawn(async move {
                                                tokio::time::sleep(Duration::from_millis(adaptive_ms)).await;
                                                if cancel.load(Ordering::Relaxed) { return; }
                                                let _ = crate::notify::send_notifications("claude", "Claude 任务已完成", Some(duration_ms), cwd, false, Some("complete")).await;
                                            });
                                            claude_state.notified_for_turn = true;
                                            claude_state.confirm_notified_for_turn = true;
                                            claude_state.last_notified_at = Some(assistant_at);
                                            log_callback(format!("[watch][claude] notification scheduled ({}ms adaptive)", adaptive_ms));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Monitor Codex
            if sources.contains(&"codex") && codex_root.exists() {
                let follow_top_n = get_codex_follow_top_n();
                let seed_catchup_ms = get_codex_seed_catchup_ms();
                let latest = find_latest_files(&codex_root, |_, name| name.to_lowercase().ends_with(".jsonl"), follow_top_n);

                codex_states.retain(|path, state| {
                    if latest.contains(path) {
                        true
                    } else {
                        state.clear_pending_completion();
                        false
                    }
                });

                for file_path in latest {
                    if !codex_states.contains_key(&file_path) {
                        if let Ok(content) = fs::read_to_string(&file_path) {
                            let mut state = CodexSessionState::new();
                            let lines: Vec<&str> = content.lines().collect();
                            let total = lines.len();

                            // Pass 1: seed (no notifications)
                            for line in &lines {
                                if let Some(obj) = safe_json_parse(line) {
                                    process_codex_object(&obj, true, &mut state);
                                }
                            }

                            // Pass 2: seedCatchupMs — treat recent lines as live
                            if seed_catchup_ms > 0 {
                                let since = now_unix_millis_i64() - seed_catchup_ms as i64;
                                for line in &lines {
                                    if let Some(obj) = safe_json_parse(line) {
                                        let ts = obj.get("timestamp").and_then(parse_timestamp);
                                        if ts.map(|t| t >= since).unwrap_or(false) {
                                            process_codex_object(&obj, false, &mut state);
                                        }
                                    }
                                }
                            }

                            state.processed_lines = total;
                            log_callback(format!("[watch][codex] following {:?}", file_path));
                            codex_states.insert(file_path.clone(), state);
                        }
                    } else if let Some(state) = codex_states.get_mut(&file_path) {
                        if let Ok(content) = fs::read_to_string(&file_path) {
                            let lines: Vec<&str> = content.lines().collect();
                            for line in lines.iter().skip(state.processed_lines) {
                                if let Some(obj) = safe_json_parse(line) {
                                    process_codex_object(&obj, false, state);
                                }
                            }
                            state.processed_lines = lines.len();
                        }
                    }
                }
            }

            // Monitor Gemini
            if sources.contains(&"gemini") && gemini_root.exists() {
                if let Some(latest_file) = find_latest_file(&gemini_root, |full_path, name| {
                    if !name.to_lowercase().ends_with(".json") {
                        return false;
                    }
                    if !name.to_lowercase().starts_with("session-") {
                        return false;
                    }
                    full_path.components().any(|c| c.as_os_str() == "chats")
                }) {
                    let stat = match safe_stat(&latest_file) {
                        Some(s) => s,
                        None => continue,
                    };

                    let mtime_ms = stat
                        .modified()
                        .ok()
                        .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);

                    if gemini_state.current_file.as_ref() != Some(&latest_file) {
                        gemini_state.cancel_pending();
                        gemini_state.current_file = Some(latest_file.clone());
                        gemini_state.current_mtime_ms = mtime_ms;
                        gemini_state.last_user_at = None;
                        gemini_state.last_gemini_at = None;
                        gemini_state.last_notified_gemini_at = None;
                        gemini_state.confirm_notified_for_turn = false;

                        if let Ok(content) = fs::read_to_string(&latest_file) {
                            if let Ok(json) = serde_json::from_str::<Value>(&content) {
                                if let Some(messages) = json.get("messages").and_then(|m| m.as_array()) {
                                    gemini_state.last_count = messages.len();
                                    for msg in messages {
                                        process_gemini_message(msg, &mut gemini_state, gemini_quiet_ms);
                                    }
                                    // After seeding, mark notified so we don't re-fire on old data
                                    gemini_state.last_notified_gemini_at = gemini_state.last_gemini_at;
                                    gemini_state.cancel_pending();
                                    log_callback(format!("[watch][gemini] following {:?}", latest_file));
                                }
                            }
                        }
                        continue;
                    }

                    if mtime_ms <= gemini_state.current_mtime_ms {
                        continue;
                    }

                    let content = match fs::read_to_string(&latest_file) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };

                    let json: Value = match serde_json::from_str(&content) {
                        Ok(j) => j,
                        Err(_) => continue,
                    };

                    let messages = match json.get("messages").and_then(|m| m.as_array()) {
                        Some(m) => m,
                        None => continue,
                    };

                    if messages.len() <= gemini_state.last_count {
                        gemini_state.current_mtime_ms = mtime_ms;
                        gemini_state.last_count = messages.len();
                        continue;
                    }

                    for msg in &messages[gemini_state.last_count..] {
                        process_gemini_message(msg, &mut gemini_state, gemini_quiet_ms);
                    }

                    gemini_state.current_mtime_ms = mtime_ms;
                    gemini_state.last_count = messages.len();
                }
            }

            // 定期清理
            cleanup_counter += 1;
            if cleanup_counter >= 60 {
                cleanup_counter = 0;
                // 可以在这里添加清理逻辑
            }
        }

        log_callback("[watch] stopped".to_string());
    });

    Ok(Box::new(move || {
        running.store(false, Ordering::Relaxed);
    }))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_sources() {
        assert_eq!(normalize_sources("all"), vec!["claude", "codex", "gemini"]);
        assert_eq!(normalize_sources(""), vec!["claude", "codex", "gemini"]);
        assert_eq!(normalize_sources("claude"), vec!["claude"]);
        assert_eq!(normalize_sources("claude,codex"), vec!["claude", "codex"]);
    }

    #[test]
    fn test_parse_timestamp() {
        let ts_str = serde_json::json!("2024-01-01T00:00:00Z");
        assert!(parse_timestamp(&ts_str).is_some());

        let ts_num_sec: Value = serde_json::json!(1704067200);
        assert_eq!(parse_timestamp(&ts_num_sec), Some(1704067200000));

        let ts_num_ms: Value = serde_json::json!(1704067200000i64);
        assert_eq!(parse_timestamp(&ts_num_ms), Some(1704067200000));
    }

    #[test]
    fn test_is_claude_work_type() {
        assert!(is_claude_work_type("progress"));
        assert!(is_claude_work_type("tool_use"));
        assert!(!is_claude_work_type("user"));
        assert!(!is_claude_work_type("assistant"));
    }

    #[test]
    fn test_is_codex_work_type() {
        assert!(is_codex_work_type("function_call"));
        assert!(is_codex_work_type("reasoning"));
        assert!(!is_codex_work_type("user_message"));
        assert!(!is_codex_work_type("task_complete"));
    }

    #[test]
    fn test_extract_text_from_any() {
        let text_only = serde_json::json!("Hello");
        assert_eq!(extract_text_from_any(&text_only), "Hello");

        let obj_with_text = serde_json::json!({"text": "Hello"});
        assert_eq!(extract_text_from_any(&obj_with_text), "Hello");
    }

    #[test]
    fn test_detect_turn_end_confirm_prompt() {
        let text = "请确认是否继续执行？";
        assert!(detect_turn_end_confirm_prompt(text).is_some());

        let text = "Execute the command";
        assert!(detect_turn_end_confirm_prompt(text).is_none());
    }

    #[test]
    fn test_has_options_in_prompt() {
        let text = "选项：A / B / C";
        assert!(has_options_in_prompt(text));

        let text = "options: A / B / C";
        assert!(has_options_in_prompt(text));

        let text = "没有选项";
        assert!(!has_options_in_prompt(text));
    }
}
