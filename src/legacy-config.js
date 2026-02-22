const { DEFAULT_CONFIG } = require('./default-config');

function coerceBoolean(value) {
  if (typeof value === 'boolean') return value;
  if (typeof value === 'string') {
    if (value.toLowerCase() === 'true') return true;
    if (value.toLowerCase() === 'false') return false;
  }
  return undefined;
}

function migrateLegacyConfig(legacyConfig) {
  const config = JSON.parse(JSON.stringify(DEFAULT_CONFIG));

  if (legacyConfig && legacyConfig.app && typeof legacyConfig.app === 'object') {
    if (typeof legacyConfig.app.name === 'string') config.app.name = legacyConfig.app.name;
    if (typeof legacyConfig.app.version === 'string') config.app.version = legacyConfig.app.version;
    if (typeof legacyConfig.app.description === 'string') config.app.description = legacyConfig.app.description;
  }

  const notification = legacyConfig && legacyConfig.notification && typeof legacyConfig.notification === 'object'
    ? legacyConfig.notification
    : null;

  if (!notification) return config;

  const telegramEnabled = coerceBoolean(notification.telegram && notification.telegram.enabled);
  const soundEnabled = coerceBoolean(notification.sound && notification.sound.enabled);

  // Migrate legacy notification flags to new config structure
  if (telegramEnabled !== undefined) config.sources.claude.channels.telegram = telegramEnabled;
  if (soundEnabled !== undefined) config.sources.claude.channels.sound = soundEnabled;

  // Note: Feishu/webhook channel has been removed. Legacy feishu.enabled and webhook_url are no longer migrated.

  return config;
}

module.exports = {
  migrateLegacyConfig
};
