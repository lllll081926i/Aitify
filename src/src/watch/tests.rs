#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::collections::HashSet;
    use std::fs;

    #[test]
    fn test_normalize_sources() {
        assert_eq!(normalize_sources("all"), vec!["claude", "codex", "pi", "opencode"]);
        assert_eq!(normalize_sources(""), vec!["claude", "codex", "pi", "opencode"]);
        assert_eq!(normalize_sources("claude"), vec!["claude"]);
        assert_eq!(normalize_sources("claude,codex"), vec!["claude", "codex"]);
        assert_eq!(normalize_sources("pi"), vec!["pi"]);
        assert_eq!(normalize_sources("opencode"), vec!["opencode"]);
    }

    #[test]
    fn test_extract_opencode_completion() {
        let assistant = serde_json::json!({
            "id": "msg-assistant-1",
            "sessionID": "session-1",
            "role": "assistant",
            "time": {
                "created": 1704067260000i64,
                "completed": 1704067320000i64
            },
            "parentID": "msg-user-1",
            "finish": "stop",
            "path": {
                "cwd": "D:/Code/Aitify",
                "root": "D:/Code/Aitify"
            }
        });

        let completion = extract_opencode_completion(
            "session-1",
            "msg-assistant-1",
            "D:/Code/Aitify",
            &assistant,
            Some(1704067200000i64),
        )
        .expect("assistant completion should be parsed");

        assert_eq!(completion.session_id, "session-1");
        assert_eq!(completion.message_id, "msg-assistant-1");
        assert_eq!(completion.cwd, "D:/Code/Aitify");
        assert_eq!(completion.completed_at, 1704067320000i64);
        assert_eq!(completion.duration_ms, Some(120000i64));
    }

    #[test]
    fn test_extract_opencode_completion_ignores_unfinished_assistant() {
        let assistant = serde_json::json!({
            "id": "msg-assistant-1",
            "sessionID": "session-1",
            "role": "assistant",
            "time": {
                "created": 1704067260000i64
            },
            "parentID": "msg-user-1"
        });

        assert!(extract_opencode_completion(
            "session-1",
            "msg-assistant-1",
            "D:/Code/Aitify",
            &assistant,
            Some(1704067200000i64),
        )
        .is_none());
    }

    #[test]
    fn test_extract_opencode_completion_ignores_tool_call_turns() {
        let assistant = serde_json::json!({
            "id": "msg-assistant-tool",
            "sessionID": "session-1",
            "role": "assistant",
            "time": {
                "created": 1704067260000i64,
                "completed": 1704067320000i64
            },
            "parentID": "msg-user-1",
            "finish": "tool-calls",
            "path": {
                "cwd": "D:/Code/Aitify"
            }
        });

        assert!(extract_opencode_completion(
            "session-1",
            "msg-assistant-tool",
            "D:/Code/Aitify",
            &assistant,
            Some(1704067200000i64),
        )
        .is_none());
    }

    #[test]
    fn test_extract_opencode_completion_ignores_aborted_assistant() {
        let assistant = serde_json::json!({
            "id": "msg-assistant-error",
            "sessionID": "session-1",
            "role": "assistant",
            "time": {
                "created": 1704067260000i64,
                "completed": 1704067320000i64
            },
            "parentID": "msg-user-1",
            "error": {
                "name": "MessageAbortedError",
                "data": { "message": "The operation was aborted." }
            },
            "path": {
                "cwd": "D:/Code/Aitify"
            }
        });

        assert!(extract_opencode_completion(
            "session-1",
            "msg-assistant-error",
            "D:/Code/Aitify",
            &assistant,
            Some(1704067200000i64),
        )
        .is_none());
    }

    #[test]
    fn test_extract_opencode_notification_ignores_subagent_session_completion() {
        let assistant = serde_json::json!({
            "id": "msg-assistant-subagent",
            "sessionID": "session-child",
            "role": "assistant",
            "time": {
                "created": 1704067260000i64,
                "completed": 1704067320000i64
            },
            "parentID": "msg-user-1",
            "finish": "stop",
            "path": {
                "cwd": "D:/Code/Aitify"
            }
        });

        assert!(extract_opencode_notification(
            "session-child",
            Some("session-parent"),
            "msg-assistant-subagent",
            "D:/Code/Aitify",
            &assistant,
            Some("子 agent 已完成修复"),
            Some(1704067200000i64),
        )
        .is_none());
    }

    #[test]
    fn test_extract_opencode_notification_marks_question_as_confirm() {
        let assistant = serde_json::json!({
            "id": "msg-assistant-confirm",
            "sessionID": "session-1",
            "role": "assistant",
            "time": {
                "created": 1704067260000i64,
                "completed": 1704067320000i64
            },
            "parentID": "msg-user-1",
            "finish": "stop",
            "path": {
                "cwd": "D:/Code/Aitify"
            }
        });

        let notification = extract_opencode_notification(
            "session-1",
            None,
            "msg-assistant-confirm",
            "D:/Code/Aitify",
            &assistant,
            Some("请确认是否继续执行？"),
            Some(1704067200000i64),
        )
        .expect("assistant confirm prompt should be parsed");

        assert_eq!(notification.session_id, "session-1");
        assert_eq!(notification.message_id, "msg-assistant-confirm");
        assert_eq!(notification.cwd, "D:/Code/Aitify");
        assert_eq!(notification.duration_ms, Some(120000i64));
        assert_eq!(notification.notification_type, "confirm");
        assert_eq!(notification.task_info, "请确认是否继续执行？");
    }

    #[test]
    fn test_next_opencode_scan_cursor_does_not_jump_past_seen_data() {
        let previous = OpencodeScanCursor {
            updated_at: 100,
            message_id: Some("msg-2".to_string()),
        };

        let advanced = next_opencode_scan_cursor(&previous, 100, Some("msg-3"));
        assert_eq!(advanced.updated_at, 100);
        assert_eq!(advanced.message_id.as_deref(), Some("msg-3"));

        let unchanged = next_opencode_scan_cursor(&advanced, 100, Some("msg-1"));
        assert_eq!(unchanged.updated_at, 100);
        assert_eq!(unchanged.message_id.as_deref(), Some("msg-3"));
    }

    #[test]
    fn test_collect_opencode_completions_keeps_same_timestamp_tail_across_pages() {
        let temp_dir = std::env::temp_dir().join(format!("aitify-opencode-db-{}", now_unix_millis_i64()));
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let db_path = temp_dir.join("opencode-test.db");
        let conn = Connection::open(&db_path).expect("db should open");

        conn.execute_batch(
            "
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                parent_id TEXT,
                directory TEXT NOT NULL
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                data TEXT NOT NULL
            );
            INSERT INTO session (id, parent_id, directory) VALUES ('session-1', NULL, 'D:/Code/Aitify');
            ",
        )
        .expect("schema should be created");

        for index in 1..=3 {
            let user_id = format!("user-{index}");
            let assistant_id = format!("assistant-{index}");
            let user_data = serde_json::json!({
                "id": user_id,
                "role": "user",
                "time": { "created": 1_704_067_200_000i64 + index as i64 }
            })
            .to_string();
            let assistant_data = serde_json::json!({
                "id": assistant_id,
                "role": "assistant",
                "parentID": format!("user-{index}"),
                "time": {
                    "created": 1_704_067_260_000i64 + index as i64,
                    "completed": 1_704_067_320_000i64 + index as i64
                },
                "finish": "stop",
                "path": { "cwd": "D:/Code/Aitify" }
            })
            .to_string();

            conn.execute(
                "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![user_id, "session-1", 10 + index as i64, 0, user_data],
            )
            .expect("user row should insert");
            conn.execute(
                "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![assistant_id, "session-1", 20 + index as i64, 100, assistant_data],
            )
            .expect("assistant row should insert");
        }

        drop(conn);

        let mut cursor = OpencodeScanCursor::default();
        let mut completions = Vec::new();

        let (first_page, next_cursor) = collect_opencode_completions(&db_path, &cursor, 2)
            .expect("first page should load");
        completions.extend(first_page.into_iter().map(|completion| completion.message_id));
        cursor = next_cursor;

        let (second_page, next_cursor) = collect_opencode_completions(&db_path, &cursor, 2)
            .expect("second page should load");
        completions.extend(second_page.into_iter().map(|completion| completion.message_id));
        cursor = next_cursor;

        assert_eq!(
            completions,
            vec![
                "assistant-1".to_string(),
                "assistant-2".to_string(),
                "assistant-3".to_string(),
            ]
        );
        assert_eq!(cursor.updated_at, 100);
        assert_eq!(cursor.message_id.as_deref(), Some("assistant-3"));

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&temp_dir);
    }

    #[test]
    fn test_poll_opencode_completions_keeps_detecting_new_rows_across_polls() {
        let temp_dir = std::env::temp_dir().join(format!("aitify-opencode-poll-{}", now_unix_millis_i64()));
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let db_path = temp_dir.join("opencode-test.db");
        let conn = Connection::open(&db_path).expect("db should open");

        conn.execute_batch(
            "
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                parent_id TEXT,
                directory TEXT NOT NULL
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                data TEXT NOT NULL
            );
            INSERT INTO session (id, parent_id, directory) VALUES ('session-1', NULL, 'D:/Code/Aitify');
            ",
        )
        .expect("schema should be created");

        let first_user = serde_json::json!({
            "id": "user-1",
            "role": "user",
            "time": { "created": 1_704_067_200_000i64 }
        })
        .to_string();
        let first_assistant = serde_json::json!({
            "id": "assistant-1",
            "role": "assistant",
            "parentID": "user-1",
            "time": {
                "created": 1_704_067_260_000i64,
                "completed": 1_704_067_320_000i64
            },
            "finish": "stop",
            "path": { "cwd": "D:/Code/Aitify" }
        })
        .to_string();

        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["user-1", "session-1", 10i64, 100i64, first_user],
        )
        .expect("first user row should insert");
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["assistant-1", "session-1", 20i64, 110i64, first_assistant],
        )
        .expect("first assistant row should insert");
        drop(conn);

        let mut state = OpencodeState::new();
        state.current_db = Some(db_path.clone());

        let first_poll = poll_opencode_completions(&mut state, &db_path, 50)
            .expect("first poll should succeed");
        assert_eq!(
            first_poll.into_iter().map(|completion| completion.message_id).collect::<Vec<_>>(),
            vec!["assistant-1".to_string()]
        );

        let conn = Connection::open(&db_path).expect("db should reopen");
        let second_user = serde_json::json!({
            "id": "user-2",
            "role": "user",
            "time": { "created": 1_704_067_400_000i64 }
        })
        .to_string();
        let second_assistant = serde_json::json!({
            "id": "assistant-2",
            "role": "assistant",
            "parentID": "user-2",
            "time": {
                "created": 1_704_067_460_000i64,
                "completed": 1_704_067_520_000i64
            },
            "finish": "stop",
            "path": { "cwd": "D:/Code/Aitify" }
        })
        .to_string();

        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["user-2", "session-1", 30i64, 200i64, second_user],
        )
        .expect("second user row should insert");
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["assistant-2", "session-1", 40i64, 210i64, second_assistant],
        )
        .expect("second assistant row should insert");
        drop(conn);

        let second_poll = poll_opencode_completions(&mut state, &db_path, 50)
            .expect("second poll should succeed");
        assert_eq!(
            second_poll.into_iter().map(|completion| completion.message_id).collect::<Vec<_>>(),
            vec!["assistant-2".to_string()]
        );

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&temp_dir);
    }

    #[test]
    fn test_collect_opencode_completions_only_keeps_terminal_stop_message() {
        let temp_dir = std::env::temp_dir().join(format!("aitify-opencode-stop-{}", now_unix_millis_i64()));
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let db_path = temp_dir.join("opencode-test.db");
        let conn = Connection::open(&db_path).expect("db should open");

        conn.execute_batch(
            "
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                parent_id TEXT,
                directory TEXT NOT NULL
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                data TEXT NOT NULL
            );
            INSERT INTO session (id, parent_id, directory) VALUES ('session-1', NULL, 'D:/Code/Aitify');
            ",
        )
        .expect("schema should be created");

        let user_data = serde_json::json!({
            "id": "user-1",
            "role": "user",
            "time": { "created": 1_704_067_200_000i64 }
        })
        .to_string();
        let tool_call_data = serde_json::json!({
            "id": "assistant-tool",
            "role": "assistant",
            "parentID": "user-1",
            "time": {
                "created": 1_704_067_260_000i64,
                "completed": 1_704_067_280_000i64
            },
            "finish": "tool-calls",
            "path": { "cwd": "D:/Code/Aitify" }
        })
        .to_string();
        let stop_data = serde_json::json!({
            "id": "assistant-stop",
            "role": "assistant",
            "parentID": "user-1",
            "time": {
                "created": 1_704_067_281_000i64,
                "completed": 1_704_067_320_000i64
            },
            "finish": "stop",
            "path": { "cwd": "D:/Code/Aitify" }
        })
        .to_string();

        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["user-1", "session-1", 10i64, 10i64, user_data],
        )
        .expect("user row should insert");
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["assistant-tool", "session-1", 20i64, 20i64, tool_call_data],
        )
        .expect("tool call row should insert");
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["assistant-stop", "session-1", 30i64, 30i64, stop_data],
        )
        .expect("stop row should insert");
        drop(conn);

        let (completions, cursor) = collect_opencode_completions(&db_path, &OpencodeScanCursor::default(), 50)
            .expect("completions should load");

        assert_eq!(
            completions
                .into_iter()
                .map(|completion| completion.message_id)
                .collect::<Vec<_>>(),
            vec!["assistant-stop".to_string()]
        );
        assert_eq!(cursor.updated_at, 30);
        assert_eq!(cursor.message_id.as_deref(), Some("assistant-stop"));

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&temp_dir);
    }

    #[test]
    fn test_collect_opencode_notifications_ignores_subagent_and_keeps_confirm_prompt() {
        let temp_dir = std::env::temp_dir().join(format!("aitify-opencode-notify-{}", now_unix_millis_i64()));
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let db_path = temp_dir.join("opencode-test.db");
        let conn = Connection::open(&db_path).expect("db should open");

        conn.execute_batch(
            "
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                project_id TEXT NOT NULL,
                parent_id TEXT,
                slug TEXT NOT NULL,
                directory TEXT NOT NULL,
                title TEXT NOT NULL,
                version TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                data TEXT NOT NULL
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL,
                data TEXT NOT NULL
            );
            INSERT INTO session (id, project_id, parent_id, slug, directory, title, version, time_created, time_updated)
            VALUES
                ('session-top', 'project-1', NULL, 'top', 'D:/Code/Aitify', 'Top', '1.0.0', 1, 1),
                ('session-child', 'project-1', 'session-top', 'child', 'D:/Code/Aitify', 'Child', '1.0.0', 2, 2);
            ",
        )
        .expect("schema should be created");

        let top_user = serde_json::json!({
            "id": "user-top",
            "role": "user",
            "time": { "created": 1_704_067_200_000i64 }
        })
        .to_string();
        let top_assistant = serde_json::json!({
            "id": "assistant-top",
            "role": "assistant",
            "parentID": "user-top",
            "time": {
                "created": 1_704_067_260_000i64,
                "completed": 1_704_067_320_000i64
            },
            "finish": "stop",
            "path": { "cwd": "D:/Code/Aitify" }
        })
        .to_string();
        let child_user = serde_json::json!({
            "id": "user-child",
            "role": "user",
            "time": { "created": 1_704_067_400_000i64 }
        })
        .to_string();
        let child_assistant = serde_json::json!({
            "id": "assistant-child",
            "role": "assistant",
            "parentID": "user-child",
            "time": {
                "created": 1_704_067_460_000i64,
                "completed": 1_704_067_520_000i64
            },
            "finish": "stop",
            "path": { "cwd": "D:/Code/Aitify" }
        })
        .to_string();

        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["user-top", "session-top", 10i64, 10i64, top_user],
        )
        .expect("top user row should insert");
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["assistant-top", "session-top", 20i64, 20i64, top_assistant],
        )
        .expect("top assistant row should insert");
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["user-child", "session-child", 30i64, 30i64, child_user],
        )
        .expect("child user row should insert");
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["assistant-child", "session-child", 40i64, 40i64, child_assistant],
        )
        .expect("child assistant row should insert");

        let confirm_part = serde_json::json!({
            "type": "text",
            "text": "请确认是否继续执行？"
        })
        .to_string();
        let child_part = serde_json::json!({
            "type": "text",
            "text": "子 agent 已完成修复"
        })
        .to_string();

        conn.execute(
            "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params!["part-top", "assistant-top", "session-top", 21i64, 21i64, confirm_part],
        )
        .expect("top part row should insert");
        conn.execute(
            "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params!["part-child", "assistant-child", "session-child", 41i64, 41i64, child_part],
        )
        .expect("child part row should insert");
        drop(conn);

        let (notifications, cursor) = collect_opencode_notifications(&db_path, &OpencodeScanCursor::default(), 50)
            .expect("notifications should load");

        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].message_id, "assistant-top");
        assert_eq!(notifications[0].notification_type, "confirm");
        assert_eq!(notifications[0].task_info, "请确认是否继续执行？");
        assert_eq!(cursor.updated_at, 40);
        assert_eq!(cursor.message_id.as_deref(), Some("assistant-child"));

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_dir(&temp_dir);
    }

    #[test]
    fn test_compact_state_text_keeps_tail_with_limit() {
        let input = "0123456789".repeat(700);
        let output = compact_state_text(&input);

        assert!(output.len() <= MAX_STATE_TEXT_CHARS);
        assert_eq!(output, input[input.len() - output.len()..].to_string());
    }

    #[test]
    fn test_read_jsonl_objects_from_offset_reads_only_new_records() {
        let temp_dir = std::env::temp_dir().join(format!("aitify-watch-test-{}", now_unix_millis_i64()));
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let file_path = temp_dir.join("sample.jsonl");
        let initial = "{\"type\":\"user\",\"message\":\"a\"}\n";
        fs::write(&file_path, initial).expect("initial file should be written");
        let initial_len = fs::metadata(&file_path).expect("metadata should exist").len();

        let appended = "{\"type\":\"assistant\",\"message\":\"b\"}\n{\"type\":\"assistant\",\"message\":\"c\"}\n";
        let mut content = String::from(initial);
        content.push_str(appended);
        fs::write(&file_path, content).expect("appended file should be written");

        let mut messages = Vec::new();
        let final_offset = read_jsonl_objects_from_offset(&file_path, initial_len, |obj: Value| {
            messages.push(
                obj.get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
            );
        })
        .expect("jsonl read should succeed");

        assert_eq!(messages, vec!["b".to_string(), "c".to_string()]);
        assert_eq!(final_offset, fs::metadata(&file_path).expect("metadata should exist").len());

        let _ = fs::remove_file(&file_path);
        let _ = fs::remove_dir(&temp_dir);
    }

    #[test]
    fn test_read_jsonl_objects_from_offset_keeps_partial_line_for_next_poll() {
        let temp_dir = std::env::temp_dir().join(format!("aitify-watch-partial-{}", now_unix_millis_i64()));
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let file_path = temp_dir.join("sample.jsonl");
        let first_line = "{\"type\":\"user\",\"message\":\"a\"}\n";
        let partial_line = "{\"type\":\"assistant\",\"message\":\"b\"";
        fs::write(&file_path, format!("{first_line}{partial_line}")).expect("partial file should be written");

        let mut messages = Vec::new();
        let offset_after_first_poll = read_jsonl_objects_from_offset(&file_path, 0, |obj: Value| {
            messages.push(
                obj.get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
            );
        })
        .expect("jsonl read should succeed");

        assert_eq!(messages, vec!["a".to_string()]);
        assert_eq!(offset_after_first_poll, first_line.len() as u64);

        fs::write(&file_path, format!("{first_line}{partial_line}}}\n")).expect("completed file should be written");

        let mut next_messages = Vec::new();
        let final_offset = read_jsonl_objects_from_offset(&file_path, offset_after_first_poll, |obj: Value| {
            next_messages.push(
                obj.get("message")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
            );
        })
        .expect("jsonl second read should succeed");

        assert_eq!(next_messages, vec!["b".to_string()]);
        assert_eq!(final_offset, fs::metadata(&file_path).expect("metadata should exist").len());

        let _ = fs::remove_file(&file_path);
        let _ = fs::remove_dir(&temp_dir);
    }

    #[test]
    fn test_normalize_processed_offset_resets_to_zero_when_file_shrinks() {
        assert_eq!(normalize_processed_offset(128, 64), 64);
        assert_eq!(normalize_processed_offset(64, 128), 0);
    }

    #[test]
    fn test_remember_seen_message_id_prunes_old_entries() {
        let mut seen = HashSet::new();
        let mut order = std::collections::VecDeque::new();

        for index in 0..(MAX_OPENCODE_SEEN_MESSAGE_IDS + 5) {
            let inserted = remember_seen_message_id(&mut seen, &mut order, format!("msg-{index}"));
            assert!(inserted);
        }

        assert_eq!(seen.len(), MAX_OPENCODE_SEEN_MESSAGE_IDS);
        assert_eq!(order.len(), MAX_OPENCODE_SEEN_MESSAGE_IDS);
        assert!(!seen.contains("msg-0"));
        assert!(seen.contains(&format!("msg-{}", MAX_OPENCODE_SEEN_MESSAGE_IDS + 4)));
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
    fn test_is_claude_agent_progress_detects_subagent_progress_event() {
        let obj = serde_json::json!({
            "type": "progress",
            "data": {
                "type": "agent_progress",
                "agentId": "agent-1",
                "message": {
                    "type": "assistant"
                }
            }
        });

        assert!(is_claude_agent_progress(&obj));

        let unrelated_progress = serde_json::json!({
            "type": "progress",
            "data": {
                "type": "thinking"
            }
        });

        assert!(!is_claude_agent_progress(&unrelated_progress));
    }

    #[test]
    fn test_process_claude_agent_progress_suppresses_completion_until_top_level_assistant_returns() {
        let mut state = ClaudeState::new();

        let user = serde_json::json!({
            "type": "user",
            "timestamp": "2024-01-01T00:00:00Z",
            "cwd": "D:/Code/Aitify",
            "message": {
                "role": "user",
                "content": "检查这里的 bug"
            }
        });
        process_claude_object(&user, false, &mut state);

        let assistant_tool_use = serde_json::json!({
            "type": "assistant",
            "timestamp": "2024-01-01T00:00:10Z",
            "cwd": "D:/Code/Aitify",
            "message": {
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "name": "Task"
                }]
            }
        });
        process_claude_object(&assistant_tool_use, false, &mut state);

        let subagent_progress = serde_json::json!({
            "type": "progress",
            "timestamp": "2024-01-01T00:00:20Z",
            "cwd": "D:/Code/Aitify",
            "data": {
                "type": "agent_progress",
                "agentId": "agent-1",
                "message": {
                    "type": "assistant",
                    "content": "正在检查代码"
                }
            }
        });
        process_claude_object(&subagent_progress, false, &mut state);

        assert!(state.has_active_subagent_progress);
        assert!(state.last_assistant_had_tool_use);
        assert!(state.last_assistant_at.unwrap() > state.last_user_at.unwrap());

        let top_level_assistant = serde_json::json!({
            "type": "assistant",
            "timestamp": "2024-01-01T00:01:00Z",
            "cwd": "D:/Code/Aitify",
            "message": {
                "role": "assistant",
                "content": [{
                    "type": "text",
                    "text": "已经定位到问题并修复完成。"
                }]
            }
        });
        process_claude_object(&top_level_assistant, false, &mut state);

        assert!(!state.has_active_subagent_progress);
        assert!(!state.last_assistant_had_tool_use);
        assert!(state.last_assistant_at.unwrap() > state.last_user_at.unwrap());
    }

    #[test]
    fn test_claude_completion_requires_turn_duration_marker() {
        let mut state = ClaudeState::new();

        let user = serde_json::json!({
            "type": "user",
            "timestamp": "2024-01-01T00:00:00Z",
            "cwd": "D:/Code/Aitify",
            "message": {
                "role": "user",
                "content": "帮我继续分析"
            }
        });
        process_claude_object(&user, false, &mut state);

        let assistant_text = serde_json::json!({
            "type": "assistant",
            "timestamp": "2024-01-01T00:00:03Z",
            "cwd": "D:/Code/Aitify",
            "message": {
                "role": "assistant",
                "content": [{
                    "type": "text",
                    "text": "我先检查关键文件，再继续处理。"
                }]
            }
        });
        process_claude_object(&assistant_text, false, &mut state);

        assert!(current_claude_completion(&state).is_none());

        let turn_end = serde_json::json!({
            "type": "system",
            "subtype": "turn_duration",
            "timestamp": "2024-01-01T00:00:03.100Z",
            "cwd": "D:/Code/Aitify"
        });
        process_claude_object(&turn_end, false, &mut state);

        let completion = current_claude_completion(&state).expect("turn end marker should enable completion");
        assert_eq!(completion.0, 1704067200000);
        assert_eq!(completion.1, 1704067203000);
        assert_eq!(completion.2, 1704067203100);
    }

    #[test]
    fn test_is_claude_turn_end_system_only_matches_turn_duration() {
        let turn_end = serde_json::json!({
            "type": "system",
            "subtype": "turn_duration"
        });
        assert!(is_claude_turn_end_system(&turn_end));

        let stop_hook = serde_json::json!({
            "type": "system",
            "subtype": "stop_hook_summary"
        });
        assert!(!is_claude_turn_end_system(&stop_hook));
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
    fn test_process_codex_session_meta_marks_subagent_session() {
        let mut state = CodexSessionState::new();
        let meta = serde_json::json!({
            "type": "session_meta",
            "payload": {
                "id": "session-1",
                "cwd": "D:/Code/Aitify",
                "source": {
                    "subagent": {
                        "thread_spawn": {
                            "parent_thread_id": "parent-1",
                            "depth": 1,
                            "agent_nickname": "Ampere",
                            "agent_role": "explorer"
                        }
                    }
                },
                "agent_nickname": "Ampere",
                "agent_role": "explorer"
            }
        });

        process_codex_object(&meta, true, &mut state);

        assert!(state.is_subagent_session);
        assert_eq!(state.last_cwd.as_deref(), Some("D:/Code/Aitify"));
    }

    #[test]
    fn test_process_codex_session_meta_keeps_top_level_session_unmarked() {
        let mut state = CodexSessionState::new();
        let meta = serde_json::json!({
            "type": "session_meta",
            "payload": {
                "id": "session-1",
                "cwd": "D:/Code/Aitify",
                "originator": "codex_cli_rs"
            }
        });

        process_codex_object(&meta, true, &mut state);

        assert!(!state.is_subagent_session);
        assert_eq!(state.last_cwd.as_deref(), Some("D:/Code/Aitify"));
    }

    #[test]
    fn test_process_codex_subagent_task_complete_is_ignored() {
        let mut state = CodexSessionState::new();
        state.is_subagent_session = true;
        state.last_user_at = Some(1704067200000);
        state.last_cwd = Some("D:/Code/Aitify".to_string());

        let task_complete = serde_json::json!({
            "timestamp": "2024-01-01T00:02:00Z",
            "type": "event_msg",
            "payload": {
                "type": "task_complete",
                "turn_id": "turn-subagent-1",
                "last_agent_message": "subagent finished"
            }
        });

        process_codex_object(&task_complete, false, &mut state);

        assert_eq!(state.last_notified_turn_id, None);
        assert!(!state.confirm_notified_for_turn);
    }

    #[test]
    fn test_select_codex_interaction_notification_text_prefers_request_prompt_without_options() {
        let text = select_codex_interaction_notification_text(
            "请输入需要处理的目录路径",
            "需要你的确认",
        );

        assert_eq!(text, "请输入需要处理的目录路径");
    }

    #[test]
    fn test_process_codex_interaction_required_task_complete_marks_confirm_as_notified() {
        let mut state = CodexSessionState::new();
        state.interaction_required_for_turn = true;
        state.last_request_user_input_prompt = "请选择下一步".to_string();
        state.last_cwd = Some("D:/Code/Aitify".to_string());

        let task_complete = serde_json::json!({
            "timestamp": "2024-01-01T00:02:00Z",
            "type": "event_msg",
            "payload": {
                "type": "task_complete"
            }
        });

        process_codex_object(&task_complete, false, &mut state);

        assert!(state.confirm_notified_for_turn);
    }

    #[test]
    fn test_process_codex_confirm_prompt_task_complete_marks_confirm_as_notified() {
        let mut state = CodexSessionState::new();
        state.last_agent_content = Some("请确认是否继续执行？".to_string());
        state.last_cwd = Some("D:/Code/Aitify".to_string());

        let task_complete = serde_json::json!({
            "timestamp": "2024-01-01T00:02:00Z",
            "type": "event_msg",
            "payload": {
                "type": "task_complete",
                "last_agent_message": "请确认是否继续执行？"
            }
        });

        process_codex_object(&task_complete, false, &mut state);

        assert!(state.confirm_notified_for_turn);
    }

    #[test]
    fn test_detect_turn_end_confirm_prompt() {
        let text = "请确认是否继续执行？";
        assert!(detect_turn_end_confirm_prompt(text).is_some());

        let text = "Execute the command";
        assert!(detect_turn_end_confirm_prompt(text).is_none());
    }

    #[test]
    fn test_detect_turn_end_confirm_prompt_ignores_optional_follow_up_offer() {
        let text = "修复已经完成，验证也通过了。\n\n如果你要，我下一步可以继续补上回归测试和发布说明。";
        assert!(detect_turn_end_confirm_prompt(text).is_none());
    }

    #[test]
    fn test_detect_turn_end_confirm_prompt_keeps_direct_action_question() {
        let text = "变更已经完成。你要我现在直接执行发布吗？";
        assert_eq!(
            detect_turn_end_confirm_prompt(text),
            Some("变更已经完成。你要我现在直接执行发布吗？".to_string())
        );
    }

    #[test]
    fn test_detect_turn_end_confirm_prompt_ignores_english_status_statement() {
        let text = "I confirmed the root cause, updated the regression test, and the patch is ready.";
        assert!(detect_turn_end_confirm_prompt(text).is_none());
    }

    #[test]
    fn test_detect_turn_end_confirm_prompt_ignores_optional_english_follow_up_offer() {
        let text = "The patch is ready.\n\nI can continue with release notes and cleanup if needed.";
        assert!(detect_turn_end_confirm_prompt(text).is_none());
    }

    #[test]
    fn test_detect_turn_end_confirm_prompt_keeps_explicit_english_confirmation_request() {
        let text = "Please confirm whether I should proceed with deployment.";
        assert_eq!(
            detect_turn_end_confirm_prompt(text),
            Some("Please confirm whether I should proceed with deployment.".to_string())
        );
    }

    #[test]
    fn test_classify_turn_end_notification_marks_question_as_confirm() {
        let (notification_type, task_info) =
            classify_turn_end_notification("请确认是否继续执行？", "任务已完成");

        assert_eq!(notification_type, "confirm");
        assert_eq!(task_info, "请确认是否继续执行？");
    }

    #[test]
    fn test_classify_turn_end_notification_keeps_complete_message_for_non_question() {
        let (notification_type, task_info) =
            classify_turn_end_notification("已经修复完成。", "Claude 任务已完成");

        assert_eq!(notification_type, "complete");
        assert_eq!(task_info, "Claude 任务已完成");
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

    #[test]
    fn test_process_pi_records() {
        let mut state = PiSessionState::new();

        let session = serde_json::json!({
            "type": "session",
            "version": 3,
            "id": "sess-1",
            "timestamp": "2024-01-01T00:00:00Z",
            "cwd": "D:/Code/Aitify"
        });
        process_pi_object(&session, true, &mut state);
        assert_eq!(state.last_cwd.as_deref(), Some("D:/Code/Aitify"));

        let user = serde_json::json!({
            "type": "message",
            "id": "u1",
            "parentId": null,
            "timestamp": "2024-01-01T00:00:00Z",
            "message": {
                "role": "user",
                "content": [{"type": "text", "text": "请帮我修复测试"}],
                "timestamp": 1704067200000i64
            }
        });
        process_pi_object(&user, true, &mut state);
        assert!(state.last_user_at.is_some());
        assert!(state.last_assistant_at.is_none());

        let tool_use = serde_json::json!({
            "type": "message",
            "id": "a1",
            "parentId": "u1",
            "timestamp": "2024-01-01T00:00:30Z",
            "message": {
                "role": "assistant",
                "content": [{"type": "toolCall", "id": "c1", "name": "bash", "arguments": {}}],
                "stopReason": "toolUse",
                "timestamp": 1704067230000i64
            }
        });
        process_pi_object(&tool_use, false, &mut state);
        assert!(state.last_assistant_at.is_none());

        let assistant = serde_json::json!({
            "type": "message",
            "id": "a2",
            "parentId": "t1",
            "timestamp": "2024-01-01T00:01:00Z",
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "done"},
                    {"type": "text", "text": "已经修复完成"}
                ],
                "stopReason": "stop",
                "timestamp": 1704067260000i64
            }
        });
        process_pi_object(&assistant, false, &mut state);
        assert!(state.last_assistant_at.is_some());
        assert_eq!(state.last_agent_content.as_deref(), Some("已经修复完成"));
        assert!(state.last_assistant_at.unwrap() > state.last_user_at.unwrap());
    }

    #[test]
    fn test_process_pi_confirm_prompt() {
        let mut state = PiSessionState::new();

        let assistant = serde_json::json!({
            "type": "message",
            "id": "a1",
            "parentId": "u1",
            "timestamp": "2024-01-01T00:01:00Z",
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": "请确认是否继续执行？"}],
                "stopReason": "stop",
                "timestamp": 1704067260000i64
            }
        });
        process_pi_object(&assistant, false, &mut state);
        assert_eq!(
            detect_turn_end_confirm_prompt(state.last_agent_content.as_deref().unwrap_or("")),
            Some("请确认是否继续执行？".to_string())
        );
    }

    #[test]
    fn test_process_pi_official_session_jsonl_sample() {
        let mut state = PiSessionState::new();
        let sample = r#"{"type":"session","version":3,"id":"019f89c4-b98e-7325-a624-b33ba44109e1","timestamp":"2024-01-01T00:00:00.000Z","cwd":"D:/Code/Aitify"}
{"type":"message","id":"u1","parentId":null,"timestamp":"2024-01-01T00:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"Please inspect the failing Rust tests"}],"timestamp":1704067200000}}
{"type":"message","id":"a1","parentId":"u1","timestamp":"2024-01-01T00:00:30.000Z","message":{"role":"assistant","content":[{"type":"toolCall","id":"c1","name":"bash","arguments":{"command":"cargo test"}}],"stopReason":"toolUse","timestamp":1704067230000}}
{"type":"message","id":"t1","parentId":"a1","timestamp":"2024-01-01T00:00:40.000Z","message":{"role":"toolResult","toolCallId":"c1","toolName":"bash","content":[{"type":"text","text":"ok"}],"isError":false,"timestamp":1704067240000}}
{"type":"message","id":"a2","parentId":"t1","timestamp":"2024-01-01T00:01:00.000Z","message":{"role":"assistant","content":[{"type":"text","text":"I found the issue and fixed the failing assertion."}],"stopReason":"stop","timestamp":1704067260000}}"#;

        for (index, line) in sample.lines().enumerate() {
            let obj = safe_json_parse(line).expect("sample line should parse");
            process_pi_object(&obj, index < 2, &mut state);
        }

        assert_eq!(state.last_cwd.as_deref(), Some("D:/Code/Aitify"));
        assert!(state.last_user_at.is_some());
        assert!(state.last_assistant_at.is_some());
        assert_eq!(
            state.last_agent_content.as_deref(),
            Some("I found the issue and fixed the failing assertion.")
        );
        assert!(state.last_assistant_at.unwrap() > state.last_user_at.unwrap());
    }

}
