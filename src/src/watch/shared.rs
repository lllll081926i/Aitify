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

fn is_confirm_alert_enabled() -> bool {
    std::env::var("WATCH_CONFIRM_ALERT_ENABLED")
        .ok()
        .map(|v| {
            let s = v.trim().to_ascii_lowercase();
            matches!(s.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

const CLAUDE_DIR: &str = ".claude/projects";
const CODEX_DIR: &str = ".codex/sessions";
const GEMINI_DIR: &str = ".gemini/tmp";
const QWEN_DIR: &str = ".qwen/projects";
const MAX_STATE_TEXT_CHARS: usize = 4096;
const MAX_OPENCODE_SEEN_MESSAGE_IDS: usize = 2048;

fn get_codex_follow_top_n() -> usize {
    std::env::var("CODEX_FOLLOW_TOP_N")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(5)
}

fn get_qwen_follow_top_n() -> usize {
    std::env::var("QWEN_FOLLOW_TOP_N")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(5)
}

fn get_opencode_scan_limit() -> usize {
    std::env::var("OPENCODE_SCAN_LIMIT")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(200)
        .max(20)
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

fn compact_state_text(text: &str) -> String {
    if text.chars().count() <= MAX_STATE_TEXT_CHARS {
        return text.to_string();
    }

    let total_chars = text.chars().count();
    text.chars()
        .skip(total_chars - MAX_STATE_TEXT_CHARS)
        .collect()
}

fn read_jsonl_objects_from_offset<F>(
    path: &Path,
    offset: u64,
    mut visit: F,
) -> std::io::Result<u64>
where
    F: FnMut(Value),
{
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;

    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut next_offset = offset;

    loop {
        line.clear();
        let line_start_offset = reader.stream_position()?;
        let bytes_read = reader.read_line(&mut line)?;
        if bytes_read == 0 {
            break;
        }

        if !line.ends_with('\n') {
            next_offset = line_start_offset;
            break;
        }

        if let Some(obj) = safe_json_parse(&line) {
            visit(obj);
        }

        next_offset = reader.stream_position()?;
    }

    Ok(next_offset)
}

fn normalize_processed_offset(file_size: u64, processed_offset: u64) -> u64 {
    if processed_offset > file_size {
        0
    } else {
        processed_offset
    }
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

fn unique_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();

    for path in paths {
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            result.push(path);
        }
    }

    result
}

fn get_opencode_data_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![home.join(".local/share/opencode")];

    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        if !local_app_data.trim().is_empty() {
            dirs.push(PathBuf::from(local_app_data).join("opencode"));
        }
    }

    if let Ok(app_data) = std::env::var("APPDATA") {
        if !app_data.trim().is_empty() {
            dirs.push(PathBuf::from(app_data).join("opencode"));
        }
    }

    if let Ok(xdg_data_home) = std::env::var("XDG_DATA_HOME") {
        if !xdg_data_home.trim().is_empty() {
            dirs.push(PathBuf::from(xdg_data_home).join("opencode"));
        }
    }

    unique_paths(dirs)
}

fn is_opencode_db_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.starts_with("opencode") && lower.ends_with(".db")
}

fn find_latest_opencode_db(home: &Path) -> Option<PathBuf> {
    let mut latest: Option<(PathBuf, u128)> = None;

    for dir in get_opencode_data_dirs(home) {
        let Ok(entries) = fs::read_dir(&dir) else { continue; };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue; };
            if !is_opencode_db_file(name) {
                continue;
            }

            let Some(mtime) = file_mtime_millis(&path) else { continue; };
            if latest.as_ref().map(|(_, ts)| mtime > *ts).unwrap_or(true) {
                latest = Some((path, mtime));
            }
        }
    }

    latest.map(|(path, _)| path)
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
        vec!["claude", "codex", "gemini", "qwen", "opencode"]
    } else {
        let mut result = Vec::new();
        for part in parts {
            match part {
                "claude" => result.push("claude"),
                "codex" => result.push("codex"),
                "gemini" => result.push("gemini"),
                "qwen" => result.push("qwen"),
                "opencode" => result.push("opencode"),
                _ => {}
            }
        }
        if result.is_empty() {
            vec!["claude", "codex", "gemini", "qwen", "opencode"]
        } else {
            result
        }
    }
}

