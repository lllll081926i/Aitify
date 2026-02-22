# AI CLI Complete Notify

![Version](https://img.shields.io/badge/version-1.5.2-blue.svg)
![License](https://img.shields.io/badge/license-ISC-green.svg)
![Platform](https://img.shields.io/badge/platform-Windows-lightgrey.svg)

[English](#english) | [中文](#中文)

---

## English

### 📖 Introduction

A lightweight task completion notification tool for Claude Code / Codex / Gemini. Get Windows native notifications when AI assistants complete long-running tasks.

**Notification Method:**
🖥️ Windows Native Desktop Notifications

### ✨ Key Features

- 🎯 **Smart Monitoring**: Automatically detects task completion from AI CLI log files
- 🔀 **Multi-Source Support**: Independent configuration for Claude / Codex / Gemini
- ⏱️ **Duration Threshold**: Only notify when tasks exceed the configured duration
- 🖥️ **Desktop GUI**: Modern interface with system tray support
- 🚀 **Auto-start**: Launch on system startup
- 🌐 **Multi-language**: English and Chinese interface

### 🚀 Quick Start

1. **Install**
   ```bash
   npm install
   npm run build
   ```

2. **Run**
   ```bash
   npm run dev
   ```

3. **Configure**
   - Enable/disable AI sources (Claude, Codex, Gemini)
   - Set minimum notification duration (minutes)
   - Configure auto-start

### 📋 Requirements

- Windows 10/11
- Node.js 18+
- Rust 1.77+

### 🏗️ Architecture

- **Frontend**: HTML/CSS/JavaScript
- **Backend**: Tauri 2 + Rust
- **Notifications**: Windows native Toast notifications

### 📝 How It Works

1. Monitors AI CLI log files:
   - Claude: `~/.claude/projects/*.jsonl`
   - Codex: `~/.codex/sessions/*.jsonl`
   - Gemini: `~/.gemini/tmp/chats/session-*.json`

2. Detects task completion signals
3. Sends Windows native notification

### 📄 License

ISC License

---

## 中文

### 📖 简介

轻量级的 AI CLI 任务完成通知工具,支持 Claude Code / Codex / Gemini。当 AI 助手完成长时间运行的任务时,自动发送 Windows 原生通知。

**通知方式:**
🖥️ Windows 原生桌面通知

### ✨ 核心功能

- 🎯 **智能监控**: 自动检测 AI CLI 日志文件中的任务完成信号
- 🔀 **多源支持**: Claude / Codex / Gemini 独立配置
- ⏱️ **时长阈值**: 仅在任务超过设定时长时通知
- 🖥️ **桌面应用**: 现代化界面,支持系统托盘
- 🚀 **开机自启**: 系统启动时自动运行
- 🌐 **多语言**: 中英文界面

### 🚀 快速开始

1. **安装**
   ```bash
   npm install
   npm run build
   ```

2. **运行**
   ```bash
   npm run dev
   ```

3. **配置**
   - 启用/禁用 AI 源 (Claude, Codex, Gemini)
   - 设置最小通知时长(分钟)
   - 配置开机自启

### 📋 系统要求

- Windows 10/11
- Node.js 18+
- Rust 1.77+

### 🏗️ 技术架构

- **前端**: HTML/CSS/JavaScript
- **后端**: Tauri 2 + Rust
- **通知**: Windows 原生 Toast 通知

### 📝 工作原理

1. 监控 AI CLI 日志文件:
   - Claude: `~/.claude/projects/*.jsonl`
   - Codex: `~/.codex/sessions/*.jsonl`
   - Gemini: `~/.gemini/tmp/chats/session-*.json`

2. 检测任务完成信号
3. 发送 Windows 原生通知

### 📄 许可证

ISC License
