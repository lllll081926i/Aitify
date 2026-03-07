# Qwen Watch Support Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 为 Aitify 增加 Qwen CLI 会话监听、完成通知、确认提醒以及对应配置与 UI 支持。

**Architecture:** 在 `src/src/watch.rs` 中新增 `Qwen` 专用状态机，扫描 `~/.qwen/projects/**/chats/*.jsonl`，按会话文件增量消费 JSONL 记录。复用现有通知与确认提示词检测逻辑，并把 `qwen` 接入配置模型和前端配置界面。

**Tech Stack:** Rust, Tauri, serde_json, Tokio, 原生 HTML/CSS/JS。

---

### Task 1: 补 Qwen watcher 失败测试

**Files:**
- Modify: `src/src/watch.rs`
- Test: `src/src/watch.rs`

**Step 1: Write the failing test**
- 为 `normalize_sources("all")` 增加 `qwen` 断言。
- 新增 Qwen JSONL 记录测试：`user` 记录应设置开始时间，`assistant` 记录应设置完成时间和内容。
- 新增确认提示测试：assistant 文本包含 `请确认是否继续` 时，识别为确认提示。

**Step 2: Run test to verify it fails**
- Run: `cd src && cargo test test_normalize_sources -- --nocapture`
- Run: `cd src && cargo test test_process_qwen_records -- --nocapture`
- Expected: 失败，因为当前尚未包含 `qwen` 和对应处理逻辑。

**Step 3: Write minimal implementation**
- 在 `watch.rs` 中补 `qwen` source 归一化、状态结构和对象处理函数。

**Step 4: Run test to verify it passes**
- Run: `cd src && cargo test test_normalize_sources -- --nocapture`
- Run: `cd src && cargo test test_process_qwen_records -- --nocapture`
- Expected: PASS

### Task 2: 接入 Qwen watcher 主循环

**Files:**
- Modify: `src/src/watch.rs`

**Step 1: Write the failing test**
- 为 Qwen 多文件扫描行为添加最小回归测试，至少验证 `all` sources 包含 `qwen`。

**Step 2: Run test to verify it fails**
- Run: `cd src && cargo test test_normalize_sources -- --nocapture`

**Step 3: Write minimal implementation**
- 新增 `QWEN_DIR` 常量。
- 在 watcher 循环中扫描 `~/.qwen/projects/**/chats/*.jsonl`。
- 跟随最近更新的多个文件并增量处理新增行。
- 发现完成/确认事件后发送 `qwen` 通知。

**Step 4: Run test to verify it passes**
- Run: `cd src && cargo test -- --nocapture`

### Task 3: 接入配置与通知过滤

**Files:**
- Modify: `src/src/config.rs`
- Modify: `src/src/notify.rs`

**Step 1: Write the failing test**
- 当前仓库无 config/notify 单测，保持最小改动，不新增独立测试文件。

**Step 2: Write minimal implementation**
- 在 `SourcesConfig` 中加入 `qwen`。
- 在通知源映射中加入 `qwen`。

**Step 3: Run verification**
- Run: `cd src && cargo check`
- Expected: PASS

### Task 4: 接入 UI 设置面板

**Files:**
- Modify: `ui/index.html`
- Modify: `ui/app.js`

**Step 1: Write minimal implementation**
- 新增 Qwen 开关和最小时长输入项。
- 更新前端 sources 列表与配置归一化逻辑。

**Step 2: Run verification**
- Run: `npm run build`
- Expected: 前端/Tauri 构建通过。

### Task 5: 更新文档并完成验证

**Files:**
- Modify: `README.md`

**Step 1: Update docs**
- 在项目说明与支持来源中补充 `Qwen`。

**Step 2: Run full verification**
- Run: `cd src && cargo test -- --nocapture`
- Run: `cd src && cargo check`
- Run: `npm run build`

**Step 3: Record results**
- 汇总实现范围、验证结果、已知限制。
