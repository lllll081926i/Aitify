const fs = require('fs');
const path = require('path');
const os = require('os');
const { spawnSync } = require('child_process');
const packager = require('electron-packager');

function readPackageVersion() {
  const pkgPath = path.join(__dirname, '..', 'package.json');
  const raw = fs.readFileSync(pkgPath, 'utf8');
  const pkg = JSON.parse(raw);
  return pkg && pkg.version ? String(pkg.version) : '0.0.0';
}

function statfsFreeBytes(p) {
  try {
    // Available in modern Node (>=18). On Windows it maps to GetDiskFreeSpaceEx.
    const stat = fs.statfsSync(p);
    const bavail = Number(stat && stat.bavail);
    const bsize = Number(stat && stat.bsize);
    if (!Number.isFinite(bavail) || !Number.isFinite(bsize)) return null;
    return bavail * bsize;
  } catch (_error) {
    return null;
  }
}

function resolvePackagerTmpDir() {
  const env = String(
    process.env.ELECTRON_PACKAGER_TMPDIR
      || process.env.EP_TMPDIR
      || process.env.PACK_TMPDIR
      || ''
  ).trim();
  if (env) return env;

  const defaultTmp = os.tmpdir();
  const freeBytes = statfsFreeBytes(defaultTmp);
  const minFreeBytes = 2 * 1024 * 1024 * 1024; // 2GB safety floor to avoid partial builds
  if (freeBytes == null || freeBytes >= minFreeBytes) {
    return defaultTmp;
  }

  // Fallback to the current drive root to avoid C: temp exhaustion and Unicode path issues.
  // (electron-packager stages a full Electron template in tmpdir).
  const driveRoot = path.parse(process.cwd()).root;
  return path.join(driveRoot, 'ai-cli-complete-notify-tmp');
}

function escapeRegExp(value) {
  return String(value || '').replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function killWindowsProcessByImageName(imageName) {
  const exe = String(imageName || '').trim();
  if (!exe) return;
  try {
    spawnSync('taskkill', ['/F', '/T', '/IM', exe], {
      stdio: 'ignore',
      windowsHide: true
    });
  } catch (_error) {
    // ignore
  }
}

function killWindowsNotifyProcesses() {
  if (process.platform !== 'win32') return;
  try {
    spawnSync(
      'powershell',
      [
        '-NoProfile',
        '-ExecutionPolicy',
        'Bypass',
        '-Command',
        "$ErrorActionPreference='SilentlyContinue'; " +
          "Get-Process | Where-Object { " +
          "$_.ProcessName -like 'ai-cli-complete-notify*' -or " +
          "($_.Path -and $_.Path -like '*ai-cli-complete-notify*')" +
          " } | Stop-Process -Force"
      ],
      {
        stdio: 'ignore',
        windowsHide: true
      }
    );
  } catch (_error) {
    // ignore
  }
}

function sleepMs(ms) {
  const waitMs = Math.max(0, Number(ms) || 0);
  if (waitMs <= 0) return;
  try {
    const signal = new Int32Array(new SharedArrayBuffer(4));
    Atomics.wait(signal, 0, 0, waitMs);
  } catch (_error) {
    // ignore
  }
}

function removeDirNative(targetDir) {
  fs.rmSync(targetDir, {
    recursive: true,
    force: true,
    maxRetries: 2,
    retryDelay: 150
  });
}

function removeDirViaCmd(targetDir) {
  if (process.platform !== 'win32') return;
  try {
    spawnSync('cmd', ['/c', 'rmdir', '/s', '/q', targetDir], {
      stdio: 'ignore',
      windowsHide: true
    });
  } catch (_error) {
    // ignore
  }
}

function collectExeImageNames(targetDir, version) {
  const names = new Set([`ai-cli-complete-notify-${version}.exe`]);
  try {
    const entries = fs.readdirSync(targetDir, { withFileTypes: true });
    for (const entry of entries) {
      if (!entry.isFile()) continue;
      if (!/\.exe$/i.test(entry.name)) continue;
      names.add(entry.name);
    }
  } catch (_error) {
    // ignore
  }
  return [...names];
}

function removeDirWithRetries(targetDir, { version, attempts = 3 } = {}) {
  if (!fs.existsSync(targetDir)) return;

  const tryCount = Math.max(1, Number(attempts) || 1);
  const exeNames = collectExeImageNames(targetDir, version);
  let lastError = null;

  for (let i = 0; i < tryCount; i += 1) {
    try {
      removeDirNative(targetDir);
    } catch (_error) {
      lastError = _error;
      // ignore and retry
    }
    if (!fs.existsSync(targetDir)) return;

    if (process.platform === 'win32') {
      for (const exeName of exeNames) {
        killWindowsProcessByImageName(exeName);
      }
      killWindowsNotifyProcesses();
      try {
        removeDirNative(targetDir);
      } catch (_error) {
        lastError = _error;
      }
      if (!fs.existsSync(targetDir)) return;

      removeDirViaCmd(targetDir);
      if (!fs.existsSync(targetDir)) return;

      try {
        removeDirNative(targetDir);
      } catch (_error) {
        lastError = _error;
      }
      if (!fs.existsSync(targetDir)) return;
    }

    sleepMs(450);
  }

  const errorHint = lastError && lastError.message ? ` Last error: ${lastError.message}` : '';
  throw new Error(
    `[pack] failed to clean output dir: ${targetDir}. ` +
    `Please close running app processes and try again.${errorHint}`
  );
}

function cleanupOutputDirs({ outDir, version, platform, arch }) {
  fs.mkdirSync(outDir, { recursive: true });

  const base = `ai-cli-complete-notify-${version}`;
  const standardName = `${base}-${platform}-${arch}`;
  const standardDir = path.join(outDir, standardName);
  const anyBuildPattern = /^ai-cli-complete-notify-[0-9]+\.[0-9]+\.[0-9]+-build\d+-win32-x64$/i;

  let legacyDirs = [];
  try {
    legacyDirs = fs
      .readdirSync(outDir, { withFileTypes: true })
      .filter((entry) => entry.isDirectory())
      .map((entry) => entry.name)
      .filter((name) => anyBuildPattern.test(name))
      .map((name) => path.join(outDir, name));
  } catch (_error) {
    legacyDirs = [];
  }

  removeDirWithRetries(standardDir, { version, attempts: 8 });
  for (const legacyDir of legacyDirs) {
    removeDirWithRetries(legacyDir, { version, attempts: 4 });
  }
}

async function main() {
  const version = readPackageVersion();
  const outDir = path.join(__dirname, '..', 'dist');
  const platform = 'win32';
  const arch = 'x64';
  const name = `ai-cli-complete-notify-${version}`;
  cleanupOutputDirs({ outDir, version, platform, arch });

  const tmpdir = resolvePackagerTmpDir();
  const ignore = [
    // Build outputs / local secrets / packaging helpers
    /[\\/]dist([\\/]|$)/i,
    /[\\/]dist-test(-[^\\/]+)?([\\/]|$)/i,
    /(^|[\\/])\.env(\.[^\\/]+)?$/i,
    /(^|[\\/])package-zip\.ps1$/i,

    // Docs are not needed at runtime; excluding them reduces pack size and tmp usage.
    /[\\/]docs([\\/]|$)/i
  ];

  const appPaths = await packager({
    dir: path.join(__dirname, '..'),
    out: outDir,
    name,
    platform,
    arch,
    overwrite: true,
    prune: true,
    icon: path.join(__dirname, '..', 'desktop', 'assets', 'tray.ico'),
    tmpdir,
    ignore
  });

  const { runPostdist } = require('./postdist');
  for (const appPath of Array.isArray(appPaths) ? appPaths : []) {
    runPostdist(appPath);
  }
}

main().catch((error) => {
  console.error('[pack] failed:', error && error.message ? error.message : error);
  process.exit(1);
});
