const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const project = path.resolve(__dirname, '..');
const tempRoot = path.join(project, 'src-tauri/target/detection-tests');
fs.mkdirSync(tempRoot, { recursive: true });
const root = fs.mkdtempSync(path.join(tempRoot, 'codex-中文 '));
const script = path.join(project, 'src-tauri/src/discover_codex_windows.ps1');
const harness = path.join(root, 'discover-fixture.ps1');
const packageName = version => 'OpenAI.Codex_' + version + '_x64__2p2nqsd0c76g0';
function fixture(drive, version, type = 'desktop') {
  const folder = path.join(root, drive, 'WindowsApps', packageName(version));
  fs.mkdirSync(path.join(folder, 'app/resources'), { recursive: true });
  fs.writeFileSync(path.join(folder, 'AppxManifest.xml'), `<Package><Identity Name="${type === 'wrong-app' ? 'Other.App' : 'OpenAI.Codex'}" Version="${version}"/><Applications><Application Id="App" Executable="app\\ChatGPT.exe"/></Applications></Package>`);
  if (type === 'desktop' || type === 'wrong-app') fs.writeFileSync(path.join(folder, 'app/ChatGPT.exe'), 'fixture');
  if (type === 'cli') fs.writeFileSync(path.join(folder, 'app/resources/codex.exe'), 'fixture-cli');
  return folder;
}
const old = fixture('E盘应用', '26.1103.7773.0');
const stale = fixture('E盘应用', '26.1104.1.0', 'missing');
const newer = fixture('F盘应用', '26.1105.1.0');
const cli = fixture('只有命令行', '26.1103.7773.0', 'cli');
const wrong = fixture('其他应用', '26.1103.7773.0', 'wrong-app');
const custom = path.join(root, 'D盘应用', '自定义安装目录');
fs.mkdirSync(path.join(custom, 'resources'), { recursive: true });
fs.writeFileSync(path.join(custom, 'ChatGPT.exe'), 'fixture-desktop');
fs.writeFileSync(path.join(custom, 'resources/codex.exe'), 'fixture-engine');
const unrelated = path.join(root, '普通ChatGPT');
fs.mkdirSync(unrelated, { recursive: true });
fs.writeFileSync(path.join(unrelated, 'ChatGPT.exe'), 'fixture-other');
fs.writeFileSync(harness, '\ufeff' + String.raw`
$ErrorActionPreference='Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
. $env:CODEX_DISCOVERY_SCRIPT
function Get-AppxPackage {
    param($Name,$ErrorAction)
    if ($env:CODEX_DISCOVERY_CASE -eq 'module-failure') { throw 'Appx module unavailable' }
    if ($env:CODEX_DISCOVERY_CASE -in @('registered','stale','registered-priority')) {
        [PSCustomObject]@{InstallLocation=$env:CODEX_DISCOVERY_OLD; Version='26.1103.7773.0'}
    }
    if ($env:CODEX_DISCOVERY_CASE -eq 'stale') {
        [PSCustomObject]@{InstallLocation=$env:CODEX_DISCOVERY_STALE; Version='26.1104.1.0'}
    }
}
function Get-AppxVolume { param($ErrorAction) }
function Get-PSDrive {
    param($PSProvider,$ErrorAction)
    foreach ($drive in ($env:CODEX_DISCOVERY_DRIVES -split ';')) {
        if ($drive) { [PSCustomObject]@{Root=$drive;DisplayRoot=''} }
    }
}
function Get-ChildItem {
    param($LiteralPath,$Filter,[switch]$Directory,$ErrorAction)
    if ($LiteralPath -like 'HK*:*') {
        if ($env:CODEX_DISCOVERY_CASE -eq 'repository' -and $LiteralPath -like 'HKCU:*') {
            [PSCustomObject]@{PSChildName='OpenAI.Codex_26.1103.7773.0_x64__2p2nqsd0c76g0';PSPath='fixture-repository'}
        }
        return
    }
    Microsoft.PowerShell.Management\Get-ChildItem -LiteralPath $LiteralPath -Filter $Filter -Directory:$Directory -ErrorAction SilentlyContinue
}
function Get-ItemProperty {
    param($LiteralPath,$ErrorAction)
    if ($LiteralPath -eq 'fixture-repository') { [PSCustomObject]@{PackageRootFolder=$env:CODEX_DISCOVERY_OLD} }
}
function Get-CodexExternalRoots {
    if ($env:CODEX_DISCOVERY_CASE -eq 'custom-directory') { $env:CODEX_DISCOVERY_CUSTOM }
    if ($env:CODEX_DISCOVERY_CASE -eq 'custom-cli') { Join-Path $env:CODEX_DISCOVERY_CLI 'app\resources' }
    if ($env:CODEX_DISCOVERY_CASE -eq 'unrelated-chatgpt') { $env:CODEX_DISCOVERY_UNRELATED }
}
Find-CodexInstallation | ConvertTo-Json -Compress
`);
try {
  for (const [name, drives, expected] of [
    ['registered', [], old],
    ['stale', [], old],
    ['module-failure', ['E盘应用'], old],
    ['repository', [], old],
    ['latest-disk', ['E盘应用', 'F盘应用'], newer],
    ['registered-priority', ['F盘应用'], old],
    ['cli-only', ['只有命令行'], null],
    ['missing', ['空磁盘'], null],
    ['wrong-app', ['其他应用'], null],
    ['custom-directory', [], custom],
    ['custom-cli', [], null],
    ['unrelated-chatgpt', [], null],
  ]) {
    const result = spawnSync('powershell.exe', ['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',harness], {
      env: {...process.env,CODEX_DISCOVERY_SCRIPT:script,CODEX_DISCOVERY_CASE:name,CODEX_DISCOVERY_OLD:old,CODEX_DISCOVERY_STALE:stale,CODEX_DISCOVERY_DRIVES:drives.map(d=>path.join(root,d)).join(';'),CODEX_DISCOVERY_CUSTOM:custom,CODEX_DISCOVERY_CLI:cli,CODEX_DISCOVERY_UNRELATED:unrelated},
      encoding:'utf8',timeout:20000,windowsHide:true,
    });
    assert.equal(result.error, undefined);
    assert.equal(result.status, 0, result.stderr);
    if (expected) {
      const data=JSON.parse(result.stdout.trim());
      assert.equal(path.normalize(data.root),expected);
      assert.equal(path.normalize(data.executable),path.join(expected,name==='custom-directory'?'ChatGPT.exe':'app/ChatGPT.exe'));
      if (name !== 'custom-directory') assert.ok(data.version);
    } else assert.ok(!result.stdout.trim() || result.stdout.trim()==='null', result.stdout);
    console.log('PASS:', name);
  }
} finally { fs.rmSync(root,{recursive:true,force:true}); }
