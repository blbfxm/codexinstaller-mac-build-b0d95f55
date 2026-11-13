// Uses only an isolated CODEX_HOME and our own app-server child processes.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const root = path.resolve(__dirname, '..');
const source = process.argv[2];
assert(source && fs.statSync(source).isFile(), 'Pass a desktop resources/codex.exe path');
const fixture = path.join(root, 'src-tauri/target/reset-engine-integration');
fs.mkdirSync(fixture, { recursive: true });
const engine = path.join(fixture, process.platform === 'win32' ? 'codex.exe' : 'codex');
// WindowsApps binaries cannot always execute outside package activation. A read-only copy
// in the test directory allows a separate engine without changing the installed application.
if (path.resolve(source) !== engine) fs.copyFileSync(source, engine);
if (process.platform !== 'win32') fs.chmodSync(engine, 0o700);
const temp = path.join(root, 'src-tauri/target/bundle-temp'); fs.mkdirSync(temp, { recursive: true });
const result = spawnSync('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml', '--lib',
  'codex_reset::tests::real_account_api_accepts_the_reset_without_credentials', '--', '--ignored', '--nocapture'], {
  cwd: root, env: { ...process.env, TEMP: temp, TMP: temp, CODEX_RESET_TEST_ENGINE: engine },
  stdio: 'inherit', windowsHide: true, timeout: 300000,
});
if (result.error) throw result.error;
assert.equal(result.status, 0, 'Actual Codex account API regression check failed');
console.log('PASS: old empty-auth bug reproduced; actual reset returns logged-out account state and preserves history');
