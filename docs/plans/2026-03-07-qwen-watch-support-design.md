# Qwen CLI 监听支持设计

**目标：** 为 Aitify 增加对 Qwen CLI / Qwen Code 会话日志的监听能力，在任务完成或需要用户确认时发送桌面通知。

**背景：** 官方文档与源码确认 Qwen 会话按项目保存在 `~/.qwen/projects/<sanitized-cwd>/chats/<sessionId>.jsonl`。记录格式为 JSONL，核心类型包含 `user`、`assistant`、`tool_result`、`system`。

**设计决策：**
- 采用与现有 `codex` 类似的“多会话跟随”模式，而不是只监听单一最新文件。
- 以 `user` 记录作为一次任务/轮次的开始信号。
- 以新的 `assistant` 记录作为完成信号。
- 复用现有确认提示词启发式，对 assistant 文本中的确认语句触发 `confirm` 通知。
- 将 `qwen` 作为新的 source 接入 `watch.rs`、`config.rs`、`ui/index.html`、`ui/app.js`，并纳入默认 sources。

**数据流：**
- watcher 扫描 `~/.qwen/projects/**/chats/*.jsonl`
- 跟随最近更新的若干个 session 文件
- 增量解析新增 JSONL 行
- 更新 `QwenSessionState`
- 发现完成/确认事件后调用 `notify::send_notifications("qwen", ...)`

**风险与约束：**
- Qwen 上游未来可能新增记录 subtype，但 `user/assistant` 基础类型相对稳定。
- 不做对 Qwen 内部 system subtype 的强绑定，减少升级脆弱性。
- 首版不尝试恢复精确工具级时长，只按最近 `user -> assistant` 计算 duration。

**验证策略：**
- 先补 `watch.rs` 单元测试，覆盖 sources 归一化与 Qwen JSONL 记录处理。
- 然后实现最小代码让测试通过。
- 最后运行 `cargo test`、`cargo check`，并做不少于 3 轮验证迭代。
