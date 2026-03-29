// ============ OpenCode Watch ============

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpencodeCompletion {
    session_id: String,
    message_id: String,
    cwd: String,
    completed_at: i64,
    duration_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpencodeNotification {
    session_id: String,
    message_id: String,
    cwd: String,
    completed_at: i64,
    duration_ms: Option<i64>,
    notification_type: &'static str,
    task_info: String,
}

struct OpencodeMessageRow {
    message_id: String,
    session_id: String,
    session_parent_id: Option<String>,
    directory: String,
    time_updated: i64,
    data: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct OpencodeScanCursor {
    updated_at: i64,
    message_id: Option<String>,
}

struct OpencodeState {
    current_db: Option<PathBuf>,
    last_scan_cursor: OpencodeScanCursor,
    seen_message_ids: HashSet<String>,
    seen_message_order: VecDeque<String>,
}

impl OpencodeState {
    fn new() -> Self {
        Self {
            current_db: None,
            last_scan_cursor: OpencodeScanCursor::default(),
            seen_message_ids: HashSet::new(),
            seen_message_order: VecDeque::new(),
        }
    }

    fn seed_from_now(&mut self, db_path: PathBuf) {
        self.current_db = Some(db_path);
        self.last_scan_cursor = OpencodeScanCursor {
            updated_at: now_unix_millis_i64(),
            message_id: None,
        };
        self.seen_message_ids.clear();
        self.seen_message_order.clear();
    }
}

fn remember_seen_message_id(
    seen_message_ids: &mut HashSet<String>,
    seen_message_order: &mut VecDeque<String>,
    message_id: String,
) -> bool {
    if !seen_message_ids.insert(message_id.clone()) {
        return false;
    }

    seen_message_order.push_back(message_id);

    while seen_message_order.len() > MAX_OPENCODE_SEEN_MESSAGE_IDS {
        if let Some(expired) = seen_message_order.pop_front() {
            seen_message_ids.remove(&expired);
        }
    }

    true
}

fn next_opencode_scan_cursor(
    previous_cursor: &OpencodeScanCursor,
    last_seen_updated_at: i64,
    last_seen_message_id: Option<&str>,
) -> OpencodeScanCursor {
    let same_timestamp_newer_id = last_seen_updated_at == previous_cursor.updated_at
        && match (last_seen_message_id, previous_cursor.message_id.as_deref()) {
            (Some(next), Some(previous)) => next > previous,
            (Some(_), None) => true,
            _ => false,
        };

    if last_seen_updated_at > previous_cursor.updated_at || same_timestamp_newer_id {
        OpencodeScanCursor {
            updated_at: last_seen_updated_at,
            message_id: last_seen_message_id.map(|value| value.to_string()),
        }
    } else {
        previous_cursor.clone()
    }
}

fn open_opencode_connection(db_path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

fn query_opencode_recent_messages(
    conn: &Connection,
    cursor: &OpencodeScanCursor,
    limit: usize,
) -> rusqlite::Result<Vec<OpencodeMessageRow>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.session_id, s.parent_id, m.time_updated, m.data, s.directory
         FROM message m
         INNER JOIN session s ON s.id = m.session_id
         WHERE m.time_updated > ?1
            OR (m.time_updated = ?1 AND ?2 IS NOT NULL AND m.id > ?2)
         ORDER BY m.time_updated ASC, m.id ASC
         LIMIT ?3",
    )?;

    let rows = stmt.query_map(params![cursor.updated_at, cursor.message_id.as_deref(), limit as i64], |row| {
        Ok(OpencodeMessageRow {
            message_id: row.get(0)?,
            session_id: row.get(1)?,
            session_parent_id: row.get(2)?,
            time_updated: row.get(3)?,
            data: row.get(4)?,
            directory: row.get(5)?,
        })
    })?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row?);
    }
    Ok(result)
}

fn query_opencode_user_created_at(conn: &Connection, message_id: &str) -> Option<i64> {
    let mut stmt = conn
        .prepare("SELECT data, time_created FROM message WHERE id = ?1 LIMIT 1")
        .ok()?;
    let row = stmt
        .query_row(params![message_id], |row| {
            let data: String = row.get(0)?;
            let time_created: i64 = row.get(1)?;
            Ok((data, time_created))
        })
        .ok()?;

    let parsed = serde_json::from_str::<Value>(&row.0).ok();
    parsed
        .as_ref()
        .and_then(|value| value.get("time"))
        .and_then(|value| value.get("created"))
        .and_then(parse_timestamp)
        .or(Some(row.1))
}

fn query_opencode_message_text(conn: &Connection, message_id: &str) -> Option<String> {
    let mut stmt = conn
        .prepare(
            "SELECT data
             FROM part
             WHERE message_id = ?1
             ORDER BY time_created ASC, id ASC",
        )
        .ok()?;
    let rows = stmt
        .query_map(params![message_id], |row| row.get::<_, String>(0))
        .ok()?;

    let mut parts = Vec::new();
    for row in rows {
        let Ok(raw) = row else { continue; };
        let Ok(part) = serde_json::from_str::<Value>(&raw) else { continue; };
        if part.get("type").and_then(|value| value.as_str()) != Some("text") {
            continue;
        }

        let text = compact_state_text(&extract_text_from_any(&part));
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(compact_state_text(&parts.join("\n\n")))
    }
}

fn extract_opencode_completion(
    session_id: &str,
    message_id: &str,
    directory: &str,
    message: &Value,
    user_created_at: Option<i64>,
) -> Option<OpencodeCompletion> {
    if message.get("role").and_then(|v| v.as_str()) != Some("assistant") {
        return None;
    }

    if message.get("error").is_some() {
        return None;
    }

    if message.get("finish").and_then(|value| value.as_str()) != Some("stop") {
        return None;
    }

    let completed_at = message
        .get("time")
        .and_then(|value| value.get("completed"))
        .and_then(parse_timestamp)?;

    let cwd = message
        .get("path")
        .and_then(|value| value.get("cwd"))
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(directory)
        .to_string();

    let duration_ms = user_created_at.and_then(|start| {
        if completed_at >= start {
            Some(completed_at - start)
        } else {
            None
        }
    });

    Some(OpencodeCompletion {
        session_id: session_id.to_string(),
        message_id: message_id.to_string(),
        cwd,
        completed_at,
        duration_ms,
    })
}

fn extract_opencode_notification(
    session_id: &str,
    session_parent_id: Option<&str>,
    message_id: &str,
    directory: &str,
    message: &Value,
    message_text: Option<&str>,
    user_created_at: Option<i64>,
) -> Option<OpencodeNotification> {
    if session_parent_id.is_some() {
        return None;
    }

    let completion = extract_opencode_completion(
        session_id,
        message_id,
        directory,
        message,
        user_created_at,
    )?;

    let prompt = message_text
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(detect_turn_end_confirm_prompt);

    let (notification_type, task_info) = match prompt {
        Some(prompt) => ("confirm", prompt),
        None => ("complete", "OpenCode 任务已完成".to_string()),
    };

    Some(OpencodeNotification {
        session_id: completion.session_id,
        message_id: completion.message_id,
        cwd: completion.cwd,
        completed_at: completion.completed_at,
        duration_ms: completion.duration_ms,
        notification_type,
        task_info,
    })
}

#[cfg(test)]
fn collect_opencode_completions(
    db_path: &Path,
    cursor: &OpencodeScanCursor,
    limit: usize,
) -> rusqlite::Result<(Vec<OpencodeCompletion>, OpencodeScanCursor)> {
    let conn = open_opencode_connection(db_path)?;
    let rows = query_opencode_recent_messages(&conn, cursor, limit)?;
    let mut completions = Vec::new();
    let mut last_seen_cursor = cursor.clone();

    for row in rows {
        last_seen_cursor = next_opencode_scan_cursor(
            &last_seen_cursor,
            row.time_updated,
            Some(&row.message_id),
        );

        let Ok(message) = serde_json::from_str::<Value>(&row.data) else { continue; };
        let parent_id = message.get("parentID").and_then(|value| value.as_str());
        let user_created_at = parent_id.and_then(|id| query_opencode_user_created_at(&conn, id));

        if let Some(completion) = extract_opencode_completion(
            &row.session_id,
            &row.message_id,
            &row.directory,
            &message,
            user_created_at,
        ) {
            completions.push(completion);
        }
    }

    Ok((completions, last_seen_cursor))
}

fn collect_opencode_notifications(
    db_path: &Path,
    cursor: &OpencodeScanCursor,
    limit: usize,
) -> rusqlite::Result<(Vec<OpencodeNotification>, OpencodeScanCursor)> {
    let conn = open_opencode_connection(db_path)?;
    let rows = query_opencode_recent_messages(&conn, cursor, limit)?;
    let mut notifications = Vec::new();
    let mut last_seen_cursor = cursor.clone();

    for row in rows {
        last_seen_cursor = next_opencode_scan_cursor(
            &last_seen_cursor,
            row.time_updated,
            Some(&row.message_id),
        );

        let Ok(message) = serde_json::from_str::<Value>(&row.data) else { continue; };
        let parent_id = message.get("parentID").and_then(|value| value.as_str());
        let user_created_at = parent_id.and_then(|id| query_opencode_user_created_at(&conn, id));
        let message_text = query_opencode_message_text(&conn, &row.message_id);

        if let Some(notification) = extract_opencode_notification(
            &row.session_id,
            row.session_parent_id.as_deref(),
            &row.message_id,
            &row.directory,
            &message,
            message_text.as_deref(),
            user_created_at,
        ) {
            notifications.push(notification);
        }
    }

    Ok((notifications, last_seen_cursor))
}

#[cfg(test)]
fn poll_opencode_completions(
    state: &mut OpencodeState,
    db_path: &Path,
    limit: usize,
) -> rusqlite::Result<Vec<OpencodeCompletion>> {
    if state.current_db.as_deref() != Some(db_path) {
        state.seed_from_now(db_path.to_path_buf());
        return Ok(Vec::new());
    }

    let (completions, next_cursor) = collect_opencode_completions(db_path, &state.last_scan_cursor, limit)?;
    state.last_scan_cursor = next_cursor;

    let mut new_completions = Vec::new();
    for completion in completions {
        if remember_seen_message_id(
            &mut state.seen_message_ids,
            &mut state.seen_message_order,
            completion.message_id.clone(),
        ) {
            new_completions.push(completion);
        }
    }

    Ok(new_completions)
}

fn poll_opencode_notifications(
    state: &mut OpencodeState,
    db_path: &Path,
    limit: usize,
) -> rusqlite::Result<Vec<OpencodeNotification>> {
    if state.current_db.as_deref() != Some(db_path) {
        state.seed_from_now(db_path.to_path_buf());
        return Ok(Vec::new());
    }

    let (notifications, next_cursor) = collect_opencode_notifications(db_path, &state.last_scan_cursor, limit)?;
    state.last_scan_cursor = next_cursor;

    let mut new_notifications = Vec::new();
    for notification in notifications {
        if remember_seen_message_id(
            &mut state.seen_message_ids,
            &mut state.seen_message_order,
            notification.message_id.clone(),
        ) {
            new_notifications.push(notification);
        }
    }

    Ok(new_notifications)
}

