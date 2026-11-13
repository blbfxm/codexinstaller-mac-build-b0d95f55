// Run the actual installer with mocked Appx/UAC/process commands. Data copies,
// hashes and recovery records are real, isolated under a temporary fixture.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const root = fs.mkdtempSync(path.join(os.tmpdir(), 'codex-msix-中文 '));
const script = path.join(root, 'install.ps1');
fs.writeFileSync(script, '\ufeff' + fs.readFileSync(path.resolve(__dirname, '../src-tauri/src/install_codex_msix.ps1'), 'utf8'));
const harness = path.join(root, 'mock.ps1');
fs.writeFileSync(harness, '\ufeff' + String.raw`
$ErrorActionPreference = 'Stop'
$scenario = $env:CODEX_MSIX_SCENARIO
$global:installed = $scenario -like 'target-relocate*' -or $scenario -eq 'target-existing-match'
$global:removed = $false
$global:onTarget = $scenario -eq 'target-existing-match'
$global:volumeReady = $scenario -notin @('target-create','target-create-fail')
$global:provisioned = $scenario -like 'target-relocate*'
$global:elevated = $false
$global:licenseReady = $false
$global:stagedOld = $scenario -like 'target-relocate*'
$global:otherUser = $scenario -eq 'target-relocate-other-user'
$global:targetStore = Join-Path $env:CODEX_MSIX_TEST_ROOT 'target'
$global:packageName = 'OpenAI.Codex_26.1103.7773.0_x64__2p2nqsd0c76g0'
$global:sourceStore = 'C:\Program Files\WindowsApps\OpenAI.Codex_test'
$global:family = 'OpenAI.Codex_2p2nqsd0c76g0'
function Trace($Message) { Add-Content -LiteralPath (Join-Path $env:CODEX_MSIX_TEST_ROOT 'trace.txt') -Value $Message -Encoding UTF8 }
function Get-Process {
    param($Id,$ErrorAction)
    if ($Id) { return [PSCustomObject]@{ Id=$Id; SessionId=1 } }
    [PSCustomObject]@{ Id=111; SessionId=1; Path=($global:sourceStore + '\app\ChatGPT.exe') }
    [PSCustomObject]@{ Id=222; SessionId=2; Path=($global:sourceStore + '\app\ChatGPT.exe') }
    [PSCustomObject]@{ Id=333; SessionId=1; Path='C:\Unrelated\Application.exe' }
}
function Stop-Process {
    param($Id,[switch]$Force,$ErrorAction)
    if ($Id -ne 111) { throw 'UNEXPECTED TERMINATION OF ANOTHER SESSION OR APP' }
    Trace 'stop-codex'
}
function Get-AppxVolume {
    param($ErrorAction)
    if ($global:volumeReady) { [PSCustomObject]@{ PackageStorePath=$global:targetStore; IsOffline=($scenario -eq 'target-offline') } }
}
function Add-AppxVolume {
    param($Path,$ErrorAction)
    if ($Path -ne $global:targetStore) { throw 'UNEXPECTED STORE PATH' }
    if ($scenario -eq 'target-create-fail') { throw 'INSTALL_LOCATION_FAILED: target volume creation denied' }
    Trace 'create-volume'; $global:volumeReady=$true
    [PSCustomObject]@{ PackageStorePath=$Path; IsOffline=$false }
}
function Move-AppxPackage { throw 'UNEXPECTED MOVE: relocation must uninstall and reinstall' }
function Add-AppxPackage {
    param($Path,$Volume,[switch]$Stage,[switch]$Register,[switch]$ForceApplicationShutdown,[switch]$ForceUpdateFromAnyVersion,$ErrorAction)
    if ($Stage) {
        if (-not $global:elevated -or $Volume.PackageStorePath -ne $global:targetStore) { throw 'UNEXPECTED STAGING CONTEXT' }
        if ($scenario -eq 'target-stage-on-c') { Trace 'stage-reused-c'; return }
        $directory = Join-Path $global:targetStore $global:packageName
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
        [IO.File]::WriteAllText((Join-Path $directory 'AppxManifest.xml'), '<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"><Identity Name="OpenAI.Codex" Publisher="CN=OpenAI" Version="26.1103.7773.0" ProcessorArchitecture="x64" /></Package>', (New-Object Text.UTF8Encoding($false)))
        Trace ('stage-target:' + $Volume.PackageStorePath)
        return
    }
    if ($Register) {
        if (-not $global:elevated) { throw 'UNEXPECTED NON-ELEVATED REGISTER' }
        Trace ('register-target:' + $Path)
        Trace ('force-app-shutdown:' + $ForceApplicationShutdown.IsPresent)
        if ($scenario -eq 'target-license' -and -not $global:licenseReady) { throw 'HRESULT 0x80073CF3: license required' }
        if ($scenario -in @('target-relocate-install-fail','target-unrelated-failure')) { throw 'HRESULT 0x80073CF3: missing dependency' }
        if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw 'REGISTER MANIFEST IS NOT IN TARGET STORE' }
        $global:installed=$true
        $global:onTarget=($scenario -notin @('target-register-on-c','target-wrong-disk'))
        return
    }
    Trace ('install-volume:' + $Volume.PackageStorePath)
    Trace ('force-app-shutdown:' + $ForceApplicationShutdown.IsPresent)
    if ($scenario -in @('target-packaged-service','target-service-other-user') -and -not $global:elevated) { throw 'HRESULT 0x80073D28: packaged service requires administrator privileges' }
    if ($scenario -in @('target-relocate-install-fail','target-unrelated-failure')) { throw 'HRESULT 0x80073CF3: missing dependency' }
    if ($scenario -in @('fallback','cancel','elevation-failure','provision-failure') -and -not $global:licenseReady) { throw 'HRESULT 0x80073CF3: license required' }
    if ($scenario -eq 'target-conflict' -and -not $global:removed -and $Volume) { throw 'HRESULT 0x80073CF9: application data is registered on C:' }
    if ($scenario -eq 'target-conflict-hresult' -and -not $global:removed -and $Volume) { throw [System.Runtime.InteropServices.COMException]::new('Localized deployment failure', -2147009287) }
    if ($scenario -eq 'unregistered') { return }
    if ($global:stagedOld) {
        # Windows reuses the C: staged payload and ignores -Volume until the
        # staged registration is removed with -AllUsers.
        Trace 'reuse-staged-c'
        $global:installed=$true; $global:onTarget=$false
    } else {
        $global:installed=$true; $global:onTarget=[bool]$Volume
    }
    if ($scenario -eq 'target-wrong-disk') { $global:onTarget=$false }
}
function Get-AppxPackage {
    param($Name,[switch]$AllUsers,$ErrorAction)
    if ($Name -ne 'OpenAI.Codex') { throw 'UNEXPECTED PACKAGE QUERY' }
    if ($global:installed) {
        $location = if ($global:onTarget) { $global:targetStore + '\OpenAI.Codex_test' } else { $global:sourceStore }
        $users = @([PSCustomObject]@{ UserSecurityId=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value; InstallState='Installed' })
        if ($AllUsers -and $global:otherUser) { $users += [PSCustomObject]@{ UserSecurityId='S-1-5-21-another-user'; InstallState='Installed' } }
        [PSCustomObject]@{ Name='OpenAI.Codex'; Version='1.0.0'; PackageFullName=$global:packageName; PackageFamilyName=$global:family; InstallLocation=$location; PackageUserInformation=$users }
    }
}
function Remove-AppxPackage {
    param($Package,[switch]$AllUsers,$User,$ErrorAction)
    if ($Package -ne $global:packageName -or $User -or -not $AllUsers) { throw 'UNEXPECTED REMOVAL OF OTHER USERS OR APPS' }
    if (-not $global:elevated) { throw 'UNEXPECTED NON-ELEVATED ALL-USERS REMOVAL' }
    $pending = Get-Content -LiteralPath (Join-Path $env:CODEX_MSIX_TEST_ROOT 'codex-migration-pending.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($pending.hasData -and -not (Test-Path -LiteralPath (Join-Path $pending.backupPath 'LocalState\conversation-state.json'))) { throw 'UNEXPECTED REMOVAL WITHOUT BACKUP' }
    Trace 'remove-all-users-codex'
    if ($scenario -eq 'target-relocate-remove-fail') { throw 'HRESULT 0x80070005: removal denied' }
    $global:installed=$false; $global:removed=$true; $global:stagedOld=$false
    $data = Join-Path $env:LOCALAPPDATA ('Packages\' + $global:family)
    # Emulate two real Windows outcomes: normal uninstall removes the folder,
    # while some builds leave it linked to the old volume.
    if ($scenario -eq 'target-relocate-data-lingering') {
        if (Test-Path -LiteralPath $data) { Set-Content -LiteralPath (Join-Path $data 'old-volume.marker') -Value 'stale' }
    } elseif (Test-Path -LiteralPath $data) { Remove-Item -LiteralPath $data -Recurse -Force }
}
function Add-AppxProvisionedPackage {
    param([switch]$Online,$PackagePath,$LicensePath,$Regions,$ErrorAction)
    Trace 'provision-package'
    if ($scenario -eq 'provision-failure') { throw 'HRESULT 0x80070005: access denied' }
    $global:licenseReady=$true
}
function Get-AppxProvisionedPackage {
    param([switch]$Online,$ErrorAction)
    if ($global:provisioned) { [PSCustomObject]@{ PackageName=$global:packageName } }
    [PSCustomObject]@{ PackageName='Microsoft.UnrelatedApp_1.0.0.0_x64__8wekyb3d8bbwe' }
}
function Remove-AppxProvisionedPackage {
    param([switch]$Online,$PackageName,[switch]$AllUsers,$ErrorAction)
    if ($AllUsers -or $PackageName -ne $global:packageName -or -not $global:elevated) { throw 'UNEXPECTED DEPROVISION CONTEXT' }
    Trace 'deprovision-codex'; $global:provisioned=$false
}
function Get-FileHash {
    param($LiteralPath,$Algorithm)
    if ($scenario -eq 'target-relocate-backup-fail' -and $LiteralPath.Contains('CodexMoveBackup')) { return [PSCustomObject]@{ Hash='invalid' } }
    if ($scenario -eq 'target-relocate-restore-fail' -and $global:removed -and -not $LiteralPath.Contains('CodexMoveBackup')) { return [PSCustomObject]@{ Hash='invalid' } }
    $stream = [IO.File]::OpenRead($LiteralPath)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { [PSCustomObject]@{ Hash=[BitConverter]::ToString($sha.ComputeHash($stream)) } }
    finally { $sha.Dispose(); $stream.Dispose() }
}
function Start-Process {
    param($FilePath,$Verb,$ArgumentList,$WindowStyle,[switch]$Wait,[switch]$PassThru,$ErrorAction)
    if ($WindowStyle -ne 'Hidden' -or $Wait) { throw 'UNEXPECTED HELPER WINDOW OR DESCENDANT WAIT' }
    Trace 'elevate'
    if ($scenario -in @('direct','old-windows')) { throw 'UNEXPECTED ELEVATION' }
    if ($scenario -in @('cancel','target-relocate-cancel')) { throw (New-Object System.ComponentModel.Win32Exception(1223)) }
    if ($scenario -eq 'elevation-failure') { throw 'PowerShell was blocked by policy' }
    $helper = @{ PackagePath=(Join-Path $env:CODEX_MSIX_TEST_ROOT 'package with spaces.msix'); LicensePath=(Join-Path $env:CODEX_MSIX_TEST_ROOT 'license.xml'); ErrorPath=(Join-Path $env:CODEX_MSIX_TEST_ROOT 'error.txt') }
    $helper.ExpectedUserSid = if ($scenario -eq 'target-service-other-user') { 'S-1-5-21-different-account' } else { [Security.Principal.WindowsIdentity]::GetCurrent().User.Value }
    if ($scenario.StartsWith('target-')) { $helper.InstallLocation=$global:targetStore }
    if ($ArgumentList.Contains('-PrepareVolumeOnly')) { $helper.PrepareVolumeOnly=$true; Trace 'prepare-volume' }
    elseif ($ArgumentList.Contains('-UnprovisionPackage')) { $helper.UnprovisionPackage=$global:packageName; Trace 'unprovision-helper' }
    elseif ($ArgumentList.Contains('-ProvisionOnly')) { $helper.ProvisionOnly=$true; Trace 'prepare-license' }
    elseif ($ArgumentList.Contains('-InstallToSelectedVolume')) { $helper.InstallToSelectedVolume=$true; Trace 'install-target'; Trace 'elevated-register' }
    elseif ($ArgumentList.Contains('-RegisterAsOriginalUser')) { $helper.RegisterAsOriginalUser=$true; Trace 'elevated-register' }
    else { throw 'UNEXPECTED ELEVATED OPERATION' }
    $global:elevated=$true
    try { & $env:CODEX_MSIX_TEST_SCRIPT @helper; $exitCode=$LASTEXITCODE }
    finally { $global:elevated=$false }
    $result=[PSCustomObject]@{ ExitCode=$exitCode }
    $result | Add-Member -MemberType ScriptMethod -Name WaitForExit -Value { }
    $result
}
function Get-ItemProperty {
    param($LiteralPath)
    [PSCustomObject]@{ CurrentMajorVersionNumber=10; CurrentMinorVersionNumber=0; CurrentBuildNumber=$(if ($scenario -eq 'old-windows') { '18363' } else { '19041' }); UBR=0 }
}
$argsForInstall=@{ PackagePath=(Join-Path $env:CODEX_MSIX_TEST_ROOT 'package with spaces.msix'); LicensePath=(Join-Path $env:CODEX_MSIX_TEST_ROOT 'license.xml'); ErrorPath=(Join-Path $env:CODEX_MSIX_TEST_ROOT 'error.txt') }
if ($scenario.StartsWith('target-')) { $argsForInstall.InstallLocation=$global:targetStore }
Add-Type -AssemblyName System.IO.Compression.FileSystem
if (-not (Test-Path -LiteralPath $argsForInstall.PackagePath)) {
    $zip=[System.IO.Compression.ZipFile]::Open($argsForInstall.PackagePath,'Create')
    try {
        $entry=$zip.CreateEntry('AppxManifest.xml'); $writer=New-Object System.IO.StreamWriter($entry.Open())
        try { $writer.Write('<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"><Identity Name="OpenAI.Codex" Publisher="CN=OpenAI" Version="26.1103.7773.0" ProcessorArchitecture="x64" /><Dependencies><TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.19041.0" /></Dependencies></Package>') } finally { $writer.Dispose() }
    } finally { $zip.Dispose() }
}
if ($env:CODEX_MSIX_PROVISION -eq '1') { $argsForInstall.ProvisionOnly=$true }
if ($scenario -eq 'target-unprovision-without-backup') {
    $argsForInstall.UnprovisionPackage=$global:packageName
    $argsForInstall.ExpectedUserSid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
}
& $env:CODEX_MSIX_TEST_SCRIPT @argsForInstall
exit $LASTEXITCODE
`);
const localAppData=path.join(root,'user-profile','AppData','Local');
const originalFile=path.join(localAppData,'Packages','OpenAI.Codex_2p2nqsd0c76g0','LocalState','conversation-state.json');
const original='{"preserve":"中文会话数据"}';
const pluginFile=path.join(localAppData,'Packages','OpenAI.Codex_2p2nqsd0c76g0','LocalState','plugins','demo','plugin.json');
const skillFile=path.join(localAppData,'Packages','OpenAI.Codex_2p2nqsd0c76g0','LocalState','skills','demo','SKILL.md');
const userCodexSkill=path.join(root,'user-profile','.codex','skills','demo','SKILL.md');
const pending=path.join(root,'codex-migration-pending.json');
const backupRoot=path.join(root,'CodexMoveBackup');
const sibling=path.join(backupRoot,'unrelated-backup','keep.txt');
function removeFixture(child) {
  const target=path.resolve(root,child);
  assert.ok(target.startsWith(path.resolve(root)+path.sep));
  fs.rmSync(target,{force:true,recursive:true});
}
function run(scenario, provision=false) {
  const logPath=path.join(root,'error.txt');
  for (const child of ['error.txt','error.txt.helper-error.json','trace.txt','target']) removeFixture(child);
  const result=spawnSync('powershell.exe',['-NoProfile','-NonInteractive','-ExecutionPolicy','Bypass','-File',harness],{
    env:{...process.env,LOCALAPPDATA:localAppData,CODEX_MSIX_SCENARIO:scenario,CODEX_MSIX_PROVISION:provision?'1':'0',CODEX_MSIX_TEST_ROOT:root,CODEX_MSIX_TEST_SCRIPT:script},
    encoding:'utf8',timeout:60000,windowsHide:true,
  });
  assert.equal(result.error,undefined);
  const log=fs.existsSync(logPath)?fs.readFileSync(logPath,'utf8'):'';
  const trace=fs.existsSync(path.join(root,'trace.txt'))?fs.readFileSync(path.join(root,'trace.txt'),'utf8'):'';
  assert.ok(!log.includes('UNEXPECTED'),`${scenario}: ${log}\n${trace}`);
  return {status:result.status,log,trace,output:result.stdout+result.stderr};
}
try {
  const scenarios=[
    ['old-windows',1603,'UNSUPPORTED_WINDOWS'],['direct',0],['fallback',0,'license required'],
    ['cancel',1223,'UAC_CANCELLED'],['elevation-failure',1,'blocked by policy'],
    ['provision-failure',1,'0x80070005'],['unregistered',1,'not registered'],
    ['target-direct',0],['target-existing-match',0],['target-create',0],['target-license',0],
    ['target-create-fail',1,'target volume creation denied'],['target-offline',1,'INSTALL_LOCATION_FAILED'],
    ['target-unrelated-failure',1,'missing dependency'],['target-packaged-service',0],
    ['target-stage-on-c',1,'CODEX_STAGE_LOCATION_FAILED'],['target-register-on-c',1,'CODEX_STAGE_REGISTRATION_FAILED'],
    ['target-service-other-user',1,'INSTALL_LOCATION_USER_MISMATCH'],
    ['target-relocate',0],['target-relocate-e',0],['target-relocate-f',0],['target-relocate-no-data',0],
    ['target-relocate-other-user',1,'CODEX_MIGRATION_OTHER_USERS'],
    ['target-unprovision-without-backup',1,'CODEX_MIGRATION_BACKUP_FAILED'],
    ['target-relocate-install-fail',1,'missing dependency'],['target-relocate-remove-fail',1,'removal denied'],
    ['target-relocate-backup-fail',1,'CODEX_MIGRATION_BACKUP_FAILED'],
    ['target-relocate-restore-fail',1,'CODEX_MIGRATION_RESTORE_FAILED'],['target-relocate-cancel',1223,'UAC_CANCELLED'],
    ['target-conflict',0],['target-conflict-hresult',0],['target-relocate-data-lingering',0],
    ['target-wrong-disk',1,'CODEX_STAGE_REGISTRATION_FAILED'],
  ];
  for (const [scenario,expected,detail] of scenarios) {
    if (process.env.CODEX_MSIX_ONLY && !scenario.includes(process.env.CODEX_MSIX_ONLY)) continue;
    for (const child of ['user-profile','CodexMoveBackup','codex-migration-pending.json','target']) removeFixture(child);
    const hasData=scenario.startsWith('target-relocate') && scenario!=='target-relocate-no-data';
    if (hasData) {
      fs.mkdirSync(path.dirname(originalFile),{recursive:true}); fs.writeFileSync(originalFile,original);
      fs.mkdirSync(path.dirname(pluginFile),{recursive:true}); fs.writeFileSync(pluginFile,'{"plugin":"demo"}');
      fs.mkdirSync(path.dirname(skillFile),{recursive:true}); fs.writeFileSync(skillFile,'# Demo skill');
    }
    fs.mkdirSync(path.dirname(userCodexSkill),{recursive:true}); fs.writeFileSync(userCodexSkill,'# User-owned skill');
    fs.mkdirSync(path.dirname(sibling),{recursive:true}); fs.writeFileSync(sibling,'keep');
    const result=run(scenario);
    assert.equal(result.status,expected,`${scenario}: ${result.output}\n${result.log}\n${result.trace}`);
    if (detail) assert.ok(result.log.includes(detail),result.log);
    if (scenario==='target-existing-match') assert.ok(!result.trace.includes('remove-all-users-codex'));
    if (scenario==='target-packaged-service') assert.ok(result.trace.includes('elevated-register') && result.trace.includes('force-app-shutdown:True'));
    if (scenario.startsWith('target-') && expected===0 && scenario!=='target-existing-match') {
      assert.ok(result.trace.includes('stage-target:'),result.trace);
      assert.ok(result.trace.includes('register-target:'),result.trace);
    }
    if (scenario==='target-stage-on-c') assert.ok(!result.trace.includes('register-target:'),result.trace);
    if (scenario.startsWith('target-relocate') && expected===0) {
      assert.ok(result.trace.includes('remove-all-users-codex'),result.trace);
      assert.ok(result.trace.indexOf('remove-all-users-codex')<result.trace.indexOf('stage-target:'),result.trace);
      assert.ok(result.trace.indexOf('stop-codex')<result.trace.indexOf('remove-all-users-codex'),result.trace);
      assert.ok(!result.trace.includes('reuse-staged-c'),result.trace);
      if (hasData) {
        assert.equal(fs.readFileSync(originalFile,'utf8'),original);
        assert.equal(fs.readFileSync(pluginFile,'utf8'),'{"plugin":"demo"}');
        assert.equal(fs.readFileSync(skillFile,'utf8'),'# Demo skill');
      }
      assert.equal(fs.readFileSync(userCodexSkill,'utf8'),'# User-owned skill');
      assert.ok(!fs.existsSync(pending));
      assert.deepEqual(fs.readdirSync(backupRoot),['unrelated-backup']);
    }
    if (scenario==='target-relocate-backup-fail') assert.ok(!result.trace.includes('remove-all-users-codex'));
    if (scenario==='target-relocate-other-user') {
      assert.ok(!result.trace.includes('remove-all-users-codex'));
      assert.ok(!result.trace.includes('deprovision-codex'));
    }
    if (scenario==='target-unprovision-without-backup') assert.ok(!result.trace.includes('deprovision-codex'));
    if (scenario==='target-relocate-cancel' || scenario==='target-relocate-remove-fail') assert.equal(fs.readFileSync(originalFile,'utf8'),original);
    if (scenario==='target-relocate-data-lingering') {
      assert.ok(!fs.existsSync(path.join(path.dirname(originalFile),'old-volume.marker')));
      assert.equal(fs.readFileSync(originalFile,'utf8'),original);
    }
    if (scenario==='target-relocate-install-fail' || scenario==='target-relocate-restore-fail') {
      const state=JSON.parse(fs.readFileSync(pending,'utf8'));
      assert.equal(fs.readFileSync(path.join(state.backupPath,'LocalState','conversation-state.json'),'utf8'),original);
      const retry=run('target-retry');
      assert.equal(retry.status,0,`${retry.log}\n${retry.trace}`);
      assert.equal(fs.readFileSync(originalFile,'utf8'),original);
      assert.ok(!fs.existsSync(pending));
      assert.deepEqual(fs.readdirSync(backupRoot),['unrelated-backup']);
      console.log('PASS:',scenario,'-> retry restores original data');
    }
    assert.equal(fs.readFileSync(sibling,'utf8'),'keep');
    console.log('PASS:',scenario);
  }
  if (!process.env.CODEX_MSIX_ONLY) for (const [scenario,expected] of [['old-windows',1603],['fallback',0],['provision-failure',1]]) {
    const result=run(scenario,true); assert.equal(result.status,expected,result.log); console.log('PASS:',scenario,'(provision only)');
  }
} finally {
  const resolved=path.resolve(root);
  assert.ok(resolved.startsWith(path.resolve(os.tmpdir())+path.sep) && path.basename(resolved).startsWith('codex-msix-'));
  fs.rmSync(resolved,{recursive:true,force:true});
}
