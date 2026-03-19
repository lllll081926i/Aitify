// ============ Codex Watch ============

struct CodexSessionState {
    processed_offset: u64,
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
            processed_offset: 0,
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

                state.last_request_user_input_prompt = compact_state_text(
                    &extract_request_user_input_text(&Value::Object(payload.clone()))
                );
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
                    state.last_agent_content = Some(compact_state_text(&assistant_text));
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
                            state.last_agent_content = Some(compact_state_text(last_agent_msg));
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
                            state.last_agent_content = Some(compact_state_text(&c));
                        }
                    }
                }

                _ => {}
            }
        }
    }
}

