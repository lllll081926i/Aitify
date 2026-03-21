// ============ OpenCode Watch ============

#[derive(Debug, Clone, PartialEq, Eq)]
struct OpencodeCompletion {
    session_id: String,
    message_id: String,
    cwd: String,
    completed_at: i64,
    duration_ms: Option<i64>,
}

struct OpencodeMessageRow {
    message_id: String,
    session_id: String,
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
    current_mtime_ms: u64,
    last_scan_cursor: OpencodeScanCursor,
    seen_message_ids: HashSet<String>,
    seen_message_order: VecDeque<String>,
}

impl OpencodeState {
    fn new() -> Self {
        Self {
            current_db: None,
            current_mtime_ms: 0,
            last_scan_cursor: OpencodeScanCursor::default(),
            seen_message_ids: HashSet::new(),
            seen_message_order: VecDeque::new(),
        }
    }

    fn seed_from_now(&mut self, db_path: PathBuf, mtime_ms: u64) {
        self.current_db = Some(db_path);
        self.current_mtime_ms = mtime_ms;
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
        "SELECT m.id, m.session_id, m.time_updated, m.data, s.directory
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
            time_updated: row.get(2)?,
            data: row.get(3)?,
            directory: row.get(4)?,
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

