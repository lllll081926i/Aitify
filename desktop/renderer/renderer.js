/**
 * AI CLI Complete Notify - Renderer Process
 * 现代化浅色主题 UI
 */

const { ipcRenderer } = require('electron');

// ========== 状态管理 ==========
const state = {
  config: null,
  watchRunning: false,
  logs: [],
  currentTab: 'overview'
};

// ========== DOM 元素 ==========
const elements = {
  // 导航
  navItems: document.querySelectorAll('.nav-item'),

  // 标签页
  tabPanes: document.querySelectorAll('.tab-pane'),

  // 状态指示器
  watchIndicator: document.getElementById('watch-indicator'),
  watchStatusText: document.getElementById('watch-status-text'),
  btnToggleWatch: document.getElementById('btn-toggle-watch'),

  // 统计卡片
  statTodayTasks: document.getElementById('stat-today-tasks'),
  statNotifications: document.getElementById('stat-notifications'),
  statActiveChannels: document.getElementById('stat-active-channels'),
  statActiveSources: document.getElementById('stat-active-sources'),

  // 日志列表
  recentLogList: document.getElementById('recent-log-list'),
  fullLogList: document.getElementById('full-log-list'),

  // Toast 容器
  toastContainer: document.getElementById('toast-container'),

  // 窗口控制
  btnMinimize: document.getElementById('btn-minimize'),
  btnMaximize: document.getElementById('btn-maximize'),
  btnClose: document.getElementById('btn-close')
};

// ========== 初始化 ==========
function init() {
  loadConfig();
  setupEventListeners();
  updateStats();
  renderLogs();

  // 请求配置路径
  ipcRenderer.send('request-config-path');
}

// ========== 事件监听 ==========
function setupEventListeners() {
  // 导航切换
  elements.navItems.forEach(item => {
    item.addEventListener('click', () => {
      const tab = item.dataset.tab;
      if (tab) switchTab(tab);
    });
  });

  // 窗口控制
  if (elements.btnMinimize) {
    elements.btnMinimize.addEventListener('click', () => {
      ipcRenderer.send('window-minimize');
    });
  }

  if (elements.btnMaximize) {
    elements.btnMaximize.addEventListener('click', () => {
      ipcRenderer.send('window-maximize');
    });
  }

  if (elements.btnClose) {
    elements.btnClose.addEventListener('click', () => {
      ipcRenderer.send('window-close');
    });
  }

  // 监控开关
  if (elements.btnToggleWatch) {
    elements.btnToggleWatch.addEventListener('click', toggleWatch);
  }

  // 渠道开关
  ['telegram', 'desktop', 'sound'].forEach(channel => {
    const el = document.getElementById(`channel-${channel}`);
    if (el) {
      el.addEventListener('change', (e) => {
        updateChannelConfig(channel, e.target.checked);
      });
    }
  });

  // Telegram 配置
  ['token', 'chat-id', 'proxy'].forEach(field => {
    const el = document.getElementById(`telegram-${field}`);
    if (el) {
      el.addEventListener('blur', () => saveTelegramConfig());
    }
  });

  // AI 源配置
  ['claude', 'codex', 'gemini'].forEach(source => {
    // 启用开关
    const enabledEl = document.getElementById(`source-${source}-enabled`);
    if (enabledEl) {
      enabledEl.addEventListener('change', (e) => {
        updateSourceConfig(source, 'enabled', e.target.checked);
      });
    }

    // 时长配置
    const durationEl = document.getElementById(`source-${source}-duration`);
    if (durationEl) {
      durationEl.addEventListener('blur', (e) => {
        updateSourceConfig(source, 'minDurationMinutes', parseInt(e.target.value) || 0);
      });
    }

    // 渠道选择
    document.querySelectorAll(`.source-channel[data-source="${source}"]`).forEach(chEl => {
      chEl.addEventListener('change', (e) => {
        updateSourceChannelConfig(source, e.target.dataset.channel, e.target.checked);
      });
    });
  });

  // 监控设置
  const watchSettings = {
    'watch-interval': (v) => updateWatchConfig('intervalMs', parseInt(v) || 1000),
    'gemini-quiet-ms': (v) => updateWatchConfig('geminiQuietMs', parseInt(v) || 3000),
    'claude-quiet-ms': (v) => updateWatchConfig('claudeQuietMs', parseInt(v) || 60000),
    'log-retention-days': (v) => updateWatchConfig('logRetentionDays', parseInt(v) || 7)
  };

  Object.entries(watchSettings).forEach(([id, handler]) => {
    const el = document.getElementById(id);
    if (el) {
      el.addEventListener('blur', (e) => handler(e.target.value));
    }
  });

  // 确认提醒
  const confirmAlertEl = document.getElementById('confirm-alert-enabled');
  if (confirmAlertEl) {
    confirmAlertEl.addEventListener('change', (e) => {
      updateWatchConfig('confirmAlertEnabled', e.target.checked);
    });
  }

  const confirmKeywordsEl = document.getElementById('confirm-alert-keywords');
  if (confirmKeywordsEl) {
    confirmKeywordsEl.addEventListener('blur', (e) => {
      updateWatchConfig('confirmAlertKeywords', e.target.value.split(',').map(s => s.trim()).filter(Boolean));
    });
  }

  // 测试按钮
  document.getElementById('btn-test-telegram')?.addEventListener('click', () => testNotification('telegram'));
  document.getElementById('btn-test-desktop')?.addEventListener('click', () => testNotification('desktop'));
  document.getElementById('btn-test-sound')?.addEventListener('click', () => testNotification('sound'));

  // 日志操作
  document.getElementById('btn-refresh-logs')?.addEventListener('click', refreshLogs);
  document.getElementById('btn-clear-logs')?.addEventListener('click', clearLogs);

  // 打开目录按钮
  document.querySelectorAll('#btn-open-log-dir').forEach(btn => {
    btn.addEventListener('click', () => {
      ipcRenderer.send('open-log-dir');
    });
  });

  // 设置相关
  document.getElementById('btn-open-config')?.addEventListener('click', () => {
    ipcRenderer.send('open-config-file');
  });

  document.getElementById('btn-open-data-dir')?.addEventListener('click', () => {
    ipcRenderer.send('open-data-dir');
  });

  // 设置变更
  ['language', 'close-behavior', 'sound-type'].forEach(field => {
    const el = document.getElementById(`setting-${field}`);
    if (el) {
      el.addEventListener('change', (e) => saveSetting(field, e.target.value));
    }
  });

  ['autostart', 'silent-start', 'autofocus'].forEach(field => {
    const el = document.getElementById(`setting-${field}`);
    if (el) {
      el.addEventListener('change', (e) => saveSetting(field, e.target.checked));
    }
  });

  document.getElementById('setting-tts-template')?.addEventListener('blur', (e) => {
    saveSetting('ttsTemplate', e.target.value);
  });

  // 快速操作
  document.querySelectorAll('[data-quick-source]').forEach(btn => {
    btn.addEventListener('click', () => {
      const source = btn.dataset.quickSource;
      showToast(`快速测试：${source}`, 'info');
      testNotification('desktop', source);
    });
  });

  // 导航到日志
  document.querySelectorAll('[data-nav]').forEach(el => {
    el.addEventListener('click', () => {
      const target = el.dataset.nav;
      if (target) switchTab(target);
    });
  });

  // IPC 监听
  ipcRenderer.on('config-loaded', (event, config) => {
    state.config = config;
    loadConfigToUI(config);
    updateStats();
  });

  ipcRenderer.on('watch-status', (event, status) => {
    state.watchRunning = status.running;
    updateWatchUI(status.running);
  });

  ipcRenderer.on('log-entry', (event, entry) => {
    state.logs.unshift({
      time: new Date().toLocaleTimeString(),
      source: entry.source || 'unknown',
      message: entry.message || '',
      type: entry.type || 'info'
    });
    if (state.logs.length > 100) state.logs.pop();
    renderLogs();
    updateStats();
  });

  ipcRenderer.on('config-path', (event, path) => {
    const el = document.getElementById('config-path');
    if (el) el.value = path;
  });

  ipcRenderer.on('toast', (event, message, type) => {
    showToast(message, type);
  });
}

// ========== 导航切换 ==========
function switchTab(tabId) {
  state.currentTab = tabId;

  // 更新导航项
  elements.navItems.forEach(item => {
    item.classList.toggle('active', item.dataset.tab === tabId);
  });

  // 更新标签页
  elements.tabPanes.forEach(pane => {
    pane.classList.toggle('active', pane.id === `tab-${tabId}`);
  });

  // 刷新日志列表（如果在日志页面）
  if (tabId === 'logs') {
    renderLogs(true);
  }
}

// ========== 配置加载 ==========
function loadConfig() {
  ipcRenderer.send('request-config');
}

function loadConfigToUI(config) {
  if (!config) return;

  // 渠道配置
  if (config.channels) {
    { const el = document.getElementById('channel-telegram'); if (el) el.checked = !!config.channels.telegram?.enabled; }
    { const el = document.getElementById('channel-desktop'); if (el) el.checked = !!config.channels.desktop?.enabled; }
    { const el = document.getElementById('channel-sound'); if (el) el.checked = !!config.channels.sound?.enabled; }

    if (config.channels.telegram) {
      { const el = document.getElementById('telegram-token'); if (el) el.value = config.channels.telegram.botToken || ''; }
      { const el = document.getElementById('telegram-chat-id'); if (el) el.value = config.channels.telegram.chatId || ''; }
      { const el = document.getElementById('telegram-proxy'); if (el) el.value = config.channels.telegram.httpProxy || ''; }
    }
  }

  // AI 源配置
  if (config.sources) {
    ['claude', 'codex', 'gemini'].forEach(source => {
      const srcConfig = config.sources[source];
      if (srcConfig) {
        { const el = document.getElementById(`source-${source}-enabled`); if (el) el.checked = !!srcConfig.enabled; }
        { const el = document.getElementById(`source-${source}-duration`); if (el) el.value = srcConfig.minDurationMinutes || 0; }

        if (srcConfig.channels) {
          document.querySelectorAll(`.source-channel[data-source="${source}"]`).forEach(chEl => {
            chEl.checked = !!srcConfig.channels[chEl.dataset.channel];
          });
        }
      }
    });
  }

  // 监控设置
  if (config.watch) {
    { const el = document.getElementById('watch-interval'); if (el) el.value = config.watch.intervalMs || 1000; }
    { const el = document.getElementById('gemini-quiet-ms'); if (el) el.value = config.watch.geminiQuietMs || 3000; }
    { const el = document.getElementById('claude-quiet-ms'); if (el) el.value = config.watch.claudeQuietMs || 60000; }
    { const el = document.getElementById('log-retention-days'); if (el) el.value = config.watch.logRetentionDays || 7; }
  }

  // 确认提醒
  if (config.confirmAlert) {
    { const el = document.getElementById('confirm-alert-enabled'); if (el) el.checked = !!config.confirmAlert.enabled; }
    { const el = document.getElementById('confirm-alert-keywords'); if (el) el.value = (config.confirmAlert.keywords || []).join(','); }
  }

  // UI 设置
  if (config.ui) {
    { const el = document.getElementById('setting-language'); if (el) el.value = config.ui.language || 'zh-CN'; }
    { const el = document.getElementById('setting-close-behavior'); if (el) el.value = config.ui.closeBehavior || 'ask'; }
    { const el = document.getElementById('setting-autostart'); if (el) el.checked = !!config.ui.autostart; }
    { const el = document.getElementById('setting-silent-start'); if (el) el.checked = !!config.ui.silentStart; }
    { const el = document.getElementById('setting-autofocus'); if (el) el.checked = !!config.ui.autoFocusOnNotify; }
    { const el = document.getElementById('setting-sound-type'); if (el) el.value = config.ui.soundType || 'system'; }
    { const el = document.getElementById('setting-tts-template'); if (el) el.value = config.ui.ttsTemplate || '任务完成了'; }
  }
}

// ========== 配置更新 ==========
function updateChannelConfig(channel, enabled) {
  ipcRenderer.send('update-channel-config', { channel, enabled });
  showToast(`已${enabled ? '启用' : '禁用'} ${channel}`, 'success');
  updateStats();
}

function saveTelegramConfig() {
  const config = {
    botToken: document.getElementById('telegram-token')?.value.trim(),
    chatId: document.getElementById('telegram-chat-id')?.value.trim(),
    httpProxy: document.getElementById('telegram-proxy')?.value.trim()
  };
  ipcRenderer.send('update-telegram-config', config);
  showToast('Telegram 配置已保存', 'success');
}

function updateSourceConfig(source, key, value) {
  ipcRenderer.send('update-source-config', { source, key, value });
  showToast(`${source} 配置已更新`, 'success');
}

function updateSourceChannelConfig(source, channel, enabled) {
  ipcRenderer.send('update-source-channel-config', { source, channel, enabled });
  showToast(`${source} - ${channel} 已${enabled ? '启用' : '禁用'}`, 'success');
}

function updateWatchConfig(key, value) {
  ipcRenderer.send('update-watch-config', { key, value });
  showToast('监控设置已保存', 'success');
}

function saveSetting(key, value) {
  ipcRenderer.send('update-setting', { key, value });
  showToast('设置已保存', 'success');
}

// ========== 监控控制 ==========
function toggleWatch() {
  if (state.watchRunning) {
    ipcRenderer.send('stop-watch');
  } else {
    ipcRenderer.send('start-watch');
  }
}

function updateWatchUI(running) {
  const indicator = elements.watchIndicator;
  const statusText = elements.watchStatusText;
  const btn = elements.btnToggleWatch;

  if (running) {
    indicator?.classList.add('active');
    statusText.textContent = '监控运行中';
    btn.textContent = '停止监控';
    btn.classList.remove('btn-primary');
    btn.classList.add('btn-danger');
  } else {
    indicator?.classList.remove('active');
    statusText.textContent = '监控已停止';
    btn.textContent = '启动监控';
    btn.classList.remove('btn-danger');
    btn.classList.add('btn-primary');
  }
}

// ========== 统计更新 ==========
function updateStats() {
  const config = state.config;

  // 启用渠道数
  let channelCount = 0;
  if (config?.channels) {
    if (config.channels.telegram?.enabled) channelCount++;
    if (config.channels.desktop?.enabled) channelCount++;
    if (config.channels.sound?.enabled) channelCount++;
  }
  elements.statActiveChannels.textContent = channelCount;

  // 启用 AI 源数
  let sourceCount = 0;
  if (config?.sources) {
    if (config.sources.claude?.enabled) sourceCount++;
    if (config.sources.codex?.enabled) sourceCount++;
    if (config.sources.gemini?.enabled) sourceCount++;
  }
  elements.statActiveSources.textContent = sourceCount;

  // 今日任务数（从日志计算）
  const today = new Date().toDateString();
  const todayCount = state.logs.filter(log => {
    // 简单计算当天的日志
    return true; // 简化处理
  }).length;
  elements.statTodayTasks.textContent = state.logs.length;

  // 通知数
  elements.statNotifications.textContent = state.logs.filter(l => l.type === 'success').length;
}

// ========== 日志渲染 ==========
function renderLogs(fullList = false) {
  const container = fullList ? elements.fullLogList : elements.recentLogList;
  if (!container) return;

  if (state.logs.length === 0) {
    container.innerHTML = `
      <div class="empty-state" style="padding: var(--space-8);">
        <div class="empty-state-icon">📝</div>
        <div class="empty-state-title">暂无日志</div>
        <div class="empty-state-desc">任务完成后的日志将显示在这里</div>
      </div>
    `;
    return;
  }

  const logsToShow = fullList ? state.logs : state.logs.slice(0, 10);

  container.innerHTML = logsToShow.map(log => `
    <div class="log-item ${log.type}">
      <span class="log-time">${log.time}</span>
      <span class="log-source ${log.source}">${log.source.toUpperCase()}</span>
      <span class="log-message">${escapeHtml(log.message)}</span>
    </div>
  `).join('');
}

function refreshLogs() {
  showToast('日志已刷新', 'success');
}

function clearLogs() {
  state.logs = [];
  renderLogs(true);
  renderLogs(false);
  showToast('日志已清空', 'success');
}

// ========== 测试通知 ==========
function testNotification(channel, source = 'claude') {
  const message = document.getElementById('test-message')?.value || '这是一条测试消息';
  showToast(`正在发送测试通知到 ${channel}...`, 'info');

  ipcRenderer.send('test-notification', { channel, source, message });
}

// ========== Toast 提示 ==========
function showToast(message, type = 'info') {
  if (!elements.toastContainer) return;

  const toast = document.createElement('div');
  toast.className = `toast ${type}`;

  const icons = {
    success: '✓',
    error: '✕',
    warning: '⚠',
    info: 'ℹ'
  };

  toast.innerHTML = `
    <span class="toast-icon">${icons[type] || icons.info}</span>
    <span class="toast-message">${escapeHtml(message)}</span>
  `;

  elements.toastContainer.appendChild(toast);

  setTimeout(() => {
    toast.style.animation = 'toast-in 0.3s ease reverse';
    setTimeout(() => toast.remove(), 300);
  }, 3000);
}

// ========== 工具函数 ==========
function escapeHtml(str) {
  if (!str) return '';
  return str
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

// ========== 启动 ==========
init();
