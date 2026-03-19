#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;

    #[test]
    fn test_normalize_sources() {
        assert_eq!(normalize_sources("all"), vec!["claude", "codex", "gemini", "qwen", "opencode"]);
        assert_eq!(normalize_sources(""), vec!["claude", "codex", "gemini", "qwen", "opencode"]);
        assert_eq!(normalize_sources("claude"), vec!["claude"]);
        assert_eq!(normalize_sources("claude,codex"), vec!["claude", "codex"]);
        assert_eq!(normalize_sources("qwen"), vec!["qwen"]);
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
    fn test_next_opencode_scan_cursor_does_not_jump_past_seen_data() {
        assert_eq!(next_opencode_scan_cursor(100, 120), 120);
        assert_eq!(next_opencode_scan_cursor(100, 80), 100);
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
    fn test_collect_gemini_message_jsons_returns_total_and_new_items() {
        let content = serde_json::json!({
            "messages": [
                { "type": "user", "timestamp": "2024-01-01T00:00:00Z" },
                { "type": "gemini", "timestamp": "2024-01-01T00:01:00Z" },
                { "type": "user", "timestamp": "2024-01-01T00:02:00Z" },
                { "type": "gemini", "timestamp": "2024-01-01T00:03:00Z" }
            ]
        })
        .to_string();

        let (new_items, total_count) =
            collect_gemini_message_jsons(&content, 2).expect("gemini messages should be collected");

        assert_eq!(total_count, 4);
        assert_eq!(new_items.len(), 2);
        assert!(new_items[0].contains("\"2024-01-01T00:02:00Z\""));
        assert!(new_items[1].contains("\"2024-01-01T00:03:00Z\""));
    }

    #[test]
    fn test_process_gemini_messages_from_content_only_processes_new_items() {
        let content = serde_json::json!({
            "messages": [
                { "type": "user", "timestamp": "2024-01-01T00:00:00Z" },
                { "type": "gemini", "timestamp": "2024-01-01T00:01:00Z" },
                { "type": "user", "timestamp": "2024-01-01T00:02:00Z" },
                { "type": "gemini", "timestamp": "2024-01-01T00:03:00Z" }
            ]
        })
        .to_string();
        let mut state = GeminiState::new();

        let total_count = process_gemini_messages_from_content(&content, 2, &mut state, 3000)
            .expect("gemini messages should be processed");

        assert_eq!(total_count, 4);
        assert_eq!(state.last_user_at, Some(1704067320000));
        assert_eq!(state.last_gemini_at, Some(1704067380000));
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

    #[test]
    fn test_process_qwen_records() {
        let mut state = QwenSessionState::new();

        let user = serde_json::json!({
            "type": "user",
            "timestamp": "2024-01-01T00:00:00Z",
            "message": {
                "role": "user",
                "parts": [{ "text": "请帮我修复测试" }]
            }
        });

        process_qwen_object(&user, true, &mut state);
        assert!(state.last_user_at.is_some());
        assert!(state.last_assistant_at.is_none());

        let assistant = serde_json::json!({
            "type": "assistant",
            "timestamp": "2024-01-01T00:01:00Z",
            "message": {
                "role": "model",
                "parts": [{ "text": "已经修复完成" }]
            }
        });

        process_qwen_object(&assistant, false, &mut state);
        assert!(state.last_assistant_at.is_some());
        assert_eq!(state.last_agent_content.as_deref(), Some("已经修复完成"));
    }

    #[test]
    fn test_process_qwen_confirm_prompt() {
        let mut state = QwenSessionState::new();

        let assistant = serde_json::json!({
            "type": "assistant",
            "timestamp": "2024-01-01T00:01:00Z",
            "message": {
                "role": "model",
                "parts": [{ "text": "请确认是否继续执行？" }]
            }
        });

        process_qwen_object(&assistant, false, &mut state);
        assert_eq!(
            detect_turn_end_confirm_prompt(state.last_agent_content.as_deref().unwrap_or("")),
            Some("请确认是否继续执行？".to_string())
        );
    }

    #[test]
    fn test_process_qwen_official_chatrecord_jsonl_sample() {
        let mut state = QwenSessionState::new();
        let sample = r#"{"uuid":"u-1","parentUuid":null,"sessionId":"550e8400-e29b-41d4-a716-446655440000","timestamp":"2024-01-01T00:00:00Z","type":"user","cwd":"D:/Code/Aitify","version":"1.5.4","gitBranch":"main","message":{"role":"user","parts":[{"text":"Please inspect the failing Rust tests"}]}}
{"uuid":"t-1","parentUuid":"u-1","sessionId":"550e8400-e29b-41d4-a716-446655440000","timestamp":"2024-01-01T00:00:05Z","type":"tool_result","cwd":"D:/Code/Aitify","version":"1.5.4","message":{"role":"user","parts":[{"functionResponse":{"name":"shell","response":{"ok":true}}}]}}
{"uuid":"a-1","parentUuid":"t-1","sessionId":"550e8400-e29b-41d4-a716-446655440000","timestamp":"2024-01-01T00:01:00Z","type":"assistant","cwd":"D:/Code/Aitify","version":"1.5.4","model":"qwen3-coder-plus","message":{"role":"model","parts":[{"text":"I found the issue and fixed the failing assertion."}]}}"#;

        for (index, line) in sample.lines().enumerate() {
            let obj = safe_json_parse(line).expect("sample line should parse");
            process_qwen_object(&obj, index < 2, &mut state);
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
