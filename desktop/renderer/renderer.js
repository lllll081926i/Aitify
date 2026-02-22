// Tauri IPC 适配层
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-shell'
import { open as dialogOpen } from '@tauri-apps/plugin-dialog'

// Tauri IPC 适配层 - 保持与原有 window.completeNotify 兼容
const TauriIPC = {
  async getMeta() {
    return await invoke('get_meta')
  },

  async getConfig() {
    return await invoke('get_config')
  },

  async saveConfig(config) {
    await invoke('save_config_command', { config })
  },

  async watchStatus() {
    return await invoke('watch_status')
  },

  async watchStart(payload) {
    await invoke('watch_start', { payload })
  },

  async watchStop() {
    await invoke('watch_stop')
  },

  async getAutostart() {
    return await invoke('get_autostart')
  },

  async setAutostart(enabled) {
    return await invoke('set_autostart', { enabled })
  },

  async testNotify(payload) {
    return await invoke('test_notify', payload)
  },

  async testSound(payload) {
    return await invoke('test_sound', payload)
  },

  async openPath(path) {
    await invoke('open_path', { path })
  },

  async openWatchLog() {
    const result = await invoke('open_watch_log')
    return { ok: true, path: result }
  },

  async setUiLanguage(language) {
    await invoke('set_ui_language', { language })
  },

  async setCloseBehavior(behavior) {
    await invoke('set_close_behavior', { behavior })
  },

  async respondClosePrompt(payload) {
    await invoke('respond_close_prompt', payload)
  },

  async openExternal(url) {
    await open(url)
  },

  async openSoundFile() {
    const selected = await dialogOpen({
      multiple: false,
      filters: [{
        name: 'Sound Files',
        extensions: ['wav', 'mp3', 'ogg', 'm4a']
      }]
    })
    if (selected) {
      return { path: selected }
    }
    return null
  },

  onWatchLog(callback) {
    return listen('watch-log', (event) => {
      callback(event.payload)
    })
  },

  onClosePrompt(callback) {
    return listen('close-prompt', (event) => {
      callback(event.payload)
    })
  },

  onDismissClosePrompt(callback) {
    return listen('dismiss-close-prompt', (event) => {
      callback(event.payload)
    })
  }
}

// 保持兼容性 - 将 TauriIPC 挂载到 window 上
window.completeNotify = TauriIPC

const CHANNELS = [
  { key: 'telegram', titleKey: 'channel.telegram', descKey: 'channel.telegram.desc' },
  { key: 'desktop', titleKey: 'channel.desktop', descKey: 'channel.desktop.desc' },
  { key: 'sound', titleKey: 'channel.sound', descKey: 'channel.sound.desc' }
]

const SOURCES = [
  { key: 'claude', titleKey: 'source.claude', descKey: 'source.claude.desc' },
  { key: 'codex', titleKey: 'source.codex', descKey: 'source.codex.desc' },
  { key: 'gemini', titleKey: 'source.gemini', descKey: 'source.gemini.desc' }
]

const SUPPORTED_LANGUAGES = ['zh-CN', 'en']

const I18N = {
  'zh-CN': {
    'brand.subtitle': "AI CLI 任务完成提醒",
    'nav.overview': "概览",
    'nav.channels': "通道",
    'nav.sources': "来源",
    'nav.watch': "监听",
    'nav.test': "测试",
    'nav.settings': "设置",
    'ui.language': "语言",
    'ui.watchToggle': "监听",
    'btn.projectLink': "项目地址",
    'btn.openDataDir': "数据目录",
    'btn.openWatchLog': "打开日志",
    'btn.reload': "刷新",
    'btn.save': "保存",
    'btn.watchStart': "开始监听",
    'btn.watchStop': "停止",
    'btn.send': "发送",
    'btn.quickTest': "快速测试",
    'btn.soundTest': "播放测试",
    'btn.soundOpenPath': "选择",
    'section.overview.title': "状态概览",
    'section.overview.sub': "监听状态与通知渠道",
    'section.channels.title': "通知通道",
    'section.channels.sub': "全局开关 + 每来源开关同时生效",
    'section.sources.title': "AI 来源",
    'section.sources.sub': "按来源独立控制：启用、阈值、各通道开关",
    'section.watch.title': "监听配置",
    'section.watch.sub': "自动监听 AI CLI 日志，任务完成后提醒",
    'section.test.title': "测试通知",
    'section.test.sub': "验证通道是否可用（强制发送，不受阈值影响）",
    'section.settings.title': "设置",
    'section.settings.sub': "应用偏好配置",
    'overview.watchStatus': "监听状态",
    'overview.activeChannels': "已启用通道",
    'overview.activeSources': "已启用来源",
    'overview.language': "当前语言",
    'overview.stopped': "未运行",
    'overview.running': "运行中",
    'watch.sources': "监听来源",
    'watch.polling': "轮询间隔",
    'watch.claudeDebounce': "Claude 去抖",
    'watch.debounce': "Gemini 去抖",
    'watch.logRetention': "日志保留",
    'watch.logs': "监听日志",
    'watch.status.running': "运行中",
    'watch.status.stopped': "未运行",
    'watch.confirmEnabled': "确认提醒",
    'watch.hint': "建议把监听常驻开启，这样无论你在终端还是 VSCode 里用 AI，都能自动提醒。",
    'watch.confirmUsageHint': "开启后，当 AI 请求确认时会提醒你。",
    'test.source': "来源",
    'test.duration': "耗时",
    'test.message': "内容",
    'test.defaultTask': "测试提醒（强制发送）",
    'settings.general': "常规",
    'settings.notification': "通知",
    'settings.sound': "提示音",
    'settings.advanced': "高级",
    'settings.days': "天",
    'channel.telegram': "Telegram",
    'channel.telegram.desc': "Bot 消息推送（可选代理）",
    'channel.desktop': "桌面通知",
    'channel.desktop.desc': "Windows 气泡提示",
    'channel.sound': "声音",
    'channel.sound.desc': "语音播报 / 提示音",
    'source.claude': "Claude",
    'source.claude.desc': "Claude Code CLI / 插件",
    'source.codex': "Codex",
    'source.codex.desc': "Codex CLI / 插件",
    'source.gemini': "Gemini",
    'source.gemini.desc': "Gemini CLI / 插件",
    'sources.threshold': "超过提醒",
    'sources.thresholdHint': "当耗时超过此值才会提醒",
    'close.message': "关闭应用？",
    'close.detail': "可选择隐藏到托盘继续运行，或直接退出并停止监听。",
    'close.hide': "隐藏到托盘",
    'close.quit': "退出",
    'close.cancel': "取消",
    'close.remember': "记住我的选择",
    'close.ask': "每次询问",
    'close.tray': "隐藏到托盘",
    'close.exit': "直接退出",
    'advanced.closeBehavior': "关闭行为",
    'advanced.autostart': "开机自启动",
    'advanced.autostartStatusUnknown': "系统自启动：未知",
    'advanced.autostartStatusOn': "系统自启动：已开启",
    'advanced.autostartStatusOff': "系统自启动：未开启",
    'advanced.autostartStatusUnsupported': "系统自启动：不支持",
    'advanced.silentStart': "静默启动",
    'advanced.autoFocus': "点击通知切回",
    'advanced.focusTarget': "切回目标",
    'advanced.forceMaximize': "切回时最大化",
    'advanced.soundTts': "语音播报（TTS）",
    'advanced.soundCustom': "自定义提示音",
    'advanced.soundCustomPath': "提示音路径",
    'advanced.soundCustomPlaceholder': "C:\\path\\to\\sound.wav",
    'focus.auto': "自动判断",
    'focus.vscode': "VSCode",
    'focus.terminal': "终端",
    'watch.logOpenFailed': "打开日志失败"
  },
  en: {
    'brand.subtitle': "AI CLI completion notifications",
    'nav.overview': "Overview",
    'nav.channels': "Channels",
    'nav.sources': "Sources",
    'nav.watch': "Watch",
    'nav.test': "Test",
    'nav.settings': "Settings",
    'ui.language': "Language",
    'ui.watchToggle': "Watch",
    'btn.projectLink': "Project",
    'btn.openDataDir': "Data Dir",
    'btn.openWatchLog': "Log",
    'btn.reload': "Reload",
    'btn.save': "Save",
    'btn.watchStart': "Start Watch",
    'btn.watchStop': "Stop",
    'btn.send': "Send",
    'btn.quickTest': "Quick Test",
    'btn.soundTest': "Play Test",
    'btn.soundOpenPath': "Browse",
    'section.overview.title': "Status Overview",
    'section.overview.sub': "Watch status and notification channels",
    'section.channels.title': "Notification Channels",
    'section.channels.sub': "Global toggle + per-source toggles apply",
    'section.sources.title': "AI Sources",
    'section.sources.sub': "Per-source: enable, threshold, channel toggles",
    'section.watch.title': "Watch Configuration",
    'section.watch.sub': "Auto-watch AI CLI logs and notify on completion",
    'section.test.title': "Test Notification",
    'section.test.sub': "Validate channels (forced send; ignores thresholds)",
    'section.settings.title': "Settings",
    'section.settings.sub': "App preferences",
    'overview.watchStatus': "Watch Status",
    'overview.activeChannels': "Active Channels",
    'overview.activeSources': "Active Sources",
    'overview.language': "Language",
    'overview.stopped': "Stopped",
    'overview.running': "Running",
    'watch.sources': "Watch Sources",
    'watch.polling': "Polling",
    'watch.claudeDebounce': "Claude Debounce",
    'watch.debounce': "Gemini Debounce",
    'watch.logRetention': "Log Retention",
    'watch.logs': "Watch Logs",
    'watch.status.running': "Running",
    'watch.status.stopped': "Stopped",
    'watch.confirmEnabled': "Confirm Alert",
    'watch.hint': "Keep watch running so notifications work for both terminal and VSCode.",
    'watch.confirmUsageHint': "Get alerted when AI needs your confirmation.",
    'test.source': "Source",
    'test.duration': "Duration",
    'test.message': "Message",
    'test.defaultTask': "Test notification (forced)",
    'settings.general': "General",
    'settings.notification': "Notification",
    'settings.sound': "Sound",
    'settings.advanced': "Advanced',
    'settings.days': "days",
    'channel.telegram': "Telegram",
    'channel.telegram.desc': "Bot messages (optional proxy)",
    'channel.desktop': "Desktop",
    'channel.desktop.desc': "Windows toast/balloon",
    'channel.sound': "Sound",
    'channel.sound.desc': "TTS / beep fallback",
    'source.claude': "Claude",
    'source.claude.desc': "Claude Code CLI / extension",
    'source.codex': "Codex",
    'source.codex.desc': "Codex CLI / extension",
    'source.gemini': "Gemini",
    'source.gemini.desc': "Gemini CLI / extension",
    'sources.threshold': "Notify if over",
    'sources.thresholdHint': "Only notify if duration exceeds this value",
    'close.message': "Close the app?",
    'close.detail': "Minimize to tray to keep running, or quit to stop watchers.",
    'close.hide': "Minimize to tray",
    'close.quit': "Quit",
    'close.cancel': "Cancel",
    'close.remember': "Remember my choice",
    'close.ask': "Ask every time",
    'close.tray': "Minimize to tray",
    'close.exit': "Quit app",
    'advanced.closeBehavior': "Close Behavior",
    'advanced.autostart': "Launch at login",
    'advanced.autostartStatusUnknown': "System autostart: unknown",
    'advanced.autostartStatusOn': "System autostart: enabled",
    'advanced.autostartStatusOff': "System autostart: disabled",
    'advanced.autostartStatusUnsupported': "System autostart: unsupported",
    'advanced.silentStart': "Silent start",
    'advanced.autoFocus': "Click notification to return",
    'advanced.focusTarget': "Return target",
    'advanced.forceMaximize': "Force maximize on return",
    'advanced.soundTts': "Voice TTS",
    'advanced.soundCustom': "Custom sound",
    'advanced.soundCustomPath': "Sound file path",
    'advanced.soundCustomPlaceholder': "C:\\path\\to\\sound.wav",
    'focus.auto': "Auto",
    'focus.vscode': "VSCode",
    'focus.terminal': "Terminal",
    'watch.logOpenFailed': "Failed to open log"
  }
}

let currentLanguage = 'zh-CN'
let autostartStatusPayload = null

function normalizeLanguage(value) {
  if (typeof value !== 'string') return 'zh-CN'
  const normalized = value.trim().toLowerCase()
  if (normalized === 'en' || normalized.startsWith('en-')) return 'en'
  if (normalized === 'zh' || normalized.startsWith('zh')) return 'zh-CN'
  return SUPPORTED_LANGUAGES.includes(value) ? value : 'zh-CN'
}

function t(key) {
  const langPack = I18N[currentLanguage] || I18N['zh-CN']
  return langPack[key] || I18N.en[key] || I18N['zh-CN'][key] || String(key)
}

function $(id) {
  return document.getElementById(id)
}

function setHint(text) {
  $('hint').textContent = text || ''
}

function setLog(text) {
  $('log').textContent = text || ''
}

function setWatchLog(text) {
  $('watchLog').textContent = text || ''
}

function setSoundTestStatus(text, tone) {
  const el = $('soundTestStatus')
  if (!el) return
  el.textContent = text || ''
  el.classList.remove('success', 'error')
  if (tone) el.classList.add(tone)
}

function formatLogTimestamp(ts) {
  const date = new Date(ts)
  const pad = (n) => String(n).padStart(2, '0')
  const y = date.getFullYear()
  const m = pad(date.getMonth() + 1)
  const d = pad(date.getDate())
  const hh = pad(date.getHours())
  const mm = pad(date.getMinutes())
  const ss = pad(date.getSeconds())
  return `${y}-${m}-${d} ${hh}:${mm}:${ss}`
}

function appendWatchLog(line) {
  const rawLine = String(line || '')
  const stamped = `[${formatLogTimestamp(Date.now())}] ${rawLine}`
  const next = ($('watchLog').textContent || '') + stamped + '\n'
  $('watchLog').textContent = next.length > 12000 ? next.slice(-12000) : next
  $('watchLog').scrollTop = $('watchLog').scrollHeight
}

function renderAutostartStatus(payload) {
  autostartStatusPayload = payload || autostartStatusPayload
  const el = $('autostartStatus')
  if (!el) return
  const info = autostartStatusPayload || {}
  const platform = String(info.platform || '')
  const system = info.system || null
  if (!platform) {
    el.textContent = t('advanced.autostartStatusUnknown')
    return
  }
  const isSupported = platform === 'win32' || platform === 'darwin'
  if (!isSupported) {
    el.textContent = t('advanced.autostartStatusUnsupported')
    return
  }
  if (system && typeof system.open_at_login === 'boolean') {
    el.textContent = system.open_at_login ? t('advanced.autostartStatusOn') : t('advanced.autostartStatusOff')
    return
  }
  el.textContent = t('advanced.autostartStatusUnknown')
}

function applyLanguageToDom(config, opts = {}) {
  currentLanguage = normalizeLanguage(currentLanguage)
  document.documentElement.lang = currentLanguage === 'en' ? 'en' : 'zh-CN'

  for (const el of document.querySelectorAll('[data-i18n]')) {
    const key = el.getAttribute('data-i18n')
    if (!key) continue
    el.textContent = t(key)
  }
  for (const el of document.querySelectorAll('[data-i18n-title]')) {
    const key = el.getAttribute('data-i18n-title')
    if (!key) continue
    const text = t(key)
    el.setAttribute('title', text)
    el.setAttribute('aria-label', text)
  }
  for (const el of document.querySelectorAll('[data-i18n-placeholder]')) {
    const key = el.getAttribute('data-i18n-placeholder')
    if (!key) continue
    el.setAttribute('placeholder', t(key))
  }

  // Update overview language
  $('overviewLanguage').textContent = currentLanguage === 'en' ? 'English' : '中文'

  if (config) {
    renderGlobalChannels(config)
    renderSources(config)
  }
  renderAutostartStatus(autostartStatusPayload)
  setSoundTestStatus('')
}

function setupNav() {
  const navLinks = Array.from(document.querySelectorAll('.navItem'))
  const contentRoot = document.querySelector('.content')

  if (navLinks.length === 0) return () => {}

  function setActiveByHash(hash) {
    const targetHash = hash && hash.startsWith('#') ? hash : navLinks[0].getAttribute('href') || ''
    for (const link of navLinks) {
      link.classList.toggle('isActive', link.getAttribute('href') === targetHash)
    }
  }

  for (const link of navLinks) {
    link.addEventListener('click', () => setActiveByHash(link.getAttribute('href')))
  }

  window.addEventListener('hashchange', () => setActiveByHash(window.location.hash))
  setActiveByHash(window.location.hash)

  const sections = navLinks
    .map((l) => document.querySelector(l.getAttribute('href') || ''))
    .filter(Boolean)

  if (!contentRoot || sections.length === 0 || typeof IntersectionObserver !== 'function') {
    return () => {}
  }

  const observer = new IntersectionObserver(
    (entries) => {
      const visible = entries.filter((e) => e.isIntersecting)
      if (visible.length === 0) return
      visible.sort((a, b) => b.intersectionRatio - a.intersectionRatio)
      const top = visible[0].target
      if (top && top.id) setActiveByHash('#' + top.id)
    },
    { root: contentRoot, threshold: [0.18, 0.26, 0.35, 0.45, 0.6] }
  )

  for (const section of sections) observer.observe(section)
  return () => observer.disconnect()
}

function createSwitch(checked, onChange) {
  const label = document.createElement('label')
  label.className = 'switch'

  const input = document.createElement('input')
  input.type = 'checkbox'
  input.checked = Boolean(checked)
  input.addEventListener('change', () => onChange(input.checked))

  const slider = document.createElement('span')
  slider.className = 'slider'

  label.appendChild(input)
  label.appendChild(slider)
  return { root: label, input }
}

function renderGlobalChannels(config) {
  const root = $('globalChannels')
  root.innerHTML = ''

  for (const ch of CHANNELS) {
    const card = document.createElement('div')
    card.className = 'channelCard'

    const info = document.createElement('div')
    info.className = 'channelInfo'

    const name = document.createElement('div')
    name.className = 'channelName'
    name.textContent = t(ch.titleKey)

    const desc = document.createElement('div')
    desc.className = 'channelDesc'
    desc.textContent = t(ch.descKey)

    info.appendChild(name)
    info.appendChild(desc)

    const toggle = createSwitch(config.channels?.[ch.key]?.enabled, (v) => {
      config.channels[ch.key].enabled = v
      updateOverviewChannels(config)
      saveConfigDebounced(config)
    })

    card.appendChild(info)
    card.appendChild(toggle.root)
    root.appendChild(card)
  }
}

function renderSources(config) {
  const root = $('sources')
  root.innerHTML = ''

  for (const src of SOURCES) {
    const card = document.createElement('div')
    card.className = 'sourceCard'

    const header = document.createElement('div')
    header.className = 'sourceHeader'

    const left = document.createElement('div')
    const name = document.createElement('div')
    name.className = 'sourceName'
    name.textContent = t(src.titleKey)
    const desc = document.createElement('div')
    desc.className = 'sourceDesc'
    desc.textContent = t(src.descKey)
    left.appendChild(name)
    left.appendChild(desc)

    const controls = document.createElement('div')
    controls.className = 'sourceControls'

    const thresholdLabel = document.createElement('div')
    thresholdLabel.className = 'labelWithHint'
    thresholdLabel.innerHTML = `
      <span>${t('sources.threshold')}</span>
      <span class="hintIcon" title="${t('sources.thresholdHint')}">?</span>
    `

    const threshold = document.createElement('div')
    threshold.className = 'numberField'
    threshold.innerHTML = `
      <input type="number" min="0" step="1" value="${config.sources?.[src.key]?.min_duration_minutes ?? 0}" />
      <span class="numberFieldUnit">min</span>
    `
    const thresholdInput = threshold.querySelector('input')
    thresholdInput.addEventListener('change', () => {
      const n = Number(thresholdInput.value)
      config.sources[src.key].min_duration_minutes = Number.isFinite(n) && n >= 0 ? n : 0
      saveConfigDebounced(config)
    })

    const enabledToggle = createSwitch(config.sources?.[src.key]?.enabled, (v) => {
      config.sources[src.key].enabled = v
      disabledWrap.classList.toggle('isDisabled', !v)
      updateOverviewSources(config)
      saveConfigDebounced(config)
    })

    controls.appendChild(thresholdLabel)
    controls.appendChild(threshold)
    controls.appendChild(enabledToggle.root)

    header.appendChild(left)
    header.appendChild(controls)
    card.appendChild(header)

    const disabledWrap = document.createElement('div')
    disabledWrap.className = 'sourceChannels'
    disabledWrap.classList.toggle('isDisabled', !config.sources?.[src.key]?.enabled)

    for (const ch of CHANNELS) {
      const item = document.createElement('label')
      item.className = 'sourceChannelItem'

      const checkbox = document.createElement('input')
      checkbox.type = 'checkbox'
      checkbox.checked = Boolean(config.sources?.[src.key]?.channels?.[ch.key])
      checkbox.addEventListener('change', () => {
        config.sources[src.key].channels[ch.key] = checkbox.checked
        updateOverviewChannels(config)
        saveConfigDebounced(config)
      })

      item.appendChild(checkbox)
      item.appendChild(document.createTextNode(t(ch.titleKey)))
      disabledWrap.appendChild(item)
    }

    card.appendChild(disabledWrap)
    root.appendChild(card)
  }
}

function updateOverviewChannels(config) {
  const el = $('overviewChannels')
  if (!el) return
  const active = CHANNELS.filter(ch => config.channels?.[ch.key]?.enabled).map(ch => t(ch.titleKey))
  el.textContent = active.length ? active.join(', ') : '-'
}

function updateOverviewSources(config) {
  const el = $('overviewSources')
  if (!el) return
  const active = SOURCES.filter(src => config.sources?.[src.key]?.enabled).map(src => t(src.titleKey))
  el.textContent = active.length ? active.join(', ') : '-'
}

function bindClosePrompt() {
  const modal = $('closeModal')
  if (!modal) return () => {}

  let activeId = null
  let promptEpoch = 0
  let suppressUntil = 0

  const setOpen = (open) => {
    modal.classList.toggle('isOpen', open)
    modal.setAttribute('aria-hidden', open ? 'false' : 'true')
    if (open) {
      const remember = $('closeRemember')
      if (remember) remember.checked = false
      const hideBtn = $('closeHideBtn')
      if (hideBtn) hideBtn.focus()
    }
  }

  const dismiss = (payload) => {
    const nextEpoch = payload && Number.isFinite(Number(payload.epoch))
      ? Number(payload.epoch)
      : promptEpoch + 1
    promptEpoch = Math.max(promptEpoch, nextEpoch)
    activeId = null
    suppressUntil = Date.now() + 600
    modal.classList.add('isForceHidden')
    setOpen(false)
    setTimeout(() => {
      if (!modal.classList.contains('isOpen')) modal.classList.remove('isForceHidden')
    }, 160)
  }

  const respond = (action) => {
    if (!activeId) {
      setOpen(false)
      return
    }
    const payload = {
      id: activeId,
      action,
      remember: Boolean($('closeRemember')?.checked)
    }
    activeId = null
    if (window.completeNotify && typeof window.completeNotify.respondClosePrompt === 'function') {
      window.completeNotify.respondClosePrompt(payload)
    }
    setOpen(false)
  }

  const onRequest = (payload) => {
    if (Date.now() < suppressUntil) return
    const incomingEpoch = payload && Number.isFinite(Number(payload.epoch))
      ? Number(payload.epoch)
      : promptEpoch
    if (incomingEpoch < promptEpoch) return
    promptEpoch = incomingEpoch
    const id = payload && payload.id ? String(payload.id) : ''
    if (!id) return
    activeId = id
    setOpen(true)
  }

  const onMaskClick = (event) => {
    if (event.target === modal) respond('cancel')
  }

  const onKeydown = (event) => {
    if (event.key === 'Escape' && modal.classList.contains('isOpen')) {
      respond('cancel')
    }
  }

  const hideBtn = $('closeHideBtn')
  const quitBtn = $('closeQuitBtn')
  const cancelBtn = $('closeCancelBtn')

  const onHideClick = () => respond('tray')
  const onQuitClick = () => respond('exit')
  const onCancelClick = () => respond('cancel')

  if (hideBtn) hideBtn.addEventListener('click', onHideClick)
  if (quitBtn) quitBtn.addEventListener('click', onQuitClick)
  if (cancelBtn) cancelBtn.addEventListener('click', onCancelClick)
  modal.addEventListener('click', onMaskClick)
  window.addEventListener('keydown', onKeydown)

  const unsubscribe = window.completeNotify && typeof window.completeNotify.onClosePrompt === 'function'
    ? window.completeNotify.onClosePrompt(onRequest)
    : () => {}
  const unsubscribeDismiss = window.completeNotify && typeof window.completeNotify.onDismissClosePrompt === 'function'
    ? window.completeNotify.onDismissClosePrompt(dismiss)
    : () => {}

  return () => {
    if (hideBtn) hideBtn.removeEventListener('click', onHideClick)
    if (quitBtn) quitBtn.removeEventListener('click', onQuitClick)
    if (cancelBtn) cancelBtn.removeEventListener('click', onCancelClick)
    modal.removeEventListener('click', onMaskClick)
    window.removeEventListener('keydown', onKeydown)
    if (typeof unsubscribe === 'function') unsubscribe()
    if (typeof unsubscribeDismiss === 'function') unsubscribeDismiss()
  }
}

let configSaveTimer = null
async function saveConfigDebounced(config) {
  clearTimeout(configSaveTimer)
  configSaveTimer = setTimeout(async () => {
    try {
      await window.completeNotify.saveConfig(config)
    } catch (error) {
      setHint(String(error?.message || error))
    }
  }, 250)
}

async function refreshWatchStatus() {
  try {
    const status = await window.completeNotify.watchStatus()
    const running = Boolean(status && status.running)
    const statusEl = $('watchStatus')
    const overviewStatusEl = $('overviewWatchStatus')

    statusEl.textContent = running ? t('watch.status.running') : t('watch.status.stopped')
    statusEl.classList.toggle('on', running)

    // Update overview
    const dot = overviewStatusEl.querySelector('.statusDot')
    const text = overviewStatusEl.querySelector('span:last-child')
    if (dot && text) {
      dot.classList.toggle('on', running)
      text.textContent = running ? t('overview.running') : t('overview.stopped')
    }

    $('watchStartBtn').disabled = running
    $('watchStopBtn').disabled = !running
    $('quickWatchToggle').disabled = running

    const logEl = $('watchLog')
    if (running && logEl && !logEl.textContent.trim()) {
      setWatchLog(currentLanguage === 'en' ? '[watch] running...' : '[watch] 运行中...')
    }
  } catch (error) {
    $('watchStatus').textContent = String(error?.message || error)
    $('watchStatus').classList.remove('on')
  }
}

function buildWatchPayloadFromUi() {
  const sources = []
  if ($('watchClaude') && $('watchClaude').checked) sources.push('claude')
  if ($('watchCodex') && $('watchCodex').checked) sources.push('codex')
  if ($('watchGemini') && $('watchGemini').checked) sources.push('gemini')
  const geminiQuietMs = Number($('watchGeminiQuietMs')?.value || 3000)
  const claudeQuietMs = Number($('watchClaudeQuietMs')?.value || 60000)
  return {
    sources: sources.length ? sources.join(',') : 'all',
    intervalMs: Number($('watchIntervalMs')?.value || 1000),
    geminiQuietMs,
    claudeQuietMs
  }
}

async function main() {
  const cleanupNav = setupNav()
  const cleanupClosePrompt = bindClosePrompt()

  const meta = await window.completeNotify.getMeta()
  $('productName').textContent = meta.product_name
  if (meta.version) $('productVersion').textContent = `v${meta.version}`

  $('openDataDir').addEventListener('click', () => window.completeNotify.openPath(meta.data_dir))

  $('reloadBtn').addEventListener('click', async () => {
    $('reloadBtn').disabled = true
    try {
      const latest = await window.completeNotify.getConfig()
      location.reload()
    } catch (error) {
      setHint(String(error?.message || error))
    } finally {
      $('reloadBtn').disabled = false
    }
  })

  $('githubBtn').addEventListener('click', () => {
    try {
      window.completeNotify.openExternal('https://github.com/ZekerTop/ai-cli-complete-notify')
    } catch (_error) {
      // ignore
    }
  })

  const config = await window.completeNotify.getConfig()
  config.ui = config.ui || {}
  currentLanguage = normalizeLanguage(config.ui.language || 'zh-CN')
  config.ui.language = currentLanguage

  // Sound settings
  const soundCfg = config.channels?.sound || {}
  if (!config.channels) config.channels = soundCfg
  if (!config.channels.sound) config.channels.sound = soundCfg

  const soundTtsToggle = $("soundTtsEnabled")
  const soundCustomToggle = $("soundCustomEnabled")
  const soundCustomRow = $("soundCustomRow")
  const soundCustomPath = $("soundCustomPath")
  const soundOpenPathBtn = $("soundOpenPathBtn")
  const soundTestBtn = $("soundTestBtn")

  const syncSoundUi = () => {
    const customEnabled = Boolean(soundCustomToggle && soundCustomToggle.checked)
    if (soundCustomRow) soundCustomRow.classList.toggle("isHidden", !customEnabled)
  }

  if (soundTtsToggle) {
    if (typeof soundCfg.tts !== "boolean") soundCfg.tts = true
    soundTtsToggle.checked = Boolean(soundCfg.tts)
    soundTtsToggle.addEventListener("change", () => {
      soundCfg.tts = Boolean(soundTtsToggle.checked)
      saveConfigDebounced(config)
    })
  }

  if (soundCustomToggle) {
    if (typeof soundCfg.use_custom !== "boolean") soundCfg.use_custom = false
    soundCustomToggle.checked = Boolean(soundCfg.use_custom)
    soundCustomToggle.addEventListener("change", () => {
      soundCfg.use_custom = Boolean(soundCustomToggle.checked)
      syncSoundUi()
      saveConfigDebounced(config)
    })
  }

  if (soundCustomPath) {
    soundCustomPath.value = String(soundCfg.custom_path || "")
    soundCustomPath.addEventListener("change", () => {
      soundCfg.custom_path = String(soundCustomPath.value || "").trim()
      saveConfigDebounced(config)
    })
  }

  if (soundOpenPathBtn) {
    soundOpenPathBtn.addEventListener("click", async () => {
      if (!window.completeNotify || typeof window.completeNotify.openSoundFile !== "function") {
        return
      }
      try {
        const result = await window.completeNotify.openSoundFile()
        if (result && result.path && soundCustomPath) {
          soundCustomPath.value = String(result.path || "")
          soundCfg.custom_path = String(result.path || "")
          if (soundCustomToggle) {
            soundCustomToggle.checked = true
            soundCfg.use_custom = true
            syncSoundUi()
          }
          saveConfigDebounced(config)
        }
      } catch (error) {
        // ignore
      }
    })
  }

  if (soundTestBtn) {
    soundTestBtn.addEventListener("click", async () => {
      soundTestBtn.disabled = true
      const title = currentLanguage === "en" ? "Sound test" : "提示音测试"
      setSoundTestStatus(currentLanguage === "en" ? "Playing..." : "正在播放...")
      try {
        const result = await window.completeNotify.testSound({
          title,
          sound: {
            enabled: true,
            tts: Boolean(soundTtsToggle && soundTtsToggle.checked),
            use_custom: Boolean(soundCustomToggle && soundCustomToggle.checked),
            custom_path: String(soundCustomPath?.value || "").trim(),
            fallback_beep: Boolean(soundCfg.fallback_beep)
          }
        })
        if (result && result.ok) {
          setSoundTestStatus(currentLanguage === "en" ? "Played" : "已播放", "success")
        } else {
          setSoundTestStatus(currentLanguage === "en" ? "Failed" : "失败", "error")
        }
      } catch (error) {
        setSoundTestStatus(currentLanguage === "en" ? "Error" : "错误", "error")
      } finally {
        soundTestBtn.disabled = false
      }
    })
  }

  syncSoundUi()

  // Watch settings
  if ($('watchLogRetentionDays')) {
    const days = Number(config?.ui?.watch_log_retention_days)
    $('watchLogRetentionDays').value = String(Number.isFinite(days) && days >= 1 ? days : 7)
    $('watchLogRetentionDays').addEventListener('change', () => {
      config.ui.watch_log_retention_days = Number($('watchLogRetentionDays').value)
      saveConfigDebounced(config)
    })
  }

  if ($('watchConfirmEnabled')) {
    if (!config.ui.confirm_alert) config.ui.confirm_alert = {}
    $('watchConfirmEnabled').checked = Boolean(config.ui.confirm_alert.enabled)
    $('watchConfirmEnabled').addEventListener('change', () => {
      config.ui.confirm_alert.enabled = Boolean($('watchConfirmEnabled').checked)
      saveConfigDebounced(config)
    })
  }

  // Settings
  $('languageSelect').value = currentLanguage
  $('languageSelect').addEventListener('change', async () => {
    const next = normalizeLanguage(String($('languageSelect').value || 'zh-CN'))
    if (next === currentLanguage) return
    currentLanguage = next
    config.ui.language = next
    try {
      await window.completeNotify.setUiLanguage(next)
    } catch (_error) {}
    applyLanguageToDom(config)
    saveConfigDebounced(config)
    await refreshWatchStatus()
  })

  // Close behavior
  const closeBehavior = ['ask', 'tray', 'exit'].includes(String(config.ui.close_behavior)) ? String(config.ui.close_behavior) : 'ask'
  $('closeBehavior').value = closeBehavior
  $('closeBehavior').addEventListener('change', async () => {
    const next = String($('closeBehavior').value || 'ask')
    config.ui.close_behavior = ['ask', 'tray', 'exit'].includes(next) ? next : 'ask'
    try {
      if (typeof window.completeNotify.setCloseBehavior === 'function') {
        await window.completeNotify.setCloseBehavior(config.ui.close_behavior)
      }
      saveConfigDebounced(config)
    } catch (_error) {}
  })

  // Auto focus
  const focusToggle = $('autoFocusOnNotify')
  const focusTargetRow = $('focusTargetRow')
  const focusTargetEl = $('focusTarget')
  const forceMaximizeRow = $('forceMaximizeRow')
  const forceMaximizeToggle = $('forceMaximizeOnFocus')

  const syncFocusUi = () => {
    const enabled = Boolean(focusToggle && focusToggle.checked)
    if (focusTargetRow) focusTargetRow.classList.toggle('isHidden', !enabled)
    if (forceMaximizeRow) forceMaximizeRow.classList.toggle('isHidden', !enabled)
  }

  if (focusToggle) {
    focusToggle.checked = Boolean(config.ui.auto_focus_on_notify)
    focusToggle.addEventListener('change', () => {
      config.ui.auto_focus_on_notify = Boolean(focusToggle.checked)
      syncFocusUi()
      saveConfigDebounced(config)
    })
  }

  if (forceMaximizeToggle) {
    if (typeof config.ui.force_maximize_on_focus !== 'boolean') config.ui.force_maximize_on_focus = false
    forceMaximizeToggle.checked = Boolean(config.ui.force_maximize_on_focus)
    forceMaximizeToggle.addEventListener('change', () => {
      config.ui.force_maximize_on_focus = Boolean(forceMaximizeToggle.checked)
      saveConfigDebounced(config)
    })
  }

  if (focusTargetEl) {
    const focusTargets = ['auto', 'vscode', 'terminal']
    const initialTarget = focusTargets.includes(String(config.ui.focus_target)) ? String(config.ui.focus_target) : 'auto'
    config.ui.focus_target = initialTarget
    focusTargetEl.value = initialTarget
    focusTargetEl.addEventListener('change', () => {
      const next = String(focusTargetEl.value || 'auto')
      config.ui.focus_target = focusTargets.includes(next) ? next : 'auto'
      saveConfigDebounced(config)
    })
  }
  syncFocusUi()

  // Autostart
  if ($('autostart')) {
    try {
      const state = await window.completeNotify.getAutostart()
      if (state && typeof state.autostart === 'boolean') {
        config.ui.autostart = state.autostart
      }
      renderAutostartStatus(state)
    } catch (_error) {
      renderAutostartStatus({ platform: '' })
    }
    $('autostart').checked = Boolean(config.ui.autostart)
    $('autostart').addEventListener('change', async () => {
      $('autostart').disabled = true
      const enabled = Boolean($('autostart').checked)
      try {
        const result = await window.completeNotify.setAutostart(enabled)
        if (result && result.ok) {
          config.ui.autostart = enabled
          renderAutostartStatus(result)
          saveConfigDebounced(config)
        } else if (result && result.error) {
          setHint(String(result.error))
          renderAutostartStatus(result)
        }
      } catch (error) {
        setHint(String(error?.message || error))
        renderAutostartStatus({ platform: '' })
      } finally {
        $('autostart').disabled = false
      }
    })
  }

  // Silent start
  const silentStartToggle = $('silentStart')
  if (silentStartToggle) {
    if (typeof config.ui.silent_start !== 'boolean') config.ui.silent_start = false
    silentStartToggle.checked = Boolean(config.ui.silent_start)
    silentStartToggle.addEventListener('change', () => {
      config.ui.silent_start = Boolean(silentStartToggle.checked)
      saveConfigDebounced(config)
    })
  }

  applyLanguageToDom(config)
  updateOverviewChannels(config)
  updateOverviewSources(config)

  // Quick actions
  $('quickWatchToggle').addEventListener('click', async () => {
    const running = !$('watchStartBtn').disabled
    if (!running) {
      try {
        await window.completeNotify.watchStart(buildWatchPayloadFromUi())
      } catch (error) {
        setHint(String(error?.message || error))
      } finally {
        await refreshWatchStatus()
      }
    }
  })

  $('quickTestBtn').addEventListener('click', () => {
    window.location.hash = '#test'
    $('testTask').focus()
  })

  // Watch toggle
  let unsubscribeWatchLog = null
  try {
    const unlisten = await window.completeNotify.onWatchLog((line) => appendWatchLog(line))
    unsubscribeWatchLog = unlisten
  } catch (_error) {}

  $('watchStartBtn').addEventListener('click', async () => {
    $('watchStartBtn').disabled = true
    try {
      await window.completeNotify.watchStart(buildWatchPayloadFromUi())
    } catch (error) {
      setHint(String(error?.message || error))
    } finally {
      await refreshWatchStatus()
    }
  })

  $('watchStopBtn').addEventListener('click', async () => {
    $('watchStopBtn').disabled = true
    try {
      await window.completeNotify.watchStop()
    } catch (error) {
      setHint(String(error?.message || error))
    } finally {
      await refreshWatchStatus()
    }
  })

  if ($('openWatchLogBtn')) {
    $('openWatchLogBtn').addEventListener('click', async () => {
      try {
        if (typeof window.completeNotify.openWatchLog !== 'function') return
        const result = await window.completeNotify.openWatchLog()
        if (!result || !result.ok) {
          setHint(t('watch.logOpenFailed'))
        }
      } catch (_error) {}
    })
  }

  // Test notification
  $('testBtn').addEventListener('click', async () => {
    $('testBtn').disabled = true
    setLog(currentLanguage === 'en' ? 'Sending test notification...' : '发送测试通知中...')
    try {
      const payload = {
        source: $('testSource').value,
        task_info: $('testTask').value || t('test.defaultTask'),
        duration_minutes: Number($('testDuration').value || 0)
      }
      const result = await window.completeNotify.testNotify(payload)
      setLog(JSON.stringify(result, null, 2))
    } catch (error) {
      setLog(String(error?.message || error))
    } finally {
      $('testBtn').disabled = false
    }
  })

  setWatchLog('')
  await refreshWatchStatus()
  setHint('')

  window.addEventListener('beforeunload', () => {
    if (typeof unsubscribeWatchLog === 'function') unsubscribeWatchLog()
    if (typeof cleanupClosePrompt === 'function') cleanupClosePrompt()
    if (typeof cleanupNav === 'function') cleanupNav()
  })
}

main().catch((error) => {
  setHint(String(error?.message || error))
})
