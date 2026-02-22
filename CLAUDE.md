# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

AI CLI Complete Notify - A task completion notification tool for Claude Code, Codex, and Gemini. Supports multiple notification channels (Telegram, Desktop, Sound) and flexible configuration.

## Commands

```bash
# Development
npm run dev              # Run Electron desktop app in development mode
npm run dist             # Build Windows portable executable
npm run dist:portable    # Alternative build using electron-builder
npm run wizard           # Run setup wizard

# CLI Usage
node ai-reminder.js notify --source claude --task "Task completed"
node ai-reminder.js run --source codex -- codex <args...>
node ai-reminder.js start --source gemini --task "Build project"
node ai-reminder.js stop --source gemini --task "Build project"
node ai-reminder.js watch --sources all --gemini-quiet-ms 3000 --claude-quiet-ms 60000
```

## Architecture

### Core Modules

- **`desktop/main.js`** - Electron main process, handles IPC, tray, window management
- **`desktop/renderer/`** - Frontend UI (index.html, style.css, renderer.js)
- **`src/cli.js`** - CLI command router (notify/run/start/stop/watch/config)
- **`src/engine.js`** - Central notification orchestrator, dispatches to multiple channels
- **`src/watch.js`** - Log file monitoring for Claude (~/.claude), Codex (~/.codex), Gemini (~/.gemini)
- **`src/config.js`** - Config loading/saving with env variable overrides

### Notification Channels (`src/notifiers/`)

- `telegram.js` - Telegram bot notifications
- `desktop.js` - Windows desktop balloon notifications
- `sound.js` - System sound/TTS alerts

### Key Features

- **Smart Debouncing**: Adaptive quiet periods (60s for tool calls, 15s otherwise)
- **Duration Threshold**: Only notify when tasks exceed configured duration
- **Confirm Alerts**: Detects interactive prompts requiring user input
- **AI Summary**: Optional task summarization via LLM API before sending notifications
- **Focus Target**: Click notification to return to VSCode/terminal/workspace

### Configuration

- **Runtime config**: `%APPDATA%\ai-cli-complete-notify\settings.json` (managed by desktop app)
- **Environment**: `.env` file for sensitive data (Telegram tokens, SMTP credentials)
- **Config structure**: `src/default-config.js` defines schema with channels, sources, and UI settings

### Data Flow

1. Watch mode: Monitors AI CLI log files → detects turn completion → calls `sendNotifications()`
2. Run mode: Wraps child process → measures duration → calls `sendNotifications()` on exit
3. `sendNotifications()` → evaluates thresholds → dispatches to enabled channels in parallel

### Platform Notes

- Desktop notifications and tray: Windows only
- Sound notifications: Windows native, WSL passthrough via PowerShell
- Log paths: `~/.claude/projects/*.jsonl`, `~/.codex/sessions/*.jsonl`, `~/.gemini/tmp/chats/session-*.json`

### UI Structure

- **Overview**: Status dashboard with watch status, active channels/sources, quick actions
- **Channels**: Global toggles for Telegram, Desktop, Sound
- **Sources**: Per-source configuration (Claude, Codex, Gemini) with duration thresholds
- **Watch**: Interactive monitoring settings with log viewer
- **Test**: Manual notification testing
- **Settings**: App preferences (language, close behavior, autostart, sound options)
