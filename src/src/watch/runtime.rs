// ============ 主循环 ============

pub fn start_watch<F>(
    sources: &str,
    interval_ms: i32,
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
    let pi_root = home.join(PI_DIR);

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    let sources = normalize_sources(sources);
    let claude_quiet_ms = (claude_quiet_ms.max(500) as u64).max(3000);

    tauri::async_runtime::spawn(async move {
        let mut claude_state = ClaudeState::new();
        let mut codex_states: HashMap<PathBuf, CodexSessionState> = HashMap::new();
        let mut pi_states: HashMap<PathBuf, PiSessionState> = HashMap::new();
        let mut opencode_state = OpencodeState::new();

        let mut tick_interval = interval(Duration::from_millis((interval_ms.max(500) as u64).max(1000)));
        let mut cleanup_counter = 0u32;

        log_callback(format!("[watch] started with sources: {:?}", sources));

        while running_clone.load(Ordering::Relaxed) {
            tick_interval.tick().await;

            // Monitor Claude
            if sources.contains(&"claude") && claude_root.exists() {
                if let Some(latest_file) = find_latest_file(&claude_root, |_, name| {
                    name.to_lowercase().ends_with(".jsonl")
                }) {
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
                        if let Some((user_at, assistant_at, turn_end_at)) = current_claude_completion(&claude_state) {
                            if !claude_state.notified_for_turn
                                && !claude_state.confirm_notified_for_turn
                            {
                                let now = now_unix_millis_i64();
                                let window_ms = (claude_quiet_ms * 2).max(15000) as i64;
                                if now - turn_end_at <= window_ms {
                                    let had_tool_use = claude_state.last_assistant_had_tool_use;
                                    let adaptive_ms = if had_tool_use { claude_quiet_ms } else { claude_quiet_ms.min(15000) };
                                    let cancel = Arc::new(AtomicBool::new(false));
                                    claude_state.pending_cancel = Some(cancel.clone());
                                    let cwd = claude_state.last_cwd.clone().unwrap_or_default();
                                    let duration_ms = assistant_at - user_at;
                                    let agent_content = claude_state.last_agent_content.clone().unwrap_or_default();
                                    tauri::async_runtime::spawn(async move {
                                        tokio::time::sleep(Duration::from_millis(adaptive_ms)).await;
                                        if cancel.load(Ordering::Relaxed) { return; }
                                        let (notification_type, task_info) =
                                            classify_turn_end_notification(&agent_content, "Claude 任务已完成");
                                        let notify_duration_ms = if notification_type == "confirm" {
                                            None
                                        } else {
                                            Some(duration_ms)
                                        };
                                        let _ = crate::notify::send_notifications(
                                            "claude",
                                            &task_info,
                                            notify_duration_ms,
                                            cwd,
                                            false,
                                            Some(notification_type),
                                        ).await;
                                    });
                                    claude_state.notified_for_turn = true;
                                    claude_state.confirm_notified_for_turn = true;
                                    claude_state.last_notified_at = Some(turn_end_at);
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
                                let prev_turn_end_at = claude_state.last_turn_end_at;
                                process_claude_object(&obj, false, &mut claude_state);

                                if claude_state.last_turn_end_at != prev_turn_end_at {
                                    if let Some((user_at, assistant_at, turn_end_at)) = current_claude_completion(&claude_state) {
                                        // Always cancel old timer first (mirrors JS: clearTimeout before rescheduling)
                                        claude_state.cancel_pending();
                                        if !claude_state.confirm_notified_for_turn {
                                            let had_tool_use = claude_state.last_assistant_had_tool_use;
                                            let adaptive_ms = if had_tool_use { claude_quiet_ms } else { claude_quiet_ms.min(15000) };
                                            let cancel = Arc::new(AtomicBool::new(false));
                                            claude_state.pending_cancel = Some(cancel.clone());
                                            let cwd = claude_state.last_cwd.clone().unwrap_or_default();
                                            let duration_ms = assistant_at - user_at;
                                            let agent_content = claude_state.last_agent_content.clone().unwrap_or_default();
                                            tauri::async_runtime::spawn(async move {
                                                tokio::time::sleep(Duration::from_millis(adaptive_ms)).await;
                                                if cancel.load(Ordering::Relaxed) { return; }
                                                let (notification_type, task_info) =
                                                    classify_turn_end_notification(&agent_content, "Claude 任务已完成");
                                                let notify_duration_ms = if notification_type == "confirm" {
                                                    None
                                                } else {
                                                    Some(duration_ms)
                                                };
                                                let _ = crate::notify::send_notifications(
                                                    "claude",
                                                    &task_info,
                                                    notify_duration_ms,
                                                    cwd,
                                                    false,
                                                    Some(notification_type),
                                                ).await;
                                            });
                                            claude_state.notified_for_turn = true;
                                            claude_state.confirm_notified_for_turn = true;
                                            claude_state.last_notified_at = Some(turn_end_at);
                                            log_callback(format!("[watch][claude] notification scheduled after turn end ({}ms adaptive)", adaptive_ms));
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

            // Monitor Pi
            if sources.contains(&"pi") && pi_root.exists() {
                let follow_top_n = get_pi_follow_top_n();
                let latest = find_latest_files(&pi_root, is_pi_session_file, follow_top_n);

                for file_path in &latest {
                    if !pi_states.contains_key(file_path) {
                        let mut state = PiSessionState::new();

                        if let Ok(offset) = read_jsonl_objects_from_offset(file_path, 0, |obj| {
                            process_pi_object(&obj, true, &mut state);
                        }) {
                            state.processed_offset = offset;
                            // Avoid replaying historical completions when attaching to an existing session.
                            state.last_notified_assistant_at = state.last_assistant_at;
                            state.confirm_notified_for_turn = true;
                        }

                        log_callback(format!("[watch][pi] following {:?}", file_path));
                        pi_states.insert(file_path.clone(), state);
                    }
                }

                let latest_set: HashSet<PathBuf> = latest.into_iter().collect();
                pi_states.retain(|path, _| latest_set.contains(path));

                let followed_paths: Vec<PathBuf> = pi_states.keys().cloned().collect();
                for file_path in followed_paths {
                    let Some(state) = pi_states.get_mut(&file_path) else { continue; };
                    let file_size = safe_stat(&file_path).map(|stat| stat.len()).unwrap_or(0);
                    state.processed_offset = normalize_processed_offset(file_size, state.processed_offset);

                    if let Ok(offset) = read_jsonl_objects_from_offset(&file_path, state.processed_offset, |obj| {
                        let previous_assistant_at = state.last_assistant_at;
                        process_pi_object(&obj, false, state);

                        let is_terminal_assistant = obj.get("type").and_then(|v| v.as_str()) == Some("message")
                            && obj
                                .get("message")
                                .and_then(|m| m.get("role"))
                                .and_then(|v| v.as_str())
                                == Some("assistant")
                            && obj
                                .get("message")
                                .and_then(|m| m.get("stopReason"))
                                .and_then(|v| v.as_str())
                                == Some("stop");

                        if !is_terminal_assistant {
                            return;
                        }

                        let assistant_at = state.last_assistant_at.unwrap_or_else(now_unix_millis_i64);
                        let is_new_assistant = previous_assistant_at.map(|prev| assistant_at > prev).unwrap_or(true);

                        if is_new_assistant && state.last_notified_assistant_at != Some(assistant_at) {
                            let cwd = state.last_cwd.clone().unwrap_or_default();
                            let agent_content = state.last_agent_content.clone().unwrap_or_default();

                            let (notification_type, task_info) =
                                classify_turn_end_notification(&agent_content, "Pi 任务已完成");

                            if notification_type == "confirm" {
                                tauri::async_runtime::spawn(async move {
                                    let _ = crate::notify::send_notifications(
                                        "pi",
                                        &task_info,
                                        None,
                                        cwd,
                                        false,
                                        Some("confirm"),
                                    )
                                    .await;
                                });
                                state.last_notified_assistant_at = Some(assistant_at);
                                state.confirm_notified_for_turn = true;
                                return;
                            }

                            let duration_ms = state.last_user_at.map(|start| {
                                if assistant_at >= start {
                                    assistant_at - start
                                } else {
                                    0
                                }
                            });

                            tauri::async_runtime::spawn(async move {
                                let _ = crate::notify::send_notifications(
                                    "pi",
                                    "Pi 任务已完成",
                                    duration_ms,
                                    cwd,
                                    false,
                                    Some("complete"),
                                )
                                .await;
                            });
                            state.last_notified_assistant_at = Some(assistant_at);
                            state.confirm_notified_for_turn = true;
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
                    match poll_opencode_notifications(&mut opencode_state, &db_path, scan_limit) {
                        Ok(notifications) => {
                            for notification in notifications {
                                let cwd = notification.cwd.clone();
                                let duration_ms = notification.duration_ms;
                                let task_info = notification.task_info.clone();
                                let notification_type = notification.notification_type;
                                tauri::async_runtime::spawn(async move {
                                    let _ = crate::notify::send_notifications(
                                        "opencode",
                                        &task_info,
                                        duration_ms,
                                        cwd,
                                        false,
                                        Some(notification_type),
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
