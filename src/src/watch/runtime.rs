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
    let qwen_root = home.join(QWEN_DIR);

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    let sources = normalize_sources(sources);
    let claude_quiet_ms = (claude_quiet_ms.max(500) as u64).max(3000);
    let gemini_quiet_ms = (gemini_quiet_ms.max(500) as u64).max(3000);

    tauri::async_runtime::spawn(async move {
        let mut claude_state = ClaudeState::new();
        let mut codex_states: HashMap<PathBuf, CodexSessionState> = HashMap::new();
        let mut gemini_state = GeminiState::new();
        let mut qwen_states: HashMap<PathBuf, QwenSessionState> = HashMap::new();
        let mut opencode_state = OpencodeState::new();

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
                        if let Ok(offset) = read_jsonl_objects_from_offset(&latest_file, 0, |obj| {
                            process_claude_object(&obj, true, &mut claude_state);
                        }) {
                            claude_state.last_file_size = offset;
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
                            if let Ok(offset) = read_jsonl_objects_from_offset(&latest_file, claude_state.last_file_size, |obj| {
                                let prev_assistant_at = claude_state.last_assistant_at;
                                process_claude_object(&obj, false, &mut claude_state);

                                if claude_state.last_assistant_at != prev_assistant_at {
                                    if let (Some(user_at), Some(assistant_at)) = (claude_state.last_user_at, claude_state.last_assistant_at) {
                                        if assistant_at >= user_at {
                                            // Always cancel old timer first (mirrors JS: clearTimeout before rescheduling)
                                            claude_state.cancel_pending();
                                            if !claude_state.confirm_notified_for_turn {
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
                            }) {
                                claude_state.last_file_size = offset;
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
                        let mut state = CodexSessionState::new();

                        if let Ok(offset) = read_jsonl_objects_from_offset(&file_path, 0, |obj| {
                            process_codex_object(&obj, true, &mut state);
                        }) {
                            state.processed_offset = offset;
                        } else {
                            continue;
                        }

                        // Pass 2: seedCatchupMs — treat recent lines as live
                        if seed_catchup_ms > 0 {
                            let since = now_unix_millis_i64() - seed_catchup_ms as i64;
                            let _ = read_jsonl_objects_from_offset(&file_path, 0, |obj| {
                                let ts = obj.get("timestamp").and_then(parse_timestamp);
                                if ts.map(|t| t >= since).unwrap_or(false) {
                                    process_codex_object(&obj, false, &mut state);
                                }
                            });
                        }

                        log_callback(format!("[watch][codex] following {:?}", file_path));
                        codex_states.insert(file_path.clone(), state);
                    } else if let Some(state) = codex_states.get_mut(&file_path) {
                        let file_size = safe_stat(&file_path).map(|stat| stat.len()).unwrap_or(0);
                        state.processed_offset = normalize_processed_offset(file_size, state.processed_offset);

                        if let Ok(offset) = read_jsonl_objects_from_offset(&file_path, state.processed_offset, |obj| {
                            process_codex_object(&obj, false, state);
                        }) {
                            state.processed_offset = offset;
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
                            if let Some(total_count) = process_gemini_messages_from_content(
                                &content,
                                0,
                                &mut gemini_state,
                                gemini_quiet_ms,
                            ) {
                                gemini_state.last_count = total_count;
                                // After seeding, mark notified so we don't re-fire on old data
                                gemini_state.last_notified_gemini_at = gemini_state.last_gemini_at;
                                gemini_state.cancel_pending();
                                log_callback(format!("[watch][gemini] following {:?}", latest_file));
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

                    let Some(total_count) = process_gemini_messages_from_content(
                        &content,
                        gemini_state.last_count,
                        &mut gemini_state,
                        gemini_quiet_ms,
                    ) else {
                        continue;
                    };

                    gemini_state.current_mtime_ms = mtime_ms;
                    gemini_state.last_count = total_count;
                }
            }

            // 定期清理
            // Monitor Qwen
            if sources.contains(&"qwen") && qwen_root.exists() {
                let follow_top_n = get_qwen_follow_top_n();
                let latest = find_latest_files(&qwen_root, is_qwen_chat_file, follow_top_n);

                for file_path in &latest {
                    if !qwen_states.contains_key(file_path) {
                        let mut state = QwenSessionState::new();

                        if let Ok(offset) = read_jsonl_objects_from_offset(file_path, 0, |obj| {
                            process_qwen_object(&obj, true, &mut state);
                        }) {
                            state.processed_offset = offset;
                        }

                        log_callback(format!("[watch][qwen] following {:?}", file_path));
                        qwen_states.insert(file_path.clone(), state);
                    }
                }

                let latest_set: HashSet<PathBuf> = latest.into_iter().collect();
                qwen_states.retain(|path, _| latest_set.contains(path));

                let followed_paths: Vec<PathBuf> = qwen_states.keys().cloned().collect();
                for file_path in followed_paths {
                    let Some(state) = qwen_states.get_mut(&file_path) else { continue; };
                    let file_size = safe_stat(&file_path).map(|stat| stat.len()).unwrap_or(0);
                    state.processed_offset = normalize_processed_offset(file_size, state.processed_offset);

                    if let Ok(offset) = read_jsonl_objects_from_offset(&file_path, state.processed_offset, |obj| {
                        let previous_assistant_at = state.last_assistant_at;
                        process_qwen_object(&obj, false, state);

                        if obj.get("type").and_then(|v| v.as_str()) == Some("assistant") {
                            let assistant_at = state.last_assistant_at.unwrap_or_else(now_unix_millis_i64);
                            let is_new_assistant = previous_assistant_at.map(|prev| assistant_at > prev).unwrap_or(true);

                            if is_new_assistant && state.last_notified_assistant_at != Some(assistant_at) {
                                let cwd = state.last_cwd.clone().unwrap_or_default();
                                let agent_content = state.last_agent_content.clone().unwrap_or_default();

                                if is_confirm_alert_enabled() {
                                    if let Some(prompt) = detect_turn_end_confirm_prompt(&agent_content) {
                                        tauri::async_runtime::spawn(async move {
                                            let _ = crate::notify::send_notifications("qwen", &prompt, None, cwd, false, Some("confirm")).await;
                                        });
                                        state.last_notified_assistant_at = Some(assistant_at);
                                        state.confirm_notified_for_turn = true;
                                        return;
                                    }
                                }

                                let duration_ms = state.last_user_at.map(|start| {
                                    if assistant_at >= start { assistant_at - start } else { 0 }
                                });

                                tauri::async_runtime::spawn(async move {
                                    let _ = crate::notify::send_notifications("qwen", "Qwen 任务已完成", duration_ms, cwd, false, Some("complete")).await;
                                });
                                state.last_notified_assistant_at = Some(assistant_at);
                                state.confirm_notified_for_turn = true;
                            }
                        }
                    }) {
                        state.processed_offset = offset;
                    }
                }
            }

            // Monitor OpenCode
            if sources.contains(&"opencode") {
                if let Some(db_path) = find_latest_opencode_db(&home) {
                    let is_new_db = opencode_state.current_db.as_ref() != Some(&db_path);
                    if is_new_db {
                        log_callback(format!("[watch][opencode] following {:?}", db_path));
                    }

                    let scan_limit = get_opencode_scan_limit();
                    match poll_opencode_completions(&mut opencode_state, &db_path, scan_limit) {
                        Ok(completions) => {
                            for completion in completions {
                                let cwd = completion.cwd.clone();
                                let duration_ms = completion.duration_ms;
                                tauri::async_runtime::spawn(async move {
                                    let _ = crate::notify::send_notifications(
                                        "opencode",
                                        "OpenCode 任务已完成",
                                        duration_ms,
                                        cwd,
                                        false,
                                        Some("complete"),
                                    )
                                    .await;
                                });
                            }
                        }
                        Err(err) => {
                            log_callback(format!("[watch][opencode] failed to scan {:?}: {}", db_path, err));
                        }
                    }
                }
            }

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

