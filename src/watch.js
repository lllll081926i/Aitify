const fs = require('fs');
const os = require('os');
const path = require('path');

const { sendNotifications } = require('./engine');

function parseTimestamp(value) {
  if (typeof value === 'number' && Number.isFinite(value)) {
    return value < 1e12 ? value * 1000 : value;
  }
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (/^\d+(\.\d+)?$/.test(trimmed)) {
    const num = Number(trimmed);
    if (!Number.isFinite(num)) return null;
    return num < 1e12 ? num * 1000 : num;
  }
  const parsed = Date.parse(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}

function hasContentType(message, expectedType) {
  if (!message || typeof message !== 'object') return false;
  if (!Array.isArray(message.content)) return false;
  return message.content.some((item) => item && item.type === expectedType);
}

function extractTextFromAny(value) {
  if (!value) return '';
  if (typeof value === 'string') return value;
  if (Array.isArray(value)) {
    return value.map(extractTextFromAny).filter(Boolean).join('\n');
  }
  if (typeof value === 'object') {
    if (typeof value.text === 'string') return value.text;
    if (typeof value.content === 'string') return value.content;
    if (typeof value.message === 'string') return value.message;
    if (typeof value.value === 'string') return value.value;
    if (typeof value.data === 'string') return value.data;
    if (Array.isArray(value.content)) return extractTextFromAny(value.content);
    if (Array.isArray(value.parts)) return extractTextFromAny(value.parts);
    if (Array.isArray(value.messages)) return extractTextFromAny(value.messages);
  }
  return '';
}

function extractMessageText(message) {
  if (!message) return '';
  if (Array.isArray(message.content)) return extractTextFromAny(message.content);
  if (typeof message.content === 'string') return message.content;
  return extractTextFromAny(message);
}

const DEFAULT_CONFIRM_KEYWORDS = [
  '是否',
  '要不要',
  '能否',
  '可否',
  '可以吗',
  '可以么',
  '请确认',
  '确认一下',
  '是否确认',
  '是否继续',
  '同意',
  '允许',
  '授权',
  '批准',
  'confirm',
  'confirmation',
  'approve',
  'approval',
  'okay to',
  'is it ok',
  'is it okay',
  'shall i',
  'should i',
  'would you like',
  'do you want me',
  'may i',
  'permission',
  'allow',
  'authorize',
  'await your',
  'waiting for your'
];
const CONFIRM_DEDUPE_MS = 15000;

function normalizeConfirmText(text) {
  return String(text || '').replace(/\s+/g, ' ').trim();
}

function truncateText(text, maxLength) {
  const value = String(text || '').trim();
  if (!value) return '';
  if (value.length <= maxLength) return value;
  return value.slice(0, Math.max(0, maxLength - 1)) + '...';
}

function buildConfirmTaskInfo(_text) {
  return '确认提醒';
}

function hasLineEndingQuestionMark(text) {
  const raw = String(text || '');
  if (!raw.trim()) return false;
  const lines = raw.split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    if (/[?？]$/.test(trimmed)) return true;
  }
  return false;
}

function createConfirmDetector(confirmAlert) {
  const enabled =
    confirmAlert && Object.prototype.hasOwnProperty.call(confirmAlert, 'enabled')
      ? Boolean(confirmAlert.enabled)
      : false;
  if (!enabled) return () => '';

  // Intentionally disable text-based heuristics (question marks / keywords):
  // - Codex/Claude often stream partial text that contains “是否/？” which caused premature confirm alerts,
  //   and then suppressed the real `task_complete` completion alert.
  // - We only send confirm alerts for explicit interactive prompts (Codex `request_user_input`).
  return () => '';
}

function createConfirmDetectorDynamic(confirmAlertOrGetter) {
  const getter = typeof confirmAlertOrGetter === 'function'
    ? confirmAlertOrGetter
    : () => confirmAlertOrGetter;

  let lastEnabled = null;
  let detector = null;

  function refresh() {
    const current = getter ? getter() : null;
    const enabled = current && Object.prototype.hasOwnProperty.call(current, 'enabled')
      ? Boolean(current.enabled)
      : false;

    if (detector && enabled === lastEnabled) return detector;
    lastEnabled = enabled;
    detector = createConfirmDetector({ enabled });
    return detector;
  }

  const wrapper = (text) => refresh()(text);
  wrapper.isEnabled = () => {
    refresh();
    return Boolean(lastEnabled);
  };
  return wrapper;
}

function safeJsonParse(line) {
  try {
    const normalized = typeof line === 'string' ? line.replace(/^\uFEFF/, '') : '';
    return JSON.parse(normalized);
  } catch (_error) {
    return null;
  }
}

function safeStat(filePath) {
  try {
    return fs.statSync(filePath);
  } catch (_error) {
    return null;
  }
}

function readFileSliceUtf8(filePath, start, length) {
  const fd = fs.openSync(filePath, 'r');
  try {
    const buffer = Buffer.alloc(Math.max(0, length));
    const bytesRead = fs.readSync(fd, buffer, 0, buffer.length, Math.max(0, start));
    return buffer.slice(0, bytesRead).toString('utf8');
  } finally {
    fs.closeSync(fd);
  }
}

function findLatestFile(rootDir, isCandidate) {
  let latest = null;

  function walk(dir) {
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch (_error) {
      return;
    }

    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(fullPath);
        continue;
      }
      if (!entry.isFile()) continue;
      if (!isCandidate(fullPath, entry.name)) continue;

      const stat = safeStat(fullPath);
      if (!stat) continue;
      if (!latest || stat.mtimeMs > latest.mtimeMs) {
        latest = { path: fullPath, mtimeMs: stat.mtimeMs, size: stat.size };
      }
    }
  }

  walk(rootDir);
  return latest;
}

function findLatestFiles(rootDir, isCandidate, limit) {
  const max = Number.isFinite(Number(limit)) ? Math.max(1, Number(limit)) : 1;
  const latest = [];

  function push(item) {
    latest.push(item);
    latest.sort((a, b) => b.mtimeMs - a.mtimeMs);
    if (latest.length > max) latest.length = max;
  }

  function walk(dir) {
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch (_error) {
      return;
    }

    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(fullPath);
        continue;
      }
      if (!entry.isFile()) continue;
      if (!isCandidate(fullPath, entry.name)) continue;

      const stat = safeStat(fullPath);
      if (!stat) continue;
      push({ path: fullPath, mtimeMs: stat.mtimeMs, size: stat.size });
    }
  }

  walk(rootDir);
  return latest;
}

class JsonlFollower {
  constructor({ seedBytes }) {
    this.seedBytes = Number.isFinite(seedBytes) ? seedBytes : 256 * 1024;
    this.filePath = null;
    this.position = 0;
    this.partial = '';
  }

  attach(filePath, onObject, options = {}) {
    const stat = safeStat(filePath);
    if (!stat) return;

    this.filePath = filePath;
    this.position = stat.size;
    this.partial = '';

    const emitSeed =
      options && Object.prototype.hasOwnProperty.call(options, 'emitSeed')
        ? Boolean(options.emitSeed)
        : true;

    const start = Math.max(0, stat.size - this.seedBytes);
    const seedText = readFileSliceUtf8(filePath, start, stat.size - start);
    let lines = seedText.split(/\r?\n/);
    if (start > 0) lines = lines.slice(1);
    if (emitSeed) {
      for (const line of lines) {
        if (!line) continue;
        const obj = safeJsonParse(line);
        if (obj) onObject(obj, { seed: true });
      }
    }
  }

  poll(onObject) {
    if (!this.filePath) return;

    const stat = safeStat(this.filePath);
    if (!stat) return;

    if (stat.size < this.position) {
      this.position = 0;
      this.partial = '';
    }

    if (stat.size === this.position) return;

    const chunk = readFileSliceUtf8(this.filePath, this.position, stat.size - this.position);
    this.position = stat.size;

    const text = this.partial + chunk;
    const parts = text.split(/\r?\n/);
    this.partial = parts.pop() || '';
    for (const line of parts) {
      if (!line) continue;
      const obj = safeJsonParse(line);
      if (obj) onObject(obj, { seed: false });
    }
  }
}

function makeLogger(log) {
  if (typeof log === 'function') return log;
  return () => {};
}

function summarizeResult(result) {
  if (!result || typeof result !== 'object') return 'unknown';
  if (result.skipped) return `skipped: ${result.reason || ''}`.trim();
  const ok = Array.isArray(result.results) ? result.results.filter((r) => r && r.ok).length : 0;
  const total = Array.isArray(result.results) ? result.results.length : 0;
  return `sent: ${ok}/${total}`;
}

async function maybeNotifyConfirm({
  source,
  text,
  cwd,
  logger,
  state,
  confirmDetector,
  tag,
  force,
  allowMultiplePerTurn = false,
  markTurnConfirmed = true,
  dedupeKey = ''
}) {
  if (typeof confirmDetector !== 'function') return false;
  const enabled = typeof confirmDetector.isEnabled === 'function' ? confirmDetector.isEnabled() : true;
  if (!enabled) return false;
  if (state && state.confirmNotifiedForTurn && !allowMultiplePerTurn) return false;

  const prompt = force ? String(text || '') : confirmDetector(text);
  if (!prompt) return false;
  const normalized = normalizeConfirmText(prompt);
  if (!normalized) return false;

  const defaultSeed = tag ? `${tag}:${normalized}` : normalized;
  const keySeed = normalizeConfirmText(dedupeKey || defaultSeed);
  const keyBase = (keySeed || normalized).slice(0, 200);
  const turnId = state && typeof state.currentTurnId === 'string' ? state.currentTurnId : '';
  const key = turnId ? `${turnId}::${keyBase}` : keyBase;
  const now = Date.now();
  if (state && state.lastConfirmKey === key && now - (state.lastConfirmAt || 0) < CONFIRM_DEDUPE_MS) return false;
  if (state) state.lastConfirmKey = key;
  if (state) state.lastConfirmAt = now;
  if (state && markTurnConfirmed) state.confirmNotifiedForTurn = true;

  const taskInfo = buildConfirmTaskInfo(normalized);
  const result = await sendNotifications({
    source,
    taskInfo,
    durationMs: null,
    cwd,
    force: true,
    notifyKind: 'confirm',
    skipSummary: true,
    summaryContext: { assistantMessage: normalized }
  });
  const suffix = tag ? ` (${tag})` : '';
  logger(`[watch][confirm:${source}] ${summarizeResult(result)}${suffix}`);
  return true;
}

function startClaudeWatch({ intervalMs, quietPeriodMs, log, claudeQuietMs, confirmDetector }) {
  const logger = makeLogger(log);
  const root = path.join(os.homedir(), '.claude', 'projects');
  const follower = new JsonlFollower({ seedBytes: 256 * 1024 });

  const state = {
    currentFile: null,
    tickRunning: false,
    lastUserTextAt: null,
    lastAssistantAt: null,
    lastNotifiedAt: null,
    notifiedForTurn: false,
    confirmNotifiedForTurn: false,
    lastCwd: null,
    pendingTimer: null,
    lastAssistantContent: null,
    lastAssistantHadToolUse: false,
    lastUserText: '',
    lastAssistantText: '',
    lastConfirmKey: '',
    lastConfirmAt: 0
  };

  const quietMs = Math.max(500, claudeQuietMs || quietPeriodMs || 60000);

  async function maybeNotify(ts) {
    if (state.lastAssistantAt == null || state.lastUserTextAt == null) return;
    if (state.lastNotifiedAt === state.lastAssistantAt) return;
    if (state.notifiedForTurn) return;
    if (state.confirmNotifiedForTurn) return;
    if (ts != null && ts !== state.lastAssistantAt) return;

    state.lastNotifiedAt = state.lastAssistantAt;
    state.notifiedForTurn = true;
    const durationMs =
      state.lastAssistantAt >= state.lastUserTextAt ? state.lastAssistantAt - state.lastUserTextAt : null;
    const cwd = state.lastCwd || process.cwd();
    const result = await sendNotifications({
      source: 'claude',
      taskInfo: 'Claude 完成',
      durationMs,
      cwd,
      outputContent: state.lastAssistantContent || state.lastAssistantText,
      summaryContext: {
        userMessage: state.lastUserText,
        assistantMessage: state.lastAssistantText
      }
    });
    state.confirmNotifiedForTurn = true;
    logger(`[watch][claude] ${summarizeResult(result)}`);
  }

  function scheduleSeedNotifyIfNeeded() {
    if (state.lastAssistantAt == null || state.lastUserTextAt == null) return;
    if (state.lastAssistantAt < state.lastUserTextAt) return;
    if (state.notifiedForTurn || state.confirmNotifiedForTurn) return;

    const now = Date.now();
    const windowMs = Math.max(quietMs * 2, 15000);
    if (now - state.lastAssistantAt > windowMs) return;

    const adaptiveQuietMs = state.lastAssistantHadToolUse ? quietMs : Math.min(15000, quietMs);
    if (state.pendingTimer) clearTimeout(state.pendingTimer);
    state.pendingTimer = setTimeout(() => {
      void maybeNotify(state.lastAssistantAt);
    }, adaptiveQuietMs);
  }

  async function processObject(obj, { seed }) {
    if (!obj || typeof obj !== 'object') return;
    if (obj.isSidechain === true) return;

    const ts = parseTimestamp(obj.timestamp);
    if (typeof obj.cwd === 'string') state.lastCwd = obj.cwd;

    if (obj.type === 'user') {
      const userText = extractMessageText(obj.message);
      state.lastUserText = userText;
      state.lastAssistantText = '';
      state.lastAssistantContent = null;
      state.lastAssistantHadToolUse = false;
      state.lastConfirmKey = '';
      state.confirmNotifiedForTurn = false;
      if (typeof obj.cwd === 'string') state.lastCwd = obj.cwd;
      if (seed) {
        if (ts != null) state.lastUserTextAt = ts;
        state.notifiedForTurn = false;
        return;
      }
      state.lastUserTextAt = ts;
      state.notifiedForTurn = false;
      return;
    }

    if (obj.type === 'assistant') {
      const assistantText = extractMessageText(obj.message);
      if (assistantText) state.lastAssistantText = assistantText;
      const hasToolUse = hasContentType(obj.message, 'tool_use');
      state.lastAssistantHadToolUse = hasToolUse;

      let content = '';

      if (obj.message && Array.isArray(obj.message.content)) {
        const textParts = obj.message.content
          .filter(item => item && item.type === 'text')
          .map(item => item.text || '')
          .filter(Boolean);
        content = textParts.join('\n\n');
      } else if (obj.message && typeof obj.message.content === 'string') {
        content = obj.message.content;
      } else if (obj.message && obj.message.text && typeof obj.message.text === 'string') {
        content = obj.message.text;
      }

      if (content && content.trim()) {
        state.lastAssistantContent = content;
      }

      if (ts != null || !seed) {
        state.lastAssistantAt = ts || Date.now();
      }

      if (seed) return;

      await maybeNotifyConfirm({
        source: 'claude',
        text: assistantText || content,
        cwd: state.lastCwd || process.cwd(),
        logger,
        state,
        confirmDetector
      });

      if (state.lastUserTextAt == null) {
        state.lastUserTextAt = ts || Date.now();
        state.notifiedForTurn = false;
      }

      if (state.confirmNotifiedForTurn) {
        if (state.pendingTimer) clearTimeout(state.pendingTimer);
        state.pendingTimer = null;
      } else {
        if (state.pendingTimer) clearTimeout(state.pendingTimer);

        const adaptiveQuietMs = hasToolUse ? quietMs : Math.min(15000, quietMs);

        state.pendingTimer = setTimeout(() => {
          void maybeNotify(state.lastAssistantAt);
        }, adaptiveQuietMs);
      }
    }
  }

  async function tick() {
    if (state.tickRunning) return;
    state.tickRunning = true;
    try {
      if (!fs.existsSync(root)) return;
      const latest = findLatestFile(root, (_full, name) => name.toLowerCase().endsWith('.jsonl'));
      if (!latest) return;
      if (latest.path !== state.currentFile) {
        state.currentFile = latest.path;
        state.lastUserTextAt = null;
        state.lastAssistantAt = null;
        state.lastNotifiedAt = null;
        state.notifiedForTurn = false;
        state.lastConfirmKey = '';
        state.confirmNotifiedForTurn = false;
        state.lastUserText = '';
        state.lastAssistantText = '';
        state.lastAssistantContent = null;
        state.lastAssistantHadToolUse = false;
        if (state.pendingTimer) clearTimeout(state.pendingTimer);
        state.pendingTimer = null;
        follower.attach(latest.path, (obj, meta) => {
          void processObject(obj, meta);
        });
        logger(`[watch][claude] following ${latest.path}`);
        scheduleSeedNotifyIfNeeded();
      }
      follower.poll((obj, meta) => {
        void processObject(obj, meta);
      });
    } finally {
      state.tickRunning = false;
    }
  }

  tick();
  const timer = setInterval(tick, Math.max(500, intervalMs || 1000));
  return () => clearInterval(timer);
}

function startCodexWatch({ intervalMs, log, confirmDetector }) {
  const logger = makeLogger(log);
  const root = path.join(os.homedir(), '.codex', 'sessions');
  const completionGraceMs = Math.max(200, Number(process.env.CODEX_TOKEN_GRACE_MS || 1500));
  const strictFinalAnswerOnly = String(process.env.CODEX_STRICT_FINAL_ANSWER || '1').trim() !== '0';
  // NOTE: Codex sessions commonly emit `event_msg:task_complete` without a `phase=final_answer` marker.
  // Prefer the explicit task_complete signal for immediate + accurate completion notifications.
  const finalAnswerQuietMs = Math.max(300, Number(process.env.CODEX_FINAL_ANSWER_QUIET_MS || 800));
  // Fallback when `task_complete` is missing (rare) or delayed: treat "assistant message + quiet window"
  // as completion. Keep this window short to reduce perceived latency, but long enough to avoid false positives.
  const emptyPhaseQuietMs = Math.max(3000, Number(process.env.CODEX_EMPTY_PHASE_QUIET_MS || 15000));
  const followTopN = Math.max(1, Number(process.env.CODEX_FOLLOW_TOP_N || 5));
  // When we attach to a just-created session, its first turn may already be fully written.
  // Re-scan the seed window and treat recent lines as "live" so confirm/complete notifications are not missed.
  const seedCatchupMs = Math.max(0, Number(process.env.CODEX_SEED_CATCHUP_MS || 30000));

  function isCodexWorkResponseType(type) {
    return [
      'reasoning',
      'function_call',
      'function_call_output',
      'custom_tool_call',
      'custom_tool_call_output',
      'web_search_call',
      'tool_use'
    ].includes(type);
  }

  function extractRequestUserInputText(payload) {
    if (!payload || typeof payload !== 'object') return '需要你确认下一步？';

    const rawArgs = payload.arguments != null ? payload.arguments : (payload.function && payload.function.arguments);
    let args = null;
    if (rawArgs && typeof rawArgs === 'object') {
      args = rawArgs;
    } else if (typeof rawArgs === 'string') {
      args = safeJsonParse(rawArgs);
    }

    const lines = [];
    const questions = args && Array.isArray(args.questions) ? args.questions : [];

    const ensureQuestion = (value) => {
      const trimmed = String(value || '').trim();
      if (!trimmed) return '';
      return /[?？]$/.test(trimmed) ? trimmed : `${trimmed}？`;
    };

    const pushOptions = (options) => {
      const labels = Array.isArray(options)
        ? options
          .map((o) => (o && typeof o.label === 'string' ? o.label.trim() : ''))
          .filter(Boolean)
        : [];
      if (labels.length) lines.push(`选项: ${labels.join(' / ')}`);
    };

    if (questions.length) {
      for (const q of questions) {
        const header = q && typeof q.header === 'string' ? q.header.trim() : '';
        const question = q && typeof q.question === 'string' ? q.question.trim() : '';
        if (header) lines.push(header);
        if (question) lines.push(ensureQuestion(question));
        pushOptions(q && q.options);
      }
    } else {
      const question = args && typeof args.question === 'string' ? args.question.trim() : '';
      if (question) lines.push(ensureQuestion(question));
      pushOptions(args && args.options);
    }

    if (!lines.length) lines.push('需要你确认下一步？');
    // Ensure detector can trigger even if upstream forgot punctuation.
    if (!lines.some((line) => /[?？]\s*$/.test(String(line).trim()))) {
      lines[0] = ensureQuestion(lines[0]);
    }

    return lines.join('\n').trim();
  }

  function hasOptionsInRequestPrompt(text) {
    const raw = String(text || '').trim();
    if (!raw) return false;
    const lines = raw.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
    return lines.some((line) => /^(选项|options?)\s*[:：]/i.test(line));
  }

  function buildRequestUserInputDedupeKey(payload, ts) {
    if (!payload || typeof payload !== 'object') {
      return `request_user_input:${ts != null ? ts : Date.now()}`;
    }
    const ids = [];
    if (typeof payload.call_id === 'string') ids.push(payload.call_id);
    if (typeof payload.id === 'string') ids.push(payload.id);
    if (typeof payload.tool_call_id === 'string') ids.push(payload.tool_call_id);
    if (typeof payload.function_call_id === 'string') ids.push(payload.function_call_id);
    if (payload.function && typeof payload.function.call_id === 'string') ids.push(payload.function.call_id);
    if (ids.length) return `request_user_input:${ids.join('|')}`;

    const turnPart = typeof payload.turn_id === 'string' ? payload.turn_id : '';
    const timePart = ts != null ? ts : Date.now();
    return `request_user_input:${turnPart}:${timePart}`;
  }

  // Turn-end prompt detection (text-only, no interactive `request_user_input`).
  // We run this ONLY at `task_complete` to avoid premature alerts during streaming output.
  const CODEX_TURN_END_CONFIRM_CUES = [
    // CN
    '请确认',
    '是否继续',
    '是否开始',
    '是否开始执行',
    '是否执行',
    '是否同意',
    '是否允许',
    '是否授权',
    '请选择',
    '请选',
    '你希望',
    '你想',
    '你要',
    '要不要',
    '可以吗',
    '可以么',
    '能否',
    '可否',
    // EN
    'please confirm',
    'confirm',
    'approve',
    'approval',
    'proceed',
    'continue',
    'should i',
    'shall i',
    'do you want me',
    'would you like',
    'may i'
  ];
  const CODEX_TURN_END_CONFIRM_CUES_LOWER = CODEX_TURN_END_CONFIRM_CUES.map((x) => String(x).toLowerCase());
  const CODEX_TURN_END_ACTION_WORDS = [
    // CN (verbs that usually imply "waiting for your input")
    '开始',
    '继续',
    '执行',
    '确认',
    '选择',
    '提交',
    '授权',
    '允许',
    '同意',
    // EN
    'proceed',
    'continue',
    'execute',
    'run',
    'confirm',
    'choose',
    'select',
    'approve',
    'authorize'
  ];
  const CODEX_TURN_END_ACTION_WORDS_LOWER = CODEX_TURN_END_ACTION_WORDS.map((x) => String(x).toLowerCase());

  function extractTailLines(text, maxLines, maxChars) {
    const raw = String(text || '').replace(/\r\n/g, '\n').trim();
    if (!raw) return '';

    const limited = raw.length > maxChars ? raw.slice(raw.length - maxChars) : raw;
    const lines = limited
      .split('\n')
      .map((l) => l.trim())
      .filter(Boolean);

    const tail = lines.slice(Math.max(0, lines.length - Math.max(1, maxLines)));
    return tail.join('\n').trim();
  }

  function detectTurnEndConfirmPrompt(text, { planMode }) {
    const snippet = extractTailLines(text, 6, 1200);
    if (!snippet) return '';

    const lower = snippet.toLowerCase();
    const tailWindow = lower.slice(Math.max(0, lower.length - 500));
    const lastLine = snippet.split('\n').map((l) => l.trim()).filter(Boolean).slice(-1)[0] || '';
    const endsWithQuestion = /[?？]\s*$/.test(lastLine);

    const cueNearEnd = CODEX_TURN_END_CONFIRM_CUES_LOWER.some((k) => k && tailWindow.includes(k));
    if (cueNearEnd) return snippet;

    const actionNearEnd = CODEX_TURN_END_ACTION_WORDS_LOWER.some((k) => k && tailWindow.includes(k));
    if (endsWithQuestion && actionNearEnd) {
      // A short, action-oriented question at the end of a turn usually means "waiting for user input".
      return snippet.length <= 600 ? snippet : extractTailLines(snippet, 3, 600);
    }

    return '';
  }

  function createSession(filePath) {
    const follower = new JsonlFollower({ seedBytes: 256 * 1024 });
    const state = {
      filePath,
      lastEventAt: 0,
      attachedAt: Date.now(),
      lastUserAt: null,
      lastAssistantAt: null,
      lastNotifiedAssistantAt: null,
      lastTaskStartedAt: null,
      currentTurnId: null,
      collaborationModeKind: '',
      lastNotifiedTurnId: null,
      lastCwd: null,
      lastAgentContent: null,
      lastUserText: '',
      lastAssistantText: '',
      lastRequestUserInputPrompt: '',
      lastConfirmKey: '',
      lastConfirmAt: 0,
      confirmNotifiedForTurn: false,
      // Plan mode interactive prompts (`request_user_input`) mean the turn ends waiting for user input.
      // We should never treat that as "task completed".
      interactionRequiredForTurn: false,
      pendingRequestUserInputCallIds: new Set(),
      pendingRequestUserInputWithoutId: 0,
      lastInteractionResolvedAt: null,
      interactionLoggedForTurn: false,
      interactionNotifiedForTurn: false,
      pendingCompletion: null
    };

    function clearPendingCompletion() {
      const pending = state.pendingCompletion;
      if (!pending) return;
      if (pending.timer) {
        clearTimeout(pending.timer);
      }
      state.pendingCompletion = null;
    }

    function stagePendingCompletion(ts, options = {}) {
      clearPendingCompletion();
      const assistantAt = ts != null ? ts : Date.now();
      state.lastAssistantAt = assistantAt;
      const tokenRequired = options && Object.prototype.hasOwnProperty.call(options, 'tokenRequired')
        ? Boolean(options.tokenRequired)
        : true;
      const quietMs = options && Number.isFinite(Number(options.quietMs)) ? Number(options.quietMs) : 0;
      state.pendingCompletion = {
        assistantAt,
        tokenRequired,
        tokenSeen: false,
        tokenAt: null,
        timer: null
      };

      if (quietMs > 0) {
        state.pendingCompletion.timer = setTimeout(() => {
          void flushPendingCompletion('quiet_fallback', { allowWithoutToken: true });
        }, quietMs);
      }
    }

    function markPendingTokenSeen(ts) {
      const pending = state.pendingCompletion;
      if (!pending) return;
      if (!pending.tokenRequired) return;
      pending.tokenSeen = true;
      pending.tokenAt = ts != null ? ts : Date.now();
      if (pending.timer) clearTimeout(pending.timer);
      pending.timer = setTimeout(() => {
        void flushPendingCompletion('token_grace');
      }, completionGraceMs);
    }

    async function flushPendingCompletion(reason, options = {}) {
      const pending = state.pendingCompletion;
      const allowWithoutToken = Boolean(options && options.allowWithoutToken);
      if (!pending) return false;
      if (pending.tokenRequired && !pending.tokenSeen && !allowWithoutToken) return false;

      clearPendingCompletion();

      if (state.confirmNotifiedForTurn) {
        logger(`[watch][codex] skipped completion (${reason}: confirm alert sent)`);
        return false;
      }

      const assistantAt = pending.assistantAt;
      if (assistantAt != null && state.lastNotifiedAssistantAt === assistantAt) {
        return false;
      }

      const startAt = state.lastUserAt != null ? state.lastUserAt : state.lastTaskStartedAt;
      const durationMs =
        assistantAt != null && startAt != null && assistantAt >= startAt
          ? assistantAt - startAt
          : null;
      const cwd = state.lastCwd || process.cwd();
      const result = await sendNotifications({
        source: 'codex',
        taskInfo: 'Codex 完成',
        durationMs,
        cwd,
        outputContent: state.lastAgentContent || state.lastAssistantText,
        summaryContext: {
          userMessage: state.lastUserText,
          assistantMessage: state.lastAssistantText
        }
      });

      state.lastNotifiedAssistantAt = assistantAt;
      state.confirmNotifiedForTurn = true;
      logger(`[watch][codex] ${summarizeResult(result)} (${reason})`);
      return true;
    }

    async function processObject(obj, { seed }) {
      if (!obj || typeof obj !== 'object') return;
      const ts = parseTimestamp(obj.timestamp);

      if (!seed) {
        state.lastEventAt = Date.now();
      }

      if (obj.type === 'turn_context') {
        if (obj.payload && typeof obj.payload.cwd === 'string') {
          state.lastCwd = obj.payload.cwd;
        }
        if (obj.payload && typeof obj.payload.turn_id === 'string') {
          const nextTurnId = obj.payload.turn_id;
          if (state.currentTurnId && state.currentTurnId !== nextTurnId) {
            clearPendingCompletion();
            state.lastRequestUserInputPrompt = '';
            state.lastConfirmKey = '';
            state.confirmNotifiedForTurn = false;
            state.interactionRequiredForTurn = false;
            state.pendingRequestUserInputCallIds.clear();
            state.pendingRequestUserInputWithoutId = 0;
            state.lastInteractionResolvedAt = null;
            state.interactionLoggedForTurn = false;
            state.interactionNotifiedForTurn = false;
          }
          state.currentTurnId = obj.payload.turn_id;
        }
        if (obj.payload && obj.payload.collaboration_mode && typeof obj.payload.collaboration_mode.mode === 'string') {
          state.collaborationModeKind = obj.payload.collaboration_mode.mode;
        }
        return;
      }

      if (obj.type === 'response_item' && obj.payload && obj.payload.type === 'message' && obj.payload.role === 'user') {
        if (!seed) {
          await flushPendingCompletion('before_user_message', { allowWithoutToken: true });
        }
        clearPendingCompletion();
        state.lastTaskStartedAt = null;
        state.lastUserAt = ts;
        state.lastUserText = extractTextFromAny(obj.payload);
        state.lastRequestUserInputPrompt = '';
        state.lastConfirmKey = '';
        state.confirmNotifiedForTurn = false;
        state.interactionRequiredForTurn = false;
        state.pendingRequestUserInputCallIds.clear();
        state.pendingRequestUserInputWithoutId = 0;
        state.lastInteractionResolvedAt = null;
        state.interactionLoggedForTurn = false;
        state.interactionNotifiedForTurn = false;
        return;
      }

      // Codex Plan mode interactive prompts: request_user_input (single-choice / submit UI).
      // These are emitted as a tool call item (e.g. `function_call` / `custom_tool_call`) with name `request_user_input`.
      if (
        obj.type === 'response_item'
        && obj.payload
        && ['function_call', 'custom_tool_call', 'tool_use'].includes(obj.payload.type)
        && (
          obj.payload.name === 'request_user_input'
          || obj.payload.function_name === 'request_user_input'
          || (obj.payload.function && obj.payload.function.name === 'request_user_input')
        )
      ) {
        clearPendingCompletion();
        state.interactionRequiredForTurn = true;
        state.lastInteractionResolvedAt = null;
        {
          const requestCallId =
            (typeof obj.payload.call_id === 'string' && obj.payload.call_id.trim())
            || (typeof obj.payload.id === 'string' && obj.payload.id.trim())
            || (typeof obj.payload.tool_call_id === 'string' && obj.payload.tool_call_id.trim())
            || (typeof obj.payload.function_call_id === 'string' && obj.payload.function_call_id.trim())
            || '';
          if (requestCallId) state.pendingRequestUserInputCallIds.add(requestCallId);
          else state.pendingRequestUserInputWithoutId += 1;
        }
        state.lastRequestUserInputPrompt = extractRequestUserInputText(obj.payload);
        if (seed) return;

        const confirmEnabled = typeof confirmDetector?.isEnabled === 'function' ? confirmDetector.isEnabled() : true;
        if (!confirmEnabled) {
          if (!state.interactionLoggedForTurn) {
            state.interactionLoggedForTurn = true;
            logger('[watch][codex] detected request_user_input (confirm disabled)');
          }
          return;
        }

        const sent = await maybeNotifyConfirm({
          source: 'codex',
          text: state.lastRequestUserInputPrompt,
          cwd: state.lastCwd || process.cwd(),
          logger,
          state,
          confirmDetector,
          tag: 'plan_request_user_input',
          force: true,
          allowMultiplePerTurn: true,
          markTurnConfirmed: false,
          dedupeKey: buildRequestUserInputDedupeKey(obj.payload, ts)
        });
        if (sent) state.interactionNotifiedForTurn = true;

        return;
      }

      if (
        obj.type === 'response_item'
        && obj.payload
        && ['function_call_output', 'custom_tool_call_output'].includes(obj.payload.type)
      ) {
        const outputCallId =
          (typeof obj.payload.call_id === 'string' && obj.payload.call_id.trim())
          || (typeof obj.payload.id === 'string' && obj.payload.id.trim())
          || (typeof obj.payload.tool_call_id === 'string' && obj.payload.tool_call_id.trim())
          || (typeof obj.payload.function_call_id === 'string' && obj.payload.function_call_id.trim())
          || '';

        if (outputCallId && state.pendingRequestUserInputCallIds.has(outputCallId)) {
          state.pendingRequestUserInputCallIds.delete(outputCallId);
        } else if (!outputCallId && state.pendingRequestUserInputWithoutId > 0) {
          state.pendingRequestUserInputWithoutId = Math.max(0, state.pendingRequestUserInputWithoutId - 1);
        }

        if (
          state.pendingRequestUserInputCallIds.size === 0
          && state.pendingRequestUserInputWithoutId === 0
        ) {
          state.interactionRequiredForTurn = false;
          state.lastInteractionResolvedAt = ts != null ? ts : Date.now();
          state.lastRequestUserInputPrompt = '';
        }
      }

      if (obj.type === 'response_item' && obj.payload && isCodexWorkResponseType(obj.payload.type)) {
        clearPendingCompletion();
        return;
      }

      if (obj.type === 'response_item' && obj.payload && obj.payload.type === 'message' && obj.payload.role === 'assistant') {
        if (seed) return;

        clearPendingCompletion();

        const assistantText = extractTextFromAny(obj.payload);
        if (assistantText) {
          state.lastAssistantText = assistantText;
          state.lastAgentContent = assistantText;
        }
        state.lastAssistantAt = ts != null ? ts : Date.now();

        // Completion for Codex is driven by `event_msg:task_complete` only.
        // This avoids premature reminders during streaming output and prevents suppressing the real completion.
        return;
      }

      if (obj.type === 'event_msg' && obj.payload && typeof obj.payload.type === 'string') {
        const kind = obj.payload.type;

        if (kind === 'task_started') {
          if (!seed) {
            await flushPendingCompletion('before_task_started', { allowWithoutToken: true });
          }
          if (obj.payload && typeof obj.payload.turn_id === 'string') {
            state.currentTurnId = obj.payload.turn_id;
          }
          if (obj.payload && typeof obj.payload.collaboration_mode_kind === 'string') {
            state.collaborationModeKind = obj.payload.collaboration_mode_kind;
          }
          state.lastTaskStartedAt = ts;
          // New turn begins; reset per-turn dedupe/flags.
          state.lastRequestUserInputPrompt = '';
          state.lastConfirmKey = '';
          state.confirmNotifiedForTurn = false;
          state.interactionRequiredForTurn = false;
          state.pendingRequestUserInputCallIds.clear();
          state.pendingRequestUserInputWithoutId = 0;
          state.lastInteractionResolvedAt = null;
          state.interactionLoggedForTurn = false;
          state.interactionNotifiedForTurn = false;
          clearPendingCompletion();
          return;
        }

        if (kind === 'task_complete') {
          if (seed) return;

          const turnId = obj.payload && typeof obj.payload.turn_id === 'string' ? obj.payload.turn_id : null;
          if (turnId && state.lastNotifiedTurnId === turnId) return;

          clearPendingCompletion();

          const completionAt = ts != null ? ts : Date.now();
          const lastAgentMessage = obj.payload && typeof obj.payload.last_agent_message === 'string'
            ? obj.payload.last_agent_message
            : '';
          if (lastAgentMessage) {
            state.lastAssistantText = lastAgentMessage;
            state.lastAgentContent = lastAgentMessage;
            state.lastAssistantAt = completionAt;
          }
          const assistantStaleByInteraction =
            !lastAgentMessage
            && state.lastInteractionResolvedAt != null
            && state.lastAssistantAt != null
            && state.lastAssistantAt <= state.lastInteractionResolvedAt;

          const confirmEnabled = typeof confirmDetector?.isEnabled === 'function' ? confirmDetector.isEnabled() : true;

          if (state.interactionRequiredForTurn) {
            let sentInThisTaskComplete = false;
            const requestPrompt = String(state.lastRequestUserInputPrompt || '').trim();
            const requestHasOptions = hasOptionsInRequestPrompt(requestPrompt);

            // Prefer explicit option prompts when available (more actionable than tail text).
            if (confirmEnabled && requestHasOptions) {
              const sent = await maybeNotifyConfirm({
                source: 'codex',
                text: requestPrompt,
                cwd: state.lastCwd || process.cwd(),
                logger,
                state,
                confirmDetector,
                tag: 'plan_request_user_input_fallback',
                force: true,
                allowMultiplePerTurn: true,
                markTurnConfirmed: false,
                dedupeKey: `request_user_input_options:${turnId || ''}:${completionAt}`
              });
              if (sent) {
                sentInThisTaskComplete = true;
                state.interactionNotifiedForTurn = true;
                if (turnId) state.lastNotifiedTurnId = turnId;
              }
            }

            if (confirmEnabled) {
              const prompt = detectTurnEndConfirmPrompt(state.lastAgentContent || state.lastAssistantText, {
                planMode: String(state.collaborationModeKind || '').toLowerCase() === 'plan'
              });
              if (prompt && !sentInThisTaskComplete) {
                const sent = await maybeNotifyConfirm({
                  source: 'codex',
                  text: prompt,
                  cwd: state.lastCwd || process.cwd(),
                  logger,
                  state,
                  confirmDetector,
                  tag: 'turn_end_question',
                  force: true,
                  allowMultiplePerTurn: true,
                  markTurnConfirmed: false,
                  dedupeKey: `turn_end_question:${turnId || ''}:${completionAt}`
                });
                if (sent) {
                  sentInThisTaskComplete = true;
                  state.interactionNotifiedForTurn = true;
                  if (turnId) state.lastNotifiedTurnId = turnId;
                }
              }
            }

            if (confirmEnabled && !sentInThisTaskComplete && !state.interactionNotifiedForTurn) {
              const fallbackText =
                requestPrompt
                || String(state.lastAgentContent || state.lastAssistantText || '').trim()
                || '需要你确认下一步？';
              const sent = await maybeNotifyConfirm({
                source: 'codex',
                text: fallbackText,
                cwd: state.lastCwd || process.cwd(),
                logger,
                state,
                confirmDetector,
                tag: 'plan_request_user_input_fallback',
                force: true,
                allowMultiplePerTurn: true,
                markTurnConfirmed: false,
                dedupeKey: `request_user_input_fallback:${turnId || ''}:${completionAt}`
              });
              if (sent) {
                state.interactionNotifiedForTurn = true;
                if (turnId) state.lastNotifiedTurnId = turnId;
              }
            }
            logger('[watch][codex] skipped completion (task_complete: interaction required)');
            return;
          }

          if (state.confirmNotifiedForTurn) {
            logger('[watch][codex] skipped completion (task_complete: already notified)');
            return;
          }

          if (confirmEnabled && !assistantStaleByInteraction) {
            const prompt = detectTurnEndConfirmPrompt(state.lastAgentContent || state.lastAssistantText, {
              planMode: String(state.collaborationModeKind || '').toLowerCase() === 'plan'
            });
            if (prompt) {
              const sent = await maybeNotifyConfirm({
                source: 'codex',
                text: prompt,
                cwd: state.lastCwd || process.cwd(),
                logger,
                state,
                confirmDetector,
                tag: 'turn_end_question',
                force: true,
                dedupeKey: `turn_end_question:${turnId || ''}:${completionAt}`
              });
              if (sent) state.interactionNotifiedForTurn = true;
              if (turnId) state.lastNotifiedTurnId = turnId;
              logger('[watch][codex] skipped completion (task_complete: question)');
              return;
            }
          }

          const startAt = state.lastUserAt != null ? state.lastUserAt : state.lastTaskStartedAt;
          const durationMs =
            startAt != null && completionAt >= startAt
              ? completionAt - startAt
              : null;
          const completionOutput =
            lastAgentMessage
            || (assistantStaleByInteraction ? '' : String(state.lastAgentContent || state.lastAssistantText || '').trim());

          const cwd = state.lastCwd || process.cwd();
          const result = await sendNotifications({
            source: 'codex',
            taskInfo: 'Codex 完成',
            durationMs,
            cwd,
            outputContent: completionOutput,
            summaryContext: {
              userMessage: state.lastUserText,
              assistantMessage: completionOutput || state.lastAssistantText
            }
          });

          state.lastNotifiedAssistantAt = completionAt;
          if (turnId) state.lastNotifiedTurnId = turnId;
          state.confirmNotifiedForTurn = true;
          logger(`[watch][codex] ${summarizeResult(result)} (task_complete)`);
          return;
        }

        if (kind === 'user_message') {
          if (!seed) {
            await flushPendingCompletion('before_user_event', { allowWithoutToken: true });
          }
          clearPendingCompletion();
          state.lastTaskStartedAt = null;
          state.lastUserAt = ts;
          state.lastUserText = extractTextFromAny(obj.payload);
          state.lastRequestUserInputPrompt = '';
          state.lastConfirmKey = '';
          state.confirmNotifiedForTurn = false;
          state.interactionRequiredForTurn = false;
          state.pendingRequestUserInputCallIds.clear();
          state.pendingRequestUserInputWithoutId = 0;
          state.lastInteractionResolvedAt = null;
          state.interactionLoggedForTurn = false;
          state.interactionNotifiedForTurn = false;
          return;
        }

        if (kind === 'token_count') {
          // Kept for backward-compat (older Codex session formats).
          if (!seed) markPendingTokenSeen(ts);
          return;
        }

        if (kind === 'agent_reasoning') {
          clearPendingCompletion();
          return;
        }

        if (kind === 'agent_message') {
          if (seed) return;
          const assistantText = extractTextFromAny(obj.payload);
          if (assistantText) state.lastAssistantText = assistantText;
          state.lastAssistantAt = ts != null ? ts : Date.now();

          let content = null;
          if (obj.payload && typeof obj.payload.content === 'string') {
            content = obj.payload.content;
          } else if (obj.payload && obj.payload.message && typeof obj.payload.message === 'string') {
            content = obj.payload.message;
          } else if (obj.payload && obj.payload.text && typeof obj.payload.text === 'string') {
            content = obj.payload.text;
          } else if (obj.payload && obj.payload.data && typeof obj.payload.data === 'string') {
            content = obj.payload.data;
          } else if (obj.message && typeof obj.message === 'string') {
            content = obj.message;
          }

          if (content && content.trim()) {
            state.lastAgentContent = content;
          }

          // Confirm alerts for Codex are triggered only by explicit interactive prompts
          // (request_user_input), not by text output.
        }
      }
    }

    follower.attach(
      filePath,
      (obj, meta) => {
        void processObject(obj, meta);
      },
      { emitSeed: false }
    );
    logger(`[watch][codex] following ${filePath}`);

    let seedPrimed = false;
    let seedPriming = null;

    async function primeFromSeedWindow(windowMs) {
      if (!windowMs || windowMs <= 0) return;
      const stat = safeStat(filePath);
      if (!stat || stat.size <= 0) return;

      const start = Math.max(0, stat.size - follower.seedBytes);
      const seedText = readFileSliceUtf8(filePath, start, stat.size - start);
      let lines = seedText.split(/\r?\n/);
      if (start > 0) lines = lines.slice(1);

      const objects = [];
      for (const line of lines) {
        if (!line) continue;
        const obj = safeJsonParse(line);
        if (obj) objects.push(obj);
      }

      // Pass 1: update state without sending notifications.
      for (const obj of objects) {
        await processObject(obj, { seed: true });
      }

      // Pass 2: treat recent seed lines as "live" to avoid missing quick turns.
      const since = Date.now() - Math.max(0, Number(windowMs));
      for (const obj of objects) {
        const ts = parseTimestamp(obj && obj.timestamp);
        if (ts != null && ts < since) continue;
        await processObject(obj, { seed: false });
      }
    }

    async function ensureSeedPrimed(windowMs) {
      if (seedPrimed) return;
      if (!seedPriming) {
        seedPriming = (async () => {
          try {
            await primeFromSeedWindow(windowMs);
          } catch (_error) {
            // ignore
          } finally {
            seedPrimed = true;
          }
        })();
      }
      await seedPriming;
    }

    return {
      filePath,
      poll: async ({ seedCatchupMs } = {}) => {
        await ensureSeedPrimed(seedCatchupMs);
        follower.poll((obj, meta) => {
          void processObject(obj, meta);
        });
      },
      stop: () => {
        clearPendingCompletion();
      },
      state
    };
  }

  const sessions = new Map();
  const state = {
    tickRunning: false
  };

  async function tick() {
    if (state.tickRunning) return;
    state.tickRunning = true;
    try {
      if (!fs.existsSync(root)) return;

      const latest = findLatestFiles(root, (_full, name) => name.toLowerCase().endsWith('.jsonl'), followTopN);
      if (!latest || latest.length === 0) return;

      const wanted = new Set(latest.map((x) => x.path));
      for (const filePath of wanted) {
        if (!sessions.has(filePath)) {
          sessions.set(filePath, createSession(filePath));
        }
      }

      // Poll wanted sessions; drop others to keep memory bounded.
      for (const [filePath, session] of sessions.entries()) {
        if (!wanted.has(filePath)) {
          try {
            if (session && typeof session.stop === 'function') session.stop();
          } finally {
            sessions.delete(filePath);
          }
          continue;
        }
        try {
          await session.poll({ seedCatchupMs });
        } catch (_error) {
          // ignore
        }
      }
    } finally {
      state.tickRunning = false;
    }
  }

  tick();
  const timer = setInterval(tick, Math.max(500, intervalMs || 1000));
  return () => {
    clearInterval(timer);
    for (const session of sessions.values()) {
      try {
        if (session && typeof session.stop === 'function') session.stop();
      } catch (_error) {
        // ignore
      }
    }
  };
}

function startGeminiWatch({ intervalMs, quietPeriodMs, log, confirmDetector }) {
  const logger = makeLogger(log);
  const root = path.join(os.homedir(), '.gemini', 'tmp');

  const state = {
    currentFile: null,
    currentMtimeMs: 0,
    lastCount: 0,
    lastUserAt: null,
    lastGeminiAt: null,
    lastNotifiedGeminiAt: null,
    tickRunning: false,
    pendingTimer: null,
    lastGeminiContent: null, // 鎹曡幏gemini鐨勮緭鍑哄唴瀹?    lastUserText: '',
    lastGeminiText: '',
    lastConfirmKey: '',
    lastConfirmAt: 0,
    confirmNotifiedForTurn: false
  };

  async function notifyIfReady(reason) {
    const endAt = state.lastGeminiAt;
    const startAt = state.lastUserAt;
    if (endAt == null || startAt == null) return;
    if (state.lastNotifiedGeminiAt === endAt) return;
    if (state.confirmNotifiedForTurn) {
      state.lastNotifiedGeminiAt = endAt;
      logger('[watch][gemini] skipped completion (confirm alert sent)');
      return;
    }

    state.lastNotifiedGeminiAt = endAt;
    const durationMs = endAt >= startAt ? endAt - startAt : null;
    const result = await sendNotifications({
      source: 'gemini',
      taskInfo: 'Gemini 完成',
      durationMs,
      cwd: process.cwd(),
      projectNameOverride: 'Gemini',
      outputContent: state.lastGeminiContent || state.lastGeminiText,
      summaryContext: {
        userMessage: state.lastUserText,
        assistantMessage: state.lastGeminiText
      }
    });
    state.confirmNotifiedForTurn = true;
    logger(`[watch][gemini] ${reason} ${summarizeResult(result)}`.trim());
  }

  function scheduleDebouncedNotify() {
    if (state.pendingTimer) clearTimeout(state.pendingTimer);
    const targetGeminiAt = state.lastGeminiAt;
    state.pendingTimer = setTimeout(() => {
      state.pendingTimer = null;
      if (state.lastGeminiAt !== targetGeminiAt) return;
      void notifyIfReady('debounced');
    }, Math.max(500, quietPeriodMs || 3000));
  }

  function switchFile(filePath, mtimeMs, messages) {
    if (state.pendingTimer) clearTimeout(state.pendingTimer);
    state.pendingTimer = null;

    state.currentFile = filePath;
    state.currentMtimeMs = mtimeMs;
    state.lastCount = Array.isArray(messages) ? messages.length : 0;

    state.lastUserAt = null;
    state.lastGeminiAt = null;
    state.lastUserText = '';
    state.lastGeminiText = '';
    state.lastConfirmKey = '';
    state.confirmNotifiedForTurn = false;

    if (Array.isArray(messages)) {
      for (const m of messages) {
        if (!m || typeof m !== 'object') continue;
        const ts = parseTimestamp(m.timestamp);
        if (m.type === 'user') {
          state.lastUserAt = ts;
          state.lastUserText = extractTextFromAny(m);
        }
        if (m.type === 'gemini') {
          state.lastGeminiAt = ts;
          const geminiText = extractTextFromAny(m);
          if (geminiText) state.lastGeminiText = geminiText;
        }
      }
    }

    state.lastNotifiedGeminiAt = state.lastGeminiAt;
    logger(`[watch][gemini] following ${filePath}`);
  }

  async function tick() {
    if (state.tickRunning) return;
    state.tickRunning = true;
    try {
      if (!fs.existsSync(root)) return;
      const latest = findLatestFile(root, (fullPath, name) => {
        if (!name.toLowerCase().endsWith('.json')) return false;
        if (!name.toLowerCase().startsWith('session-')) return false;
        return fullPath.toLowerCase().includes(`${path.sep}chats${path.sep}`);
      });
      if (!latest) return;

      const stat = safeStat(latest.path);
      if (!stat) return;

      if (latest.path !== state.currentFile) {
        try {
          const parsed = JSON.parse(fs.readFileSync(latest.path, 'utf8').replace(/^\uFEFF/, ''));
          switchFile(latest.path, stat.mtimeMs, parsed && parsed.messages);
        } catch (_error) {
          return;
        }
        return;
      }

      if (stat.mtimeMs <= state.currentMtimeMs) return;

      let parsed;
      try {
        parsed = JSON.parse(fs.readFileSync(latest.path, 'utf8').replace(/^\uFEFF/, ''));
      } catch (_error) {
        return;
      }

      const messages = parsed && Array.isArray(parsed.messages) ? parsed.messages : [];
      if (messages.length <= state.lastCount) {
        state.currentMtimeMs = stat.mtimeMs;
        state.lastCount = messages.length;
        return;
      }

      const newMessages = messages.slice(state.lastCount);
      for (const m of newMessages) {
        if (!m || typeof m !== 'object') continue;
        const ts = parseTimestamp(m.timestamp);
        if (m.type === 'user') {
          if (state.pendingTimer) clearTimeout(state.pendingTimer);
          state.pendingTimer = null;
          state.lastUserAt = ts;
          state.lastUserText = extractTextFromAny(m);
          state.lastGeminiAt = null;
          state.lastNotifiedGeminiAt = null;
          state.lastGeminiText = '';
          state.lastConfirmKey = '';
          state.confirmNotifiedForTurn = false;
          continue;
        }

        if (m.type === 'gemini') {
          state.lastGeminiAt = ts;

          // 鎹曡幏gemini娑堟伅鐨勫唴瀹?- 澧炲己鐗?          let content = '';

          if (m.content && Array.isArray(m.content)) {
            // 鏍煎紡1锛歝ontent鏄瓧绗︿覆鏁扮粍
            const textParts = m.content
              .filter(item => item && typeof item === 'string')
              .filter(Boolean);
            content = textParts.join('\n\n');
          } else if (m.content && typeof m.content === 'string') {
            // 鏍煎紡2锛歝ontent鐩存帴鏄瓧绗︿覆
            content = m.content;
          } else if (m.parts && Array.isArray(m.parts)) {
            // 鏍煎紡3锛氫娇鐢╬arts瀛楁
            const textParts = m.parts
              .filter(part => part && part.text && typeof part.text === 'string')
              .map(part => part.text)
              .filter(Boolean);
            content = textParts.join('\n\n');
          } else if (m.text && typeof m.text === 'string') {
            // 鏍煎紡4锛氱洿鎺ヤ娇鐢╰ext瀛楁
            content = m.text;
          }

          if (content && content.trim()) {
            state.lastGeminiContent = content;
          }

          const geminiText = extractTextFromAny(m);
          if (geminiText) state.lastGeminiText = geminiText;

          await maybeNotifyConfirm({
            source: 'gemini',
            text: geminiText || content,
            cwd: process.cwd(),
            logger,
            state,
            confirmDetector
          });

          if (state.confirmNotifiedForTurn) {
            if (state.pendingTimer) clearTimeout(state.pendingTimer);
            state.pendingTimer = null;
            continue;
          }

          scheduleDebouncedNotify();
        }
      }

      state.currentMtimeMs = stat.mtimeMs;
      state.lastCount = messages.length;
    } finally {
      state.tickRunning = false;
    }
  }

  tick();
  const timer = setInterval(tick, Math.max(500, intervalMs || 1500));
  return () => {
    clearInterval(timer);
    if (state.pendingTimer) clearTimeout(state.pendingTimer);
  };
}

function normalizeSources(input) {
  if (!input) return ['claude', 'codex', 'gemini'];
  const raw = Array.isArray(input) ? input.join(',') : String(input);
  const parts = raw
    .split(',')
    .map((p) => p.trim().toLowerCase())
    .filter(Boolean);
  if (parts.includes('all')) return ['claude', 'codex', 'gemini'];
  return [...new Set(parts)];
}

function startWatch({ sources, intervalMs, geminiQuietMs, claudeQuietMs, log, confirmAlert }) {
  const normalizedSources = normalizeSources(sources);
  const stops = [];
  const confirmDetector = createConfirmDetectorDynamic(confirmAlert || {});

  if (normalizedSources.includes('claude')) {
    stops.push(startClaudeWatch({ intervalMs, quietPeriodMs: geminiQuietMs, claudeQuietMs, log, confirmDetector }));
  }
  if (normalizedSources.includes('codex')) {
    stops.push(startCodexWatch({ intervalMs, log, confirmDetector }));
  }
  if (normalizedSources.includes('gemini')) {
    stops.push(startGeminiWatch({ intervalMs, quietPeriodMs: geminiQuietMs, log, confirmDetector }));
  }

  return () => {
    for (const stop of stops) stop();
  };
}

module.exports = {
  startWatch
};


