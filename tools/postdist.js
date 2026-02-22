const fs = require('fs');
const path = require('path');

function copyEnvExample(targetDir) {
  const envExample = path.join(process.cwd(), '.env.example');
  if (!fs.existsSync(envExample)) return;

  const destExample = path.join(targetDir, '.env.example');
  if (fs.existsSync(destExample)) return;

  fs.copyFileSync(envExample, destExample);
  console.log(`[postdist] copied .env.example -> ${destExample}`);
}

function removeTopEnvFiles(targetDir) {
  // Never ship real secrets next to the exe.
  let entries = [];
  try {
    entries = fs.readdirSync(targetDir, { withFileTypes: true });
  } catch (_error) {
    return;
  }

  for (const entry of entries) {
    if (!entry.isFile()) continue;
    const name = entry.name;
    if (!name.toLowerCase().startsWith('.env')) continue;
    if (name.toLowerCase() === '.env.example') continue;

    const fullPath = path.join(targetDir, name);
    try {
      fs.unlinkSync(fullPath);
      console.log(`[postdist] removed ${fullPath}`);
    } catch (error) {
      console.warn(`[postdist] failed to remove ${fullPath}: ${error && error.message ? error.message : error}`);
    }
  }
}

function removeDeepEnv(targetDir) {
  // Ensure no stray .env under resources/app
  const targets = [
    path.join(targetDir, 'resources', 'app', '.env'),
    path.join(targetDir, 'resources', 'app', '.env.example'),
    path.join(targetDir, 'resources', 'app', 'package-zip.ps1')
  ];
  for (const t of targets) {
    if (!fs.existsSync(t)) continue;
    try {
      fs.unlinkSync(t);
      console.log(`[postdist] removed ${t}`);
    } catch (error) {
      console.warn(`[postdist] failed to remove ${t}: ${error && error.message ? error.message : error}`);
    }
  }
}

function pickLatestBuildDir(distRoot) {
  let entries = [];
  try {
    entries = fs.readdirSync(distRoot, { withFileTypes: true });
  } catch (_error) {
    return '';
  }

  const candidates = entries
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .filter((name) => name.startsWith('ai-cli-complete-notify-') && name.endsWith('-win32-x64'))
    .map((name) => ({ name, full: path.join(distRoot, name) }));

  let latest = null;
  for (const item of candidates) {
    try {
      const stat = fs.statSync(item.full);
      if (!latest || stat.mtimeMs > latest.mtimeMs) {
        latest = { ...item, mtimeMs: stat.mtimeMs };
      }
    } catch (_error) {
      // ignore
    }
  }
  return latest ? latest.full : '';
}

function runPostdist(targetDir) {
  const dir = String(targetDir || '').trim();
  if (!dir) return;
  if (!fs.existsSync(dir)) return;

  copyEnvExample(dir);
  removeTopEnvFiles(dir);
  removeDeepEnv(dir);
}

function main() {
  const explicit = process.argv[2] ? String(process.argv[2]).trim() : '';
  if (explicit) {
    runPostdist(explicit);
    return;
  }

  const distRoot = path.join(process.cwd(), 'dist');
  const latest = pickLatestBuildDir(distRoot);
  if (!latest) {
    console.warn(`[postdist] dist target not found under: ${distRoot}`);
    return;
  }
  runPostdist(latest);
}

module.exports = {
  runPostdist
};

if (require.main === module) {
  main();
}

