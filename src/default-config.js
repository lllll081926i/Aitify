const DEFAULT_CONFIG = {
  version: 2,
  app: {
    host: '127.0.0.1',
    port: 3210
  },
  format: {
    includeSourcePrefixInTitle: true
  },
  summary: {
    enabled: false,
    provider: 'openai',
    apiUrl: '',
    apiKey: '',
    model: '',
    timeoutMs: 15000,
    maxTokens: 200,
    prompt: ''
  },
  ui: {
    language: 'zh-CN',
    closeBehavior: 'ask',
    autostart: false,
    silentStart: false,
    watchLogRetentionDays: 7,
    autoFocusOnNotify: false,
    forceMaximizeOnFocus: false,
    focusTarget: 'auto',
    confirmAlert: {
      enabled: false
    }
  },
  channels: {
    telegram: {
      enabled: true,
      botToken: '',
      chatId: '',
      botTokenEnv: 'TELEGRAM_BOT_TOKEN',
      chatIdEnv: 'TELEGRAM_CHAT_ID',
      proxyUrl: '',
      proxyEnvCandidates: ['HTTPS_PROXY', 'HTTP_PROXY', 'https_proxy', 'http_proxy']
    },
    sound: {
      enabled: true,
      tts: true,
      fallbackBeep: true,
      useCustom: false,
      customPath: ''
    },
    desktop: {
      enabled: true,
      balloonMs: 6000
    }
  },
  sources: {
    claude: {
      enabled: true,
      minDurationMinutes: 0,
      channels: {
        telegram: false,
        sound: true,
        desktop: true
      }
    },
    codex: {
      enabled: true,
      minDurationMinutes: 0,
      channels: {
        telegram: false,
        sound: true,
        desktop: true
      }
    },
    gemini: {
      enabled: true,
      minDurationMinutes: 0,
      channels: {
        telegram: false,
        sound: true,
        desktop: true
      }
    }
  }
};

module.exports = {
  DEFAULT_CONFIG
};
