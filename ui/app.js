const { invoke } = window.__TAURI__.core;

const WATCH_DEFAULTS = {
  sources: 'all',
  interval_ms: 1000,
  claude_quiet_ms: 3000
};

const state = {
  meta: null,
  config: null,
  watchRunning: false
};

async function init() {
  await loadMeta();
  await loadConfig();
  setupEventListeners();
  setupNativeSelects();
  await syncWatchStatus();
}

async function loadMeta() {
  try {
    state.meta = await invoke('get_meta');
    renderMeta();
  } catch (e) {
    console.error('Failed to load meta:', e);
  }
}

function setupEventListeners() {
  document.getElementById('btn-toggle-watch')?.addEventListener('click', toggleWatch);

  ['claude', 'codex', 'pi', 'opencode'].forEach(source => {
    document.getElementById(`source-${source}-enabled`)?.addEventListener('change', (e) => {
      updateSourceConfig(source, 'enabled', e.target.checked);
    });
    document.getElementById(`source-${source}-duration`)?.addEventListener('blur', (e) => {
      updateSourceConfig(source, 'minDurationMinutes', parseInt(e.target.value) || 0);
    });
  });

  document.getElementById('btn-test-desktop')?.addEventListener('click', testNotification);
  document.getElementById('setting-autostart')?.addEventListener('change', (e) => saveSetting('autostart', e.target.checked));
  document.getElementById('setting-silent-start')?.addEventListener('change', (e) => saveSetting('silent_start', e.target.checked));
}

function setupNativeSelects() {
  document.querySelectorAll('.native-select').forEach((root) => {
    if (root.dataset.bound === '1') return;
    root.dataset.bound = '1';

    const trigger = root.querySelector('.native-select-trigger');
    const menu = root.querySelector('.native-select-menu');
    if (!trigger || !menu) return;

    const setOpen = (open) => {
      if (open) {
        closeAllNativeSelects(root);
        root.classList.add('is-open');
        trigger.setAttribute('aria-expanded', 'true');
      } else {
        root.classList.remove('is-open');
        trigger.setAttribute('aria-expanded', 'false');
      }
    };

    trigger.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      setOpen(!root.classList.contains('is-open'));
    });

    trigger.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowDown' || e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        setOpen(true);
        root.querySelector('.native-select-option.is-selected')?.focus();
      } else if (e.key === 'Escape') {
        setOpen(false);
      }
    });

    root.querySelectorAll('.native-select-option').forEach((option) => {
      option.addEventListener('click', (e) => {
        e.preventDefault();
        e.stopPropagation();
        const value = option.dataset.value || '';
        setNativeSelectValue(root, value, { emitChange: true });
        setOpen(false);
        trigger.focus();
      });

      option.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') {
          e.preventDefault();
          setOpen(false);
          trigger.focus();
        } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
          e.preventDefault();
          const options = Array.from(root.querySelectorAll('.native-select-option'));
          const index = options.indexOf(option);
          const next = e.key === 'ArrowDown'
            ? options[Math.min(options.length - 1, index + 1)]
            : options[Math.max(0, index - 1)];
          next?.focus();
        } else if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          option.click();
        }
      });
    });
  });

  if (!document.body.dataset.nativeSelectDocBound) {
    document.body.dataset.nativeSelectDocBound = '1';
    document.addEventListener('click', (e) => {
      if (!(e.target instanceof Element) || !e.target.closest('.native-select')) {
        closeAllNativeSelects();
      }
    });
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') closeAllNativeSelects();
    });
  }
}

function closeAllNativeSelects(except) {
  document.querySelectorAll('.native-select.is-open').forEach((root) => {
    if (except && root === except) return;
    root.classList.remove('is-open');
    root.querySelector('.native-select-trigger')?.setAttribute('aria-expanded', 'false');
  });
}

function setNativeSelectValue(rootOrId, value, { emitChange = false } = {}) {
  const root = typeof rootOrId === 'string'
    ? document.getElementById(rootOrId)
    : rootOrId;
  if (!root) return;

  const nextValue = value || '';
  const prevValue = root.dataset.value || '';
  const valueEl = root.querySelector('.native-select-value');
  const options = Array.from(root.querySelectorAll('.native-select-option'));
  const matched = options.find((option) => option.dataset.value === nextValue) || options[0];
  if (!matched) return;

  root.dataset.value = matched.dataset.value || '';
  if (valueEl) valueEl.textContent = matched.textContent?.trim() || '';

  options.forEach((option) => {
    const selected = option === matched;
    option.classList.toggle('is-selected', selected);
    option.setAttribute('aria-selected', selected ? 'true' : 'false');
  });

  if (emitChange && prevValue !== root.dataset.value) {
    root.dispatchEvent(new CustomEvent('change', {
      bubbles: true,
      detail: { value: root.dataset.value }
    }));
  }
}

function getNativeSelectValue(rootOrId) {
  const root = typeof rootOrId === 'string'
    ? document.getElementById(rootOrId)
    : rootOrId;
  return root?.dataset.value || '';
}

async function loadConfig() {
  try {
    state.config = normalizeConfig(await invoke('get_config'));
    renderConfig();
  } catch (e) {
    console.error('Failed to load config:', e);
  }
}

async function syncWatchStatus() {
  try {
    const status = await invoke('watch_status');
    state.watchRunning = !!(status && status.running);
  } catch (e) {
    console.error('Failed to sync watch status:', e);
    state.watchRunning = false;
  } finally {
    updateWatchStatus();
  }
}

function renderConfig() {
  if (!state.config) return;

  ['claude', 'codex', 'pi', 'opencode'].forEach(source => {
    const cfg = state.config.sources[source];
    const enabledEl = document.getElementById(`source-${source}-enabled`);
    const durationEl = document.getElementById(`source-${source}-duration`);
    if (enabledEl) enabledEl.checked = cfg.enabled;
    if (durationEl) durationEl.value = cfg.min_duration_minutes || 0;
  });

  const langEl = document.getElementById('setting-language');
  const autostartEl = document.getElementById('setting-autostart');
  const silentStartEl = document.getElementById('setting-silent-start');
  if (langEl) setNativeSelectValue(langEl, state.config.ui.language || 'zh-CN');
  if (autostartEl) autostartEl.checked = state.config.ui.autostart || false;
  if (silentStartEl) silentStartEl.checked = state.config.ui.silent_start || false;
}

function renderMeta() {
  const versionEl = document.getElementById('app-version');
  if (!versionEl) return;

  const version = state.meta?.version;
  versionEl.textContent = version ? `Aitify v${version}` : 'Aitify';
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

function normalizeConfig(config) {
  const next = config || {};

  if (!next.ui) next.ui = {};
  if (!next.channels) next.channels = {};
  if (!next.channels.desktop) next.channels.desktop = { enabled: true };

  if (!next.sources) next.sources = {};
  ['claude', 'codex', 'pi', 'opencode'].forEach((source) => {
    if (!next.sources[source]) next.sources[source] = {};
    if (typeof next.sources[source].enabled !== 'boolean') next.sources[source].enabled = true;
    if (typeof next.sources[source].min_duration_minutes !== 'number') next.sources[source].min_duration_minutes = 0;
    if (!next.sources[source].channels) next.sources[source].channels = {};
    if (typeof next.sources[source].channels.desktop !== 'boolean') next.sources[source].channels.desktop = true;
  });

  if (!next.ui.language) next.ui.language = 'zh-CN';
  if (typeof next.ui.autostart !== 'boolean') next.ui.autostart = false;
  if (typeof next.ui.silent_start !== 'boolean') next.ui.silent_start = false;
  if (!next.ui.window || typeof next.ui.window !== 'object') next.ui.window = {};
  if (typeof next.ui.window.width !== 'number') next.ui.window.width = 468;
  if (typeof next.ui.window.height !== 'number') next.ui.window.height = 740;

  return next;
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
      await invoke('start_watch', { payload: WATCH_DEFAULTS });
      state.watchRunning = true;
      updateWatchStatus();
      showToast('监控已启动', 'success');
    }
  } catch (e) {
    await syncWatchStatus();
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
    btnToggleWatch.textContent = state.watchRunning ? '停止' : '启动';
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
  toastContainer?.appendChild(toast);
  setTimeout(() => toast.classList.add('show'), 10);
  setTimeout(() => {
    toast.classList.remove('show');
    setTimeout(() => toast.remove(), 300);
  }, 3000);
}

document.addEventListener('DOMContentLoaded', () => {
  const languageSelect = document.getElementById('setting-language');
  languageSelect?.addEventListener('change', (e) => {
    const value = e.detail?.value || getNativeSelectValue(languageSelect);
    saveSetting('language', value);
  });
  void init();
});
