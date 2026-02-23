use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::time::interval;

const CLAUDE_DIR: &str = ".claude/projects";
const CODEX_DIR: &str = ".codex/sessions";
const GEMINI_DIR: &str = ".gemini/tmp";

// Codex 环境变量配置
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
                        if safe_stat(&path).is_some() {
                            let stat = safe_stat(&path).unwrap();
                            let mtime = stat
                                .modified()
                                .ok()
                                .and_then(|m: std::time::SystemTime| m.duration_since(SystemTime::UNIX_EPOCH).ok())
                                .map(|d: Duration| d.as_millis())
                                .unwrap_or(0);
                            if latest.is_none() || mtime > latest.as_ref().unwrap().1 {
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

fn find_latest_files<F>(root_dir: &Path, mut is_candidate: F, _limit: usize) -> Vec<PathBuf>
where
    F: FnMut(&Path, &str) -> bool,
{
    let limit = _limit;
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
                        if safe_stat(&path).is_some() {
                            let stat = safe_stat(&path).unwrap();
                            let mtime = stat
                                .modified()
                                .ok()
                                .and_then(|m: std::time::SystemTime| m.duration_since(SystemTime::UNIX_EPOCH).ok())
                                .map(|d: Duration| d.as_millis())
                                .unwrap_or(0);
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

fn has_content_type(message: &Value, expected_type: &str) -> bool {
    if let Some(content) = message.get("content").and_then(|c| c.as_array()) {
        return content.iter().any(|item| {
            item.get("type").and_then(|t| t.as_str()) == Some(expected_type)
        });
    }
    false
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

fn extract_message_text(message: &Value) -> String {
    if let Some(obj) = message.as_object() {
        if let Some(content) = obj.get("content") {
            if let Some(arr) = content.as_array() {
                return arr
                    .iter()
                    .map(extract_text_from_any)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
            }
            return extract_text_from_any(content);
        }
    }
    extract_text_from_any(message)
}

// 检测 Codex turn-end 确认提示
fn detect_turn_end_confirm_prompt(text: &str, _plan_mode: bool) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }

    // 取最后 6 行，最多 1200 字符
    let raw = text.replace("\r\n", "\n");
    let limited = if raw.len() > 1200 {
        &raw[raw.len() - 1200..]
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
    let ends_with_question = last_line.ends_with('?') || last_line.ends_with('?');

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
    text.lines()
        .any(|line| {
            let trimmed = line.trim();
            trimmed.starts_with("选项：") || trimmed.starts_with("选项:")
                || trimmed.starts_with("options:") || trimmed.starts_with("options：")
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
    last_user_at: Option<i64>,
    last_assistant_at: Option<i64>,
    last_notified_at: Option<i64>,
    notified_for_turn: bool,
    confirm_notified_for_turn: bool,
    last_cwd: Option<String>,
    last_user_text: String,
    last_assistant_text: String,
    last_assistant_content: Option<String>,
    last_assistant_had_tool_use: bool,
}

impl ClaudeState {
    fn new() -> Self {
        Self {
            current_file: None,
            last_user_at: None,
            last_assistant_at: None,
            last_notified_at: None,
            notified_for_turn: false,
            confirm_notified_for_turn: false,
            last_cwd: None,
            last_user_text: String::new(),
            last_assistant_text: String::new(),
            last_assistant_content: None,
            last_assistant_had_tool_use: false,
        }
    }

    fn reset_for_new_file(&mut self) {
        self.last_user_at = None;
        self.last_assistant_at = None;
        self.last_notified_at = None;
        self.notified_for_turn = false;
        self.confirm_notified_for_turn = false;
        self.last_user_text.clear();
        self.last_assistant_text.clear();
        self.last_assistant_content = None;
        self.last_assistant_had_tool_use = false;
    }
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

    // 更新 cwd
    if let Some(cwd) = obj.get("cwd").and_then(|v| v.as_str()) {
        state.last_cwd = Some(cwd.to_string());
    }

    match record_type {
        Some("user") => {
            let user_text = extract_message_text(obj.get("message").unwrap_or(&Value::Null));
            state.last_user_text = user_text;
            state.last_assistant_text.clear();
            state.last_assistant_content = None;
            state.last_assistant_had_tool_use = false;
            state.confirm_notified_for_turn = false;
            state.notified_for_turn = false;
            state.last_user_at = ts;
        }
        Some("assistant") => {
            let msg = obj.get("message").unwrap_or(&Value::Null);
            let assistant_text = extract_message_text(msg);
            if !assistant_text.is_empty() {
                state.last_assistant_text = assistant_text;
            }

            let has_tool_use = has_content_type(msg, "tool_use");
            state.last_assistant_had_tool_use = has_tool_use;

            // 提取纯文本内容
            let content = if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
                content_arr
                    .iter()
                    .filter(|item| item.get("type").and_then(|t| t.as_str()) == Some("text"))
                    .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            } else if let Some(content_str) = msg.get("content").and_then(|c| c.as_str()) {
                content_str.to_string()
            } else if let Some(text_str) = msg.get("text").and_then(|t| t.as_str()) {
                text_str.to_string()
            } else {
                String::new()
            };

            if !content.trim().is_empty() {
                state.last_assistant_content = Some(content);
            }

            state.last_assistant_at = ts.or_else(|| {
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_millis() as i64)
            });

            if state.last_user_at.is_none() {
                state.last_user_at = state.last_assistant_at;
                state.notified_for_turn = false;
            }
        }
        Some(work_type) if is_claude_work_type(work_type) => {
            // 工作中事件，不触发通知
        }
        _ => {}
    }
}

// ============ Codex Watch ============

struct CodexSessionState {
    last_user_at: Option<i64>,
    last_assistant_at: Option<i64>,
    last_notified_assistant_at: Option<i64>,
    last_task_started_at: Option<i64>,
    current_turn_id: Option<String>,
    last_notified_turn_id: Option<String>,
    collaboration_mode_kind: String,
    last_cwd: Option<String>,
    last_agent_content: Option<String>,
    last_user_text: String,
    last_assistant_text: String,
    last_request_user_input_prompt: String,
    confirm_notified_for_turn: bool,
    interaction_required_for_turn: bool,
    pending_request_user_input_call_ids: HashSet<String>,
    pending_request_user_input_without_id: usize,
    last_interaction_resolved_at: Option<i64>,
    interaction_notified_for_turn: bool,
    codex_task_protocol_seen: bool,
}

impl CodexSessionState {
    fn new(_file_path: PathBuf) -> Self {
        Self {
            last_user_at: None,
            last_assistant_at: None,
            last_notified_assistant_at: None,
            last_task_started_at: None,
            current_turn_id: None,
            last_notified_turn_id: None,
            collaboration_mode_kind: String::new(),
            last_cwd: None,
            last_agent_content: None,
            last_user_text: String::new(),
            last_assistant_text: String::new(),
            last_request_user_input_prompt: String::new(),
            confirm_notified_for_turn: false,
            interaction_required_for_turn: false,
            pending_request_user_input_call_ids: HashSet::new(),
            pending_request_user_input_without_id: 0,
            last_interaction_resolved_at: None,
            interaction_notified_for_turn: false,
            codex_task_protocol_seen: false,
        }
    }

    fn reset_for_new_turn(&mut self) {
        self.last_user_text.clear();
        self.last_assistant_text.clear();
        self.last_agent_content = None;
        self.confirm_notified_for_turn = false;
        self.interaction_required_for_turn = false;
        self.pending_request_user_input_call_ids.clear();
        self.pending_request_user_input_without_id = 0;
        self.last_interaction_resolved_at = None;
        self.interaction_notified_for_turn = false;
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
        if trimmed.ends_with('?') || trimmed.ends_with('?') {
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
            }
        }
    } else if let Some(question) = args_obj.get("question").and_then(|v| v.as_str()) {
        let q = ensure_question(question);
        if !q.is_empty() {
            lines.push(q);
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
                state.last_task_started_at = None;
                state.last_user_at = ts;
                state.last_user_text = extract_text_from_any(&Value::Object(payload.clone()));
                state.reset_for_new_turn();
                return;
            }

            // request_user_input
            if payload_type == Some("function_call")
                && payload.get("name").and_then(|v| v.as_str()) == Some("request_user_input")
            {
                state.interaction_required_for_turn = true;

                if let Some(call_id) = payload.get("call_id").and_then(|v| v.as_str()) {
                    if !call_id.trim().is_empty() {
                        state.pending_request_user_input_call_ids.insert(call_id.to_string());
                    } else {
                        state.pending_request_user_input_without_id += 1;
                    }
                }

                state.last_request_user_input_prompt = extract_request_user_input_text(&Value::Object(payload.clone()));
                return;
            }

            // function_call_output
            if payload_type == Some("function_call_output") {
                if let Some(call_id) = payload.get("call_id").and_then(|v| v.as_str()) {
                    state.pending_request_user_input_call_ids.remove(call_id);
                } else if state.pending_request_user_input_without_id > 0 {
                    state.pending_request_user_input_without_id -= 1;
                }

                if state.pending_request_user_input_call_ids.is_empty()
                    && state.pending_request_user_input_without_id == 0
                {
                    state.interaction_required_for_turn = false;
                    state.last_interaction_resolved_at = ts;
                }
                return;
            }

            // 工作类型 response，取消 pending
            if payload_type.map(|t| is_codex_work_type(t)).unwrap_or(false) {
                return;
            }

            // assistant message
            if payload_type == Some("message") && payload_role == Some("assistant") {
                let assistant_text = extract_text_from_any(&Value::Object(payload.clone()));
                if !assistant_text.is_empty() {
                    state.last_assistant_text = assistant_text.clone();
                    state.last_agent_content = Some(assistant_text);
                }

                state.last_assistant_at = ts.or_else(|| {
                    SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .ok()
                        .map(|d| d.as_millis() as i64)
                });
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
                    if let Some(turn_id) = payload.get("turn_id").and_then(|v| v.as_str()) {
                        state.current_turn_id = Some(turn_id.to_string());
                    }
                    if let Some(collab_kind) = payload.get("collaboration_mode_kind").and_then(|v| v.as_str()) {
                        state.collaboration_mode_kind = collab_kind.to_string();
                    }

                    state.last_task_started_at = ts;
                    state.reset_for_new_turn();
                    state.codex_task_protocol_seen = true;
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

                    // 检查是否需要交互
                    if state.interaction_required_for_turn {
                        // 交互模式下，尝试发送确认通知或跳过
                        let request_prompt = state.last_request_user_input_prompt.clone();
                        let request_has_options = has_options_in_prompt(&request_prompt);
                        let agent_content = state.last_agent_content.clone().unwrap_or_default();

                        // 优先发送带选项的确认
                        if request_has_options {
                            // 发送确认通知
                            let cwd = state.last_cwd.clone().unwrap_or_default();
                            let _ = crate::notify::send_notifications(
                                "codex",
                                &request_prompt,
                                None,
                                cwd,
                                false,
                                Some("confirm"),
                            );
                        } else {
                            // 检查尾部问题
                            let prompt = detect_turn_end_confirm_prompt(&agent_content, state.collaboration_mode_kind == "plan");
                            if let Some(p) = prompt {
                                // 发送确认通知
                                let cwd = state.last_cwd.clone().unwrap_or_default();
                                let _ = crate::notify::send_notifications(
                                    "codex",
                                    &p,
                                    None,
                                    cwd,
                                    false,
                                    Some("confirm"),
                                );
                            } else {
                                // 发送 fallback 确认
                                let cwd = state.last_cwd.clone().unwrap_or_default();
                                let _ = crate::notify::send_notifications(
                                    "codex",
                                    "需要你的确认",
                                    None,
                                    cwd,
                                    false,
                                    Some("confirm"),
                                );
                            }
                        }
                        return;
                    }

                    if state.confirm_notified_for_turn {
                        return;
                    }

                    let completion_at = ts.unwrap_or_else(|| {
                        SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as i64
                    });

                    if let Some(last_agent_msg) = payload.get("last_agent_message").and_then(|v| v.as_str()) {
                        state.last_assistant_text = last_agent_msg.to_string();
                        state.last_agent_content = Some(last_agent_msg.to_string());
                        state.last_assistant_at = Some(completion_at);
                    }

                    // 检查尾部问题（非交互模式）
                    let agent_content = state.last_agent_content.clone().unwrap_or_default();
                    let prompt = detect_turn_end_confirm_prompt(&agent_content, state.collaboration_mode_kind == "plan");
                    if let Some(_p) = prompt {
                        // 发送确认通知
                        let cwd = state.last_cwd.clone().unwrap_or_default();
                        let _ = crate::notify::send_notifications(
                            "codex",
                            &_p,
                            None,
                            cwd,
                            false,
                            Some("confirm"),
                        );
                        return;
                    }

                    // 发送完成通知
                    let start_at = state.last_user_at.or(state.last_task_started_at);
                    let duration_ms = start_at.map(|start| {
                        if completion_at >= start {
                            completion_at - start
                        } else {
                            0
                        }
                    });

                    let cwd = state.last_cwd.clone().unwrap_or_default();
                    let _ = crate::notify::send_notifications(
                        "codex",
                        "Codex 任务已完成",
                        duration_ms,
                        cwd,
                        false,
                        Some("complete"),
                    );

                    state.last_notified_assistant_at = Some(completion_at);
                    state.last_notified_turn_id = turn_id;
                    state.confirm_notified_for_turn = true;
                }

                Some("user_message") => {
                    state.last_task_started_at = None;
                    state.last_user_at = ts;
                    state.last_user_text = extract_text_from_any(obj.get("payload").unwrap_or(&Value::Null));
                    state.reset_for_new_turn();
                }

                Some("agent_message") => {
                    if seed {
                        return;
                    }

                    let payload = obj.get("payload").unwrap_or(&Value::Null);
                    let assistant_text = extract_text_from_any(payload);
                    if !assistant_text.is_empty() {
                        state.last_assistant_text = assistant_text;
                    }

                    state.last_assistant_at = ts.or_else(|| {
                        SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .ok()
                            .map(|d| d.as_millis() as i64)
                    });

                    // 提取 content
                    let content = payload
                        .get("content")
                        .and_then(|c| c.as_str())
                        .or_else(|| payload.get("message").and_then(|m| m.as_str()))
                        .or_else(|| payload.get("text").and_then(|t| t.as_str()))
                        .or_else(|| payload.get("data").and_then(|d| d.as_str()))
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
    notified_for_turn: bool,
    last_user_text: String,
    last_gemini_text: String,
    last_gemini_content: Option<String>,
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
            notified_for_turn: false,
            last_user_text: String::new(),
            last_gemini_text: String::new(),
            last_gemini_content: None,
        }
    }
}

fn process_gemini_message(
    msg: &Value,
    state: &mut GeminiState,
) {
    let ts = msg.get("timestamp").and_then(parse_timestamp);
    let msg_type = msg.get("type").and_then(|v| v.as_str());

    match msg_type {
        Some("user") => {
            state.last_user_at = ts;
            state.last_user_text = extract_text_from_any(msg);
            state.last_gemini_at = None;
            state.last_notified_gemini_at = None;
            state.last_gemini_text.clear();
            state.notified_for_turn = false;
        }
        Some("gemini") => {
            state.last_gemini_at = ts;

            let gemini_text = extract_text_from_any(msg);
            if !gemini_text.is_empty() {
                state.last_gemini_text = gemini_text;
            }

            // 提取 content（支持多种格式）
            let content = if let Some(content_arr) = msg.get("content").and_then(|c| c.as_array()) {
                content_arr
                    .iter()
                    .filter_map(|item| item.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n")
            } else if let Some(content_str) = msg.get("content").and_then(|c| c.as_str()) {
                content_str.to_string()
            } else if let Some(parts) = msg.get("parts").and_then(|p| p.as_array()) {
                parts
                    .iter()
                    .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            } else if let Some(text) = msg.get("text").and_then(|t| t.as_str()) {
                text.to_string()
            } else {
                String::new()
            };

            if !content.trim().is_empty() {
                state.last_gemini_content = Some(content);
            }
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
    let claude_quiet_ms = (claude_quiet_ms.max(500) as u64).max(5000);
    let gemini_quiet_ms = (gemini_quiet_ms.max(500) as u64).max(3000);

    tokio::spawn(async move {
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

                        // 处理种子内容
                        if let Ok(content) = fs::read_to_string(&latest_file) {
                            for line in content.lines() {
                                if let Some(obj) = safe_json_parse(line) {
                                    process_claude_object(&obj, true, &mut claude_state);
                                }
                            }
                        }
                    }

                    // 检查是否需要发送通知
                    if let (Some(user_at), Some(assistant_at)) = (claude_state.last_user_at, claude_state.last_assistant_at) {
                        if assistant_at >= user_at
                            && !claude_state.notified_for_turn
                            && !claude_state.confirm_notified_for_turn
                            && claude_state.last_notified_at != Some(assistant_at)
                        {
                            // 静默期检查
                            let now = SystemTime::now()
                                .duration_since(SystemTime::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as i64;

                            if now - assistant_at >= claude_quiet_ms as i64 {
                                let duration_ms = assistant_at - user_at;
                                let cwd = claude_state.last_cwd.clone().unwrap_or_default();

                                let _ = crate::notify::send_notifications(
                                    "claude",
                                    "Claude 任务已完成",
                                    Some(duration_ms),
                                    cwd,
                                    false,
                                    Some("complete"),
                                );

                                claude_state.last_notified_at = Some(assistant_at);
                                claude_state.notified_for_turn = true;
                                claude_state.confirm_notified_for_turn = true;

                                log_callback(format!("[watch][claude] notification sent ({}ms)", duration_ms));
                            }
                        }
                    }
                }
            }

            // Monitor Codex
            if sources.contains(&"codex") && codex_root.exists() {
                let follow_top_n = get_codex_follow_top_n();
                let latest = find_latest_files(&codex_root, |_, name| name.to_lowercase().ends_with(".jsonl"), follow_top_n);

                // 清理不再存在的会话
                codex_states.retain(|path, _| latest.contains(path));

                for file_path in latest {
                    if !codex_states.contains_key(&file_path) {
                        // 新会话
                        if let Ok(content) = fs::read_to_string(&file_path) {
                            let mut state = CodexSessionState::new(file_path.clone());

                            // 处理种子内容
                            for line in content.lines() {
                                if let Some(obj) = safe_json_parse(line) {
                                    process_codex_object(&obj, true, &mut state);
                                }
                            }

                            log_callback(format!("[watch][codex] following {:?}", file_path));
                            codex_states.insert(file_path.clone(), state);
                        }
                    }

                    // 处理新内容
                    if let Some(state) = codex_states.get_mut(&file_path) {
                        if let Ok(content) = fs::read_to_string(&file_path) {
                            for line in content.lines() {
                                if let Some(obj) = safe_json_parse(line) {
                                    process_codex_object(&obj, false, state);
                                }
                            }
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
                    full_path.to_string_lossy().contains("/chats/")
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
                        // 新文件
                        gemini_state.current_file = Some(latest_file.clone());
                        gemini_state.current_mtime_ms = mtime_ms;

                        if let Ok(content) = fs::read_to_string(&latest_file) {
                            if let Ok(json) = serde_json::from_str::<Value>(&content) {
                                if let Some(messages) = json.get("messages").and_then(|m| m.as_array()) {
                                    gemini_state.last_count = messages.len();

                                    for msg in messages {
                                        process_gemini_message(msg, &mut gemini_state);
                                    }

                                    gemini_state.last_notified_gemini_at = gemini_state.last_gemini_at;
                                    log_callback(format!("[watch][gemini] following {:?}", latest_file));
                                }
                            }
                        }
                        continue;
                    }

                    if mtime_ms <= gemini_state.current_mtime_ms {
                        continue;
                    }

                    // 文件已更新
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

                    // 处理新消息
                    for msg in &messages[gemini_state.last_count..] {
                        process_gemini_message(msg, &mut gemini_state);
                    }

                    gemini_state.current_mtime_ms = mtime_ms;
                    gemini_state.last_count = messages.len();

                    // 检查是否需要发送通知
                    if let (Some(user_at), Some(gemini_at)) = (gemini_state.last_user_at, gemini_state.last_gemini_at) {
                        if gemini_at >= user_at && !gemini_state.notified_for_turn {
                            let now = SystemTime::now()
                                .duration_since(SystemTime::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as i64;

                            if now - gemini_at >= gemini_quiet_ms as i64 {
                                let duration_ms = gemini_at - user_at;

                                let _ = crate::notify::send_notifications(
                                    "gemini",
                                    "Gemini 任务已完成",
                                    Some(duration_ms),
                                    String::new(),
                                    false,
                                    Some("complete"),
                                );

                                gemini_state.last_notified_gemini_at = Some(gemini_at);
                                gemini_state.notified_for_turn = true;

                                log_callback(format!("[watch][gemini] notification sent ({}ms)", duration_ms));
                            }
                        }
                    }
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
        assert!(detect_turn_end_confirm_prompt(text, false).is_some());

        let text = "Execute the command";
        assert!(detect_turn_end_confirm_prompt(text, false).is_none());
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
