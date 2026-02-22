const { invoke } = window.__TAURI__.core;

const state = {
  config: null,
  watchRunning: false
};

function init() {
  loadConfig();
  setupEventListeners();
}

function setupEventListeners() {
  document.getElementById('btn-toggle-watch')?.addEventListener('click', toggleWatch);

  ['claude', 'codex', 'gemini'].forEach(source => {
    document.getElementById(`source-${source}-enabled`)?.addEventListener('change', (e) => {
      updateSourceConfig(source, 'enabled', e.target.checked);
    });
    document.getElementById(`source-${source}-duration`)?.addEventListener('blur', (e) => {
      updateSourceConfig(source, 'minDurationMinutes', parseInt(e.target.value) || 0);
    });
  });

  document.getElementById('btn-test-desktop')?.addEventListener('click', testNotification);
  document.getElementById('setting-language')?.addEventListener('change', (e) => saveSetting('language', e.target.value));
  document.getElementById('setting-autostart')?.addEventListener('change', (e) => saveSetting('autostart', e.target.checked));
}

async function loadConfig() {
  try {
    state.config = await invoke('get_config');
    renderConfig();
  } catch (e) {
    console.error('Failed to load config:', e);
  }
}

function renderConfig() {
  if (!state.config) return;

  ['claude', 'codex', 'gemini'].forEach(source => {
    const cfg = state.config.sources[source];
    const enabledEl = document.getElementById(`source-${source}-enabled`);
    const durationEl = document.getElementById(`source-${source}-duration`);
    if (enabledEl) enabledEl.checked = cfg.enabled;
    if (durationEl) durationEl.value = cfg.min_duration_minutes || 0;
  });

  const langEl = document.getElementById('setting-language');
  const autostartEl = document.getElementById('setting-autostart');
  if (langEl) langEl.value = state.config.ui.language || 'zh-CN';
  if (autostartEl) autostartEl.checked = state.config.ui.autostart || false;
}

function updateSourceConfig(source, field, value) {
  if (!state.config) return;
  if (field === 'enabled') {
    state.config.sources[source].enabled = value;
  } else if (field === 'minDurationMinutes') {
    state.config.sources[source].min_duration_minutes = value;
  }
  saveConfig();
}

function saveSetting(field, value) {
  if (!state.config) return;
  state.config.ui[field] = value;
  saveConfig();
}

async function saveConfig() {
  try {
    await invoke('save_config', { config: state.config });
    showToast('配置已保存', 'success');
  } catch (e) {
    showToast('保存失败', 'error');
  }
}

async function toggleWatch() {
  try {
    if (state.watchRunning) {
      await invoke('stop_watch');
      state.watchRunning = false;
      updateWatchStatus();
      showToast('监控已停止', 'info');
    } else {
      await invoke('start_watch', { payload: { sources: 'all', interval_ms: 1000, gemini_quiet_ms: 3000, claude_quiet_ms: 60000 } });
      state.watchRunning = true;
      updateWatchStatus();
      showToast('监控已启动', 'success');
    }
  } catch (e) {
    showToast('操作失败', 'error');
  }
}

function updateWatchStatus() {
  const watchIndicator = document.getElementById('watch-indicator');
  const watchStatusText = document.getElementById('watch-status-text');
  const btnToggleWatch = document.getElementById('btn-toggle-watch');

  if (watchIndicator) {
    watchIndicator.className = state.watchRunning ? 'status-dot active' : 'status-dot';
  }
  if (watchStatusText) {
    watchStatusText.textContent = state.watchRunning ? '监控运行中' : '监控已停止';
  }
  if (btnToggleWatch) {
    btnToggleWatch.textContent = state.watchRunning ? '停止监控' : '启动监控';
  }
}

async function testNotification() {
  try {
    await invoke('test_notification', { payload: { source: 'claude', task_info: '这是一条测试通知', duration_minutes: null } });
    showToast('测试通知已发送', 'success');
  } catch (e) {
    showToast('测试通知失败', 'error');
  }
}

function showToast(message, type = 'info') {
  const toastContainer = document.getElementById('toast-container');
  const toast = document.createElement('div');
  toast.className = `toast toast-${type}`;
  toast.textContent = message;
  toastContainer.appendChild(toast);
  setTimeout(() => toast.classList.add('show'), 10);
  setTimeout(() => {
    toast.classList.remove('show');
    setTimeout(() => toast.remove(), 300);
  }, 3000);
}

document.addEventListener('DOMContentLoaded', init);
