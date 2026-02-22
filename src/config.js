const fs = require('fs');
const path = require('path');
const { DEFAULT_CONFIG } = require('./default-config');
const { migrateLegacyConfig } = require('./legacy-config');
const { getSettingsPath, getDataDir } = require('./paths');

const SETTINGS_PATH = getSettingsPath();

function ensureDataDir() {
  const dir = getDataDir();
  if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
}

function parseEnvBoolean(value) {
  if (value === undefined) return undefined;
  if (typeof value === 'boolean') return value;
  if (typeof value !== 'string') return undefined;
  const normalized = value.trim().toLowerCase();
  if (normalized === 'true') return true;
  if (normalized === 'false') return false;
  return undefined;
}

function applyEnvOverrides(config) {
  const soundEnabled = parseEnvBoolean(process.env.SOUND_ENABLED);
  if (soundEnabled !== undefined) {
    config.channels.sound.enabled = soundEnabled;
  }

  return config;
}

function deepMerge(base, override) {
  if (!override || typeof override !== 'object') return base;
  const output = Array.isArray(base) ? [...base] : { ...base };
  for (const [key, value] of Object.entries(override)) {
    if (value && typeof value === 'object' && !Array.isArray(value) && base && typeof base[key] === 'object' && !Array.isArray(base[key])) {
      output[key] = deepMerge(base[key], value);
    } else {
      output[key] = value;
    }
  }
  return output;
}

function normalizeConfig(rawConfig) {
  if (rawConfig && rawConfig.version === 2) {
    return deepMerge(DEFAULT_CONFIG, rawConfig);
  }
  return migrateLegacyConfig(rawConfig || {});
}

function loadRawConfig() {
  try {
    if (fs.existsSync(SETTINGS_PATH)) {
      // Be tolerant of UTF-8 BOM and Windows editors that may write UTF-16.
      const buf = fs.readFileSync(SETTINGS_PATH);
      if (buf.length >= 2 && buf[0] === 0xff && buf[1] === 0xfe) {
        const content = buf.slice(2).toString('utf16le').replace(/^\uFEFF/, '');
        return JSON.parse(content);
      }
      if (buf.length >= 2 && buf[0] === 0xfe && buf[1] === 0xff) {
        const swapped = Buffer.alloc(Math.max(0, buf.length - 2));
        for (let i = 2; i + 1 < buf.length; i += 2) {
          swapped[i - 2] = buf[i + 1];
          swapped[i - 1] = buf[i];
        }
        const content = swapped.toString('utf16le').replace(/^\uFEFF/, '');
        return JSON.parse(content);
      }

      const content = buf.toString('utf8').replace(/^\uFEFF/, '');
      return JSON.parse(content);
    }

    // 兼容：旧版本可能在项目根目录有 config.json
    const legacyPath = path.join(__dirname, '..', 'config.json');
    if (!fs.existsSync(legacyPath)) return null;
    const content = fs.readFileSync(legacyPath, 'utf8').replace(/^\uFEFF/, '');
    return JSON.parse(content);
  } catch (error) {
    return null;
  }
}

function loadConfig() {
  return applyEnvOverrides(normalizeConfig(loadRawConfig()));
}

function saveConfig(config) {
  const normalized = normalizeConfig(config);
  ensureDataDir();
  fs.writeFileSync(SETTINGS_PATH, JSON.stringify(normalized, null, 2), 'utf8');
  return normalized;
}

function getConfigPath() {
  return SETTINGS_PATH;
}

module.exports = {
  loadConfig,
  saveConfig,
  getConfigPath,
  normalizeConfig
};
