param(
    [Parameter(Mandatory = $true)][string]$PackagePath,
    [Parameter(Mandatory = $true)][string]$LicensePath,
    [Parameter(Mandatory = $true)][string]$ErrorPath,
    [string]$InstallLocation = '',
    [switch]$ProvisionOnly,
    [switch]$PrepareVolumeOnly,
    [switch]$InstallToSelectedVolume,
    [switch]$RegisterAsOriginalUser,
    [string]$UnprovisionPackage = '',
    [string]$ExpectedUserSid = '',
    [string]$StatusPath = ''
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$OutputEncoding = [Console]::OutputEncoding
$ProgressPreference = 'SilentlyContinue'
if (-not $StatusPath) { $StatusPath = $ErrorPath + '.status.json' }

function Write-InstallStatus([string]$Stage, [string]$Message) {
    $json = @{ stage=$Stage; message=$Message } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText($StatusPath, $json, (New-Object Text.UTF8Encoding($false)))
}

function Save-InstallError($Phase, $Record) {
    $detail = $Record | Format-List * -Force | Out-String
    "[$Phase]`r`n$detail" | Add-Content -LiteralPath $ErrorPath -Encoding UTF8
}

function Test-DeploymentCode($Record, [int]$Code, [string]$HexCode) {
    # Data-volume conflicts use the generic deployment HRESULT. Check the
    # exception chain too, so recovery does not depend on Windows' language.
    $cause = $Record.Exception
    while ($cause) {
        if ($cause.HResult -eq $Code -or $cause.Message -match ('(?i)\b' + $HexCode + '\b')) { return $true }
        $cause = $cause.InnerException
    }
    return $Record.ToString() -match ('(?i)\b' + $HexCode + '\b')
}

function Test-UacCancelled($Record) {
    $cause = $Record.Exception
    while ($cause) {
        if ($cause -is [System.ComponentModel.Win32Exception] -and $cause.NativeErrorCode -eq 1223) { return $true }
        $cause = $cause.InnerException
    }
    return $false
}

function Invoke-InstallPreparation([string]$Operation, [string]$PackageName = '') {
    # Machine preparation can use administrator authorization. Registration
    # must use the same Windows account; user-data operations stay in the parent.
    $arguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}" -PackagePath "{1}" -LicensePath "{2}" -ErrorPath "{3}"' -f $PSCommandPath, $PackagePath, $LicensePath, $ErrorPath
    if ($InstallLocation) { $arguments += ' -InstallLocation "{0}"' -f $InstallLocation }
    $arguments += ' -StatusPath "{0}" -ExpectedUserSid "{1}"' -f $StatusPath, ([Security.Principal.WindowsIdentity]::GetCurrent().User.Value)
    $failurePath = $ErrorPath + '.helper-error.json'
    Remove-Item -LiteralPath $failurePath -Force -ErrorAction SilentlyContinue
    switch ($Operation) {
        'volume' { $arguments += ' -PrepareVolumeOnly'; $message = '等待 Windows 管理员授权，以准备安装磁盘…' }
        'license' { $arguments += ' -ProvisionOnly'; $message = '等待 Windows 管理员授权，以完成安装授权…' }
        'target' { $arguments += ' -InstallToSelectedVolume'; $message = '正在以管理员权限将 Codex 暂存并注册到所选磁盘…' }
        'register' { $arguments += ' -RegisterAsOriginalUser'; $message = '正在申请 Windows 管理员授权，以注册 Codex 后台服务…' }
        'unprovision' { $arguments += ' -UnprovisionPackage "{0}"' -f $PackageName; $message = '正在清除旧 Codex 的注册和暂存包，以便按所选磁盘重新安装…' }
        default { throw 'Unknown installation preparation operation.' }
    }
    Write-InstallStatus 'authorization' $message
    # Wait for this helper, not for unrelated descendants started by Windows.
    $process = Start-Process -FilePath (Join-Path $PSHOME 'powershell.exe') -Verb RunAs -ArgumentList $arguments -WindowStyle Hidden -PassThru -ErrorAction Stop
    $process.WaitForExit()
    if ($process.ExitCode -eq 1223) { throw (New-Object System.ComponentModel.Win32Exception(1223)) }
    if ($process.ExitCode -ne 0) {
        if (Test-Path -LiteralPath $failurePath) {
            $failure = Get-Content -LiteralPath $failurePath -Raw -Encoding UTF8 | ConvertFrom-Json
            throw $failure.message
        }
        throw "Installation preparation failed (exit code $($process.ExitCode))."
    }
}

function Remove-CodexProvisioning([string]$PackageName) {
    # Remove only Codex's machine provisioning, never an existing user's app
    # registration/data, other applications, or WindowsApps permissions.
    if ($PackageName -notmatch '^OpenAI\.Codex_[0-9.]+_(?:x64|arm64|neutral)_[^_]*_2p2nqsd0c76g0$') {
        throw 'INSTALL_LOCATION_REPAIR_FAILED: Unexpected Codex package identity.'
    }
    $provisioned = @(Get-AppxProvisionedPackage -Online -ErrorAction Stop | Where-Object { $_.PackageName -eq $PackageName })
    foreach ($item in $provisioned) {
        Remove-AppxProvisionedPackage -Online -PackageName $item.PackageName -ErrorAction Stop | Out-Null
    }
}

function Remove-CodexRegistrationForRelocation([string]$PackageName) {
    # Current-user uninstall leaves the staged payload on C:. Windows can then
    # reuse that payload and silently ignore -Volume on the next registration.
    # Remove all registrations/staged files only when no other user has Codex.
    if ($PackageName -notmatch '^OpenAI\.Codex_[0-9.]+_(?:x64|arm64|neutral)_[^_]*_2p2nqsd0c76g0$') {
        throw 'CODEX_MIGRATION_UNSUPPORTED_PACKAGE: Unexpected Codex package identity.'
    }
    if (-not $ExpectedUserSid) { throw 'CODEX_MIGRATION_USER_MISMATCH: Missing original Windows user identity.' }
    $packages = @(Get-AppxPackage -AllUsers -Name 'OpenAI.Codex' -ErrorAction Stop |
        Where-Object { $_.PackageFullName -eq $PackageName })
    $otherUsers = @($packages | ForEach-Object { @($_.PackageUserInformation) } |
        Where-Object { $_.UserSecurityId -and $_.UserSecurityId -ne $ExpectedUserSid -and $_.InstallState -notin @('Uninstalled', 'Staged') })
    if ($otherUsers.Count) {
        throw 'CODEX_MIGRATION_OTHER_USERS: Codex 仍安装在其他 Windows 用户账户中。为保护其他用户，未删除旧版本，请先将 Codex 安装位置改回原磁盘。'
    }
    # -AllUsers is required to remove the stale staged payload, but Windows
    # does not let it be combined with -PreserveApplicationData. Never remove
    # that registration without a verified backup of the current user's data.
    $migration = Read-MigrationState
    if (-not $migration) {
        throw 'CODEX_MIGRATION_BACKUP_FAILED: 未找到 Codex 用户数据备份，已停止卸载重装。'
    }
    Remove-CodexProvisioning $PackageName
    if ($packages.Count) {
        Write-InstallStatus 'removing-old-installation' '正在清除旧 Codex 的当前用户注册和暂存包…'
        Remove-AppxPackage -Package $PackageName -AllUsers -ErrorAction Stop
    }
    if (Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction Stop) {
        throw 'CODEX_MIGRATION_REMOVE_FAILED: Windows 未能清除旧 Codex 的当前用户注册。'
    }
}

function Install-ForCurrentUser {
    if ($InstallLocation) {
        throw 'INSTALL_LOCATION_FAILED: Selected-volume installs must use the staged-manifest registration path.'
    }
    $parameters = @{ Path=$PackagePath; ErrorAction='Stop'; ForceApplicationShutdown=$true }
    Write-InstallStatus 'registering' '正在为当前 Windows 用户安装 Codex…'
    Add-AppxPackage @parameters
    Write-InstallStatus 'verifying' '正在核实 Codex 的用户登记…'
    $package = Get-CodexPackageForCurrentUser
    if (-not $package) { throw 'The package was not registered for the current Windows user.' }
}

function Install-ForCurrentUserSafely {
    try {
        Install-ForCurrentUser
    } catch {
        if (-not (Test-DeploymentCode $_ 0 '0x80073D28')) { throw }
        Save-InstallError 'Packaged service requires administrator authorization' $_
        Invoke-InstallPreparation 'register'
    }
}

function Get-InstallVolume {
    param([switch]$Create)
    $volume = Get-AppxVolume -ErrorAction Stop | Where-Object { $_.PackageStorePath -and $_.PackageStorePath.TrimEnd('\') -ieq $InstallLocation.TrimEnd('\') } | Select-Object -First 1
    if (-not $volume -and $Create) { $volume = Add-AppxVolume -Path $InstallLocation -ErrorAction Stop }
    if (-not $volume -or $volume.IsOffline) { throw "INSTALL_LOCATION_FAILED: Target installation disk is unavailable or needs administrator authorization: $InstallLocation" }
    return $volume
}

$script:MigrationStatePath = Join-Path (Split-Path -Parent $PackagePath) 'codex-migration-pending.json'

function Get-CodexPackageForCurrentUser {
    Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction Stop | Sort-Object Version -Descending | Select-Object -First 1
}

function Find-StagedCodexOnSelectedVolume {
    if (-not $InstallLocation) { throw 'CODEX_STAGE_LOCATION_FAILED: 未设置 Codex 安装磁盘。' }
    $root = [IO.Path]::GetFullPath($InstallLocation).TrimEnd('\')
    if (-not (Test-Path -LiteralPath $root -PathType Container)) {
        throw "CODEX_STAGE_LOCATION_FAILED: Windows 没有创建所选磁盘的应用目录：$root"
    }
    $candidates = @()
    foreach ($directory in @(Get-ChildItem -LiteralPath $root -Directory -Force -ErrorAction Stop |
        Where-Object { $_.Name -like 'OpenAI.Codex_*' })) {
        $manifestPath = Join-Path $directory.FullName 'AppxManifest.xml'
        if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { continue }
        try { [xml]$stagedManifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 } catch { continue }
        $identity = $stagedManifest.Package.Identity
        if (-not $identity) { continue }
        if ([string]$identity.Name -ne $script:PackageIdentityName) { continue }
        if ([string]$identity.Version -ne $script:PackageVersion) { continue }
        if ($script:PackageArchitecture -and [string]$identity.ProcessorArchitecture -ne $script:PackageArchitecture) { continue }
        $candidates += [PSCustomObject]@{
            PackageFullName = $directory.Name
            InstallLocation = $directory.FullName
            ManifestPath = $manifestPath
            LastWriteTime = $directory.LastWriteTimeUtc
        }
    }
    if (-not $candidates.Count) {
        throw "CODEX_STAGE_LOCATION_FAILED: Codex 安装文件没有落到所选磁盘 $root。Windows 可能仍复用了 C 盘的旧暂存包，本次未注册 C 盘版本。"
    }
    return $candidates | Sort-Object LastWriteTime -Descending | Select-Object -First 1
}

function Stage-CodexOnSelectedVolume {
    Write-InstallStatus 'staging' ("正在将 Codex 安装文件暂存到 {0}…" -f $InstallLocation)
    Add-AppxPackage -Stage -Path $PackagePath -Volume (Get-InstallVolume) -ErrorAction Stop
    return Find-StagedCodexOnSelectedVolume
}

function Register-StagedCodexForCurrentUser([object]$StagedPackage) {
    Write-InstallStatus 'registering' ("正在从 {0} 注册 Codex…" -f $StagedPackage.InstallLocation)
    $parameters = @{
        Path = $StagedPackage.ManifestPath
        Register = $true
        ErrorAction = 'Stop'
        ForceApplicationShutdown = $true
        ForceUpdateFromAnyVersion = $true
    }
    try { Add-AppxPackage @parameters }
    catch {
        if (Test-DeploymentCode $_ -2147009287 '0x80073CF9') {
            throw "CODEX_APP_DATA_VOLUME_CONFLICT: Codex 的旧应用数据仍绑定在 C 盘，Windows 拒绝注册到 $InstallLocation。$($_.Exception.Message)"
        }
        throw
    }
    $package = Get-CodexPackageForCurrentUser
    if (-not $package) { throw 'CODEX_STAGE_REGISTRATION_FAILED: Codex 没有登记到当前 Windows 用户。' }
    $expectedRoot = $InstallLocation.TrimEnd('\') + '\'
    if (-not $package.InstallLocation.StartsWith($expectedRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "CODEX_STAGE_REGISTRATION_FAILED: Windows 注册后仍将 Codex 放在 $($package.InstallLocation)，而不是 $InstallLocation。"
    }
    return $package
}

function Install-CodexToSelectedVolume {
    if (-not $InstallLocation) { throw 'INSTALL_LOCATION_FAILED: No installation disk was selected.' }
    # The package is explicitly staged on the selected Appx volume and then
    # registered from that volume's manifest. Do not fall back to registering
    # the MSIX without Volume: that lets Windows silently reuse a C: payload.
    $staged = Stage-CodexOnSelectedVolume
    try {
        Register-StagedCodexForCurrentUser $staged | Out-Null
    } catch {
        if (-not (Test-DeploymentCode $_ -2147009293 '0x80073CF3')) { throw }
        Save-InstallError 'Direct registration requires the Codex offline license' $_
        Write-InstallStatus 'licensing' '正在登记 Codex 的官方离线安装授权…'
        Add-AppxProvisionedPackage -Online -PackagePath $PackagePath -LicensePath $LicensePath -Regions all -ErrorAction Stop | Out-Null
        # Provisioning may stage a second copy on C:. Put the selected-volume
        # copy back in place before registering its manifest explicitly.
        $staged = Stage-CodexOnSelectedVolume
        $package = Register-StagedCodexForCurrentUser $staged
        # Keep the per-user license and registration, but do not leave Codex as
        # a machine-provisioned system package (which blocks later relocation).
        try { Remove-CodexProvisioning $package.PackageFullName }
        catch { Save-InstallError 'Remove temporary Codex machine provisioning' $_ }
    }
    Write-InstallStatus 'verifying' '正在核实 Codex 的安装位置和用户登记…'
    $package = Get-CodexPackageForCurrentUser
    if (-not $package) { throw 'CODEX_STAGE_REGISTRATION_FAILED: Codex 没有登记到当前 Windows 用户。' }
    $expectedRoot = $InstallLocation.TrimEnd('\') + '\'
    if (-not $package.InstallLocation.StartsWith($expectedRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "CODEX_MIGRATION_INSTALL_FAILED: Windows 未能将 Codex 安装到 $InstallLocation；当前所在位置：$($package.InstallLocation)。"
    }
    return $package
}

function Remove-CodexDataBindingAfterBackup {
    $state = Read-MigrationState
    if (-not $state) { return }
    $dataPath = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA ("Packages\{0}" -f $state.packageFamily)))
    $packagesRoot = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'Packages')).TrimEnd('\') + '\'
    if (-not $dataPath.StartsWith($packagesRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'CODEX_MIGRATION_STATE_INVALID: Codex 应用数据路径无效，已停止清理。'
    }
    # A successful package uninstall normally removes this directory. If
    # Windows leaves it behind, its volume identity keeps 0x80073CF9 alive,
    # so remove it only after Read-MigrationState has verified the backup.
    if (Test-Path -LiteralPath $dataPath -PathType Container) {
        Write-InstallStatus 'clearing-old-data-binding' '正在清理旧磁盘的应用数据绑定，随后会恢复全部会话、技能和插件…'
        Remove-Item -LiteralPath $dataPath -Recurse -Force -ErrorAction Stop
    }
    if (Test-Path -LiteralPath $dataPath) {
        throw 'CODEX_MIGRATION_STATE_INVALID: Windows 未能清理 C 盘的旧应用数据绑定。'
    }
}

function Invoke-Robocopy([string]$Source, [string]$Destination) {
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    & (Join-Path $env:SystemRoot 'System32/robocopy.exe') $Source $Destination /E /COPY:DAT /DCOPY:DAT /R:2 /W:1 /XJ /NFL /NDL /NJH /NJS | Out-Null
    $code = $LASTEXITCODE
    if ($code -ge 8) { throw "CODEX_MIGRATION_COPY_FAILED: Robocopy returned $code. Source: $Source; destination: $Destination" }
}

function Get-DataFingerprint([string]$Directory) {
    if (-not (Test-Path -LiteralPath $Directory -PathType Container)) { return @{ files=0; bytes=0L } }
    $files = @(Get-ChildItem -LiteralPath $Directory -File -Force -Recurse -ErrorAction Stop)
    return @{ files=$files.Count; bytes=[long](($files | Measure-Object -Property Length -Sum).Sum) }
}

function Test-DataCopy([string]$Source, [string]$Destination) {
    $sourceRoot = (Get-Item -LiteralPath $Source -Force -ErrorAction Stop).FullName.TrimEnd('\') + '\'
    foreach ($file in @(Get-ChildItem -LiteralPath $Source -File -Force -Recurse -ErrorAction Stop)) {
        $copy = Join-Path $Destination $file.FullName.Substring($sourceRoot.Length)
        if (-not (Test-Path -LiteralPath $copy -PathType Leaf)) { return $false }
        if ((Get-Item -LiteralPath $copy -Force).Length -ne $file.Length) { return $false }
        if ((Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $copy -Algorithm SHA256).Hash) { return $false }
    }
    return $true
}

function Save-MigrationState($State) {
    $json = $State | ConvertTo-Json -Compress
    $temporary = $script:MigrationStatePath + '.tmp'
    [IO.File]::WriteAllText($temporary, $json, (New-Object Text.UTF8Encoding($false)))
    Move-Item -LiteralPath $temporary -Destination $script:MigrationStatePath -Force
    if ($State.backupPath) { Add-Content -LiteralPath $ErrorPath -Value ("CODEX_MIGRATION_BACKUP: {0}" -f $State.backupPath) -Encoding UTF8 }
}

function Read-MigrationState {
    if (-not (Test-Path -LiteralPath $script:MigrationStatePath -PathType Leaf)) { return $null }
    try {
        $state = Get-Content -LiteralPath $script:MigrationStatePath -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($state.userSid -ne [Security.Principal.WindowsIdentity]::GetCurrent().User.Value) { throw 'Migration belongs to another Windows user.' }
        if ($state.packageFamily -ne 'OpenAI.Codex_2p2nqsd0c76g0') { throw 'Unexpected migration package family.' }
        if ($state.hasData) {
            $backupRoot = [IO.Path]::GetFullPath((Join-Path (Split-Path -Parent $PackagePath) 'CodexMoveBackup'))
            $backup = [IO.Path]::GetFullPath([string]$state.backupPath)
            $expectedBackup = [IO.Path]::GetFullPath((Join-Path $backupRoot ([IO.Path]::GetFileName($backup))))
            if ($backup -ine $expectedBackup -or [IO.Path]::GetFileName($backup) -notmatch '^[a-f0-9]{32}$') { throw 'Invalid migration backup path.' }
            if (-not (Test-Path -LiteralPath $backup -PathType Container)) { throw 'Migration backup is missing.' }
            $fingerprint = Get-DataFingerprint $backup
            if ($fingerprint.files -ne $state.files -or $fingerprint.bytes -ne $state.bytes) { throw 'Migration backup is incomplete.' }
        }
        if ($state.backupPath) { Add-Content -LiteralPath $ErrorPath -Value ("CODEX_MIGRATION_BACKUP: {0}" -f $state.backupPath) -Encoding UTF8 }
        return $state
    }
    catch { throw 'CODEX_MIGRATION_STATE_INVALID: Codex 重装恢复信息损坏，备份文件已保留。' }
}

function Restore-MigrationData {
    $state = Read-MigrationState
    if (-not $state) { return }
    $expectedFamily = 'OpenAI.Codex_2p2nqsd0c76g0'
    if ($state.packageFamily -ne $expectedFamily) { throw 'CODEX_MIGRATION_STATE_INVALID: 重装数据与 Codex 包标识不匹配，备份文件已保留。' }
    if ($state.hasData) {
        $backupRoot = [IO.Path]::GetFullPath((Join-Path (Split-Path -Parent $PackagePath) 'CodexMoveBackup'))
        $backup = [IO.Path]::GetFullPath([string]$state.backupPath)
        $expectedBackup = [IO.Path]::GetFullPath((Join-Path $backupRoot ([IO.Path]::GetFileName($backup))))
        if ($backup -ine $expectedBackup -or [IO.Path]::GetFileName($backup) -notmatch '^[a-f0-9]{32}$') {
            throw 'CODEX_MIGRATION_STATE_INVALID: 重装备份路径无效，备份文件已保留。'
        }
        $package = Get-CodexPackageForCurrentUser
        if (-not $package) { throw "CODEX_MIGRATION_RESTORE_FAILED: Codex 尚未注册；数据备份已保留在 $backup" }
        $destination = Join-Path $env:LOCALAPPDATA ("Packages\{0}" -f $state.packageFamily)
        Write-InstallStatus 'restoring-data' '正在恢复 Codex 会话、技能、插件与设置…'
        Invoke-Robocopy $backup $destination
        if (-not (Test-DataCopy $backup $destination)) {
            throw "CODEX_MIGRATION_RESTORE_FAILED: 用户数据未能完整恢复；备份已保留在 $backup"
        }
    }
    Remove-Item -LiteralPath $script:MigrationStatePath -Force -ErrorAction Stop
    # Restoration is committed before optional cleanup; a cleanup failure must
    # not leave a pending state pointing to an already deleted backup.
    if ($state.hasData) { Remove-Item -LiteralPath $backup -Recurse -Force -ErrorAction SilentlyContinue }
}

function Remove-InstalledCodexForRelocation {
    $package = Get-CodexPackageForCurrentUser
    if (-not $InstallLocation -or -not $package) { return }
    $expectedRoot = $InstallLocation.TrimEnd('\') + '\'
    if ($package.InstallLocation.StartsWith($expectedRoot, [StringComparison]::OrdinalIgnoreCase)) { return }

    $state = Read-MigrationState
    $family = [string]$package.PackageFamilyName
    if ($family -ne 'OpenAI.Codex_2p2nqsd0c76g0') { throw 'CODEX_MIGRATION_UNSUPPORTED_PACKAGE: 当前 Codex 包标识与预期不符，未卸载现有应用。' }
    $oldRoot = ([IO.Path]::GetFullPath($package.InstallLocation)).TrimEnd('\') + '\'
    Get-Process -ErrorAction SilentlyContinue | ForEach-Object {
        $processPath = $null
        try { $processPath = $_.Path } catch { }
        if ($processPath -and $_.SessionId -eq (Get-Process -Id $PID).SessionId -and $processPath.StartsWith($oldRoot, [StringComparison]::OrdinalIgnoreCase)) {
            Stop-Process -Id $_.Id -Force -ErrorAction Stop
        }
    }
    if (-not $state) {
        $dataPath = Join-Path $env:LOCALAPPDATA ("Packages\{0}" -f $family)
        $fingerprint = Get-DataFingerprint $dataPath
        $backup = $null
        $hasData = $fingerprint.files -gt 0
        if ($hasData) {
            $backup = Join-Path (Join-Path (Split-Path -Parent $PackagePath) 'CodexMoveBackup') ([guid]::NewGuid().ToString('N'))
            Write-InstallStatus 'backing-up-data' '正在备份 Codex 会话、技能、插件与设置，完成后将重新安装到所选磁盘…'
            Invoke-Robocopy $dataPath $backup
            $copied = Get-DataFingerprint $backup
            if ($copied.files -ne $fingerprint.files -or $copied.bytes -ne $fingerprint.bytes -or -not (Test-DataCopy $dataPath $backup)) {
                throw "CODEX_MIGRATION_BACKUP_FAILED: Codex 用户数据备份校验失败，原应用未卸载；备份目录：$backup"
            }
        }
        $state = @{ userSid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value; packageFamily=$family; sourceLocation=$package.InstallLocation; hasData=$hasData; backupPath=$backup; files=$fingerprint.files; bytes=$fingerprint.bytes }
        Save-MigrationState $state
    } elseif ($state.packageFamily -ne $family) {
        throw 'CODEX_MIGRATION_STATE_INVALID: 已有待恢复数据属于其他包，未卸载现有应用。'
    }

    Write-InstallStatus 'removing-old-installation' '正在关闭并移除当前用户的旧 Codex 安装…'
    Invoke-InstallPreparation 'unprovision' $package.PackageFullName
    if (Get-CodexPackageForCurrentUser) { throw 'CODEX_MIGRATION_REMOVE_FAILED: Windows 未能移除当前用户的旧 Codex；数据备份已保留。' }
}

# Read the package's own requirement so future Codex releases can raise it.
# Reject an unsupported OS before registration or an unnecessary UAC prompt.
try {
    Write-InstallStatus 'checking' '正在检查安装包与 Windows 版本兼容性…'
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($PackagePath)
    try {
        $entry = $archive.GetEntry('AppxManifest.xml')
        if (-not $entry) { throw 'AppxManifest.xml was not found in the installation package.' }
        $reader = New-Object System.IO.StreamReader($entry.Open())
        try { [xml]$manifest = $reader.ReadToEnd() } finally { $reader.Dispose() }
    } finally { $archive.Dispose() }
    $packageIdentity = $manifest.Package.Identity
    if (-not $packageIdentity) { throw 'AppxManifest.xml does not contain a package identity.' }
    $script:PackageIdentityName = [string]$packageIdentity.Name
    $script:PackageVersion = [string]$packageIdentity.Version
    $script:PackageArchitecture = [string]$packageIdentity.ProcessorArchitecture
    $requirements = @($manifest.SelectNodes("//*[local-name()='TargetDeviceFamily']") | Where-Object { $_.Name -in @('Windows.Desktop', 'Windows.Universal') })
    $minimum = $requirements | ForEach-Object { [version]$_.MinVersion } | Sort-Object | Select-Object -First 1
    if ($minimum) {
        $os = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
        $majorMinor = if ($null -ne $os.CurrentMajorVersionNumber) { '{0}.{1}' -f $os.CurrentMajorVersionNumber, $os.CurrentMinorVersionNumber } else { $os.CurrentVersion }
        $revision = if ($null -ne $os.UBR) { $os.UBR } else { 0 }
        $current = [version]('{0}.{1}.{2}' -f $majorMinor, $os.CurrentBuildNumber, $revision)
        if ($current -lt $minimum) {
            "UNSUPPORTED_WINDOWS: 当前系统版本 $current，安装包要求 Windows $minimum 或更高版本。请升级 Windows 后重试；安装包已保留。" | Add-Content -LiteralPath $ErrorPath -Encoding UTF8
            exit 1603
        }
    }
} catch {
    Save-InstallError 'Check package compatibility' $_
    exit 1
}

if ($InstallToSelectedVolume) {
    try {
        if (-not $InstallLocation) { throw 'INSTALL_LOCATION_FAILED: No installation disk was selected.' }
        if (-not $ExpectedUserSid -or [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne $ExpectedUserSid) {
            throw 'INSTALL_LOCATION_USER_MISMATCH: 管理员授权必须使用当前 Windows 用户对应的管理员账户，才能注册 Codex。'
        }
        Install-CodexToSelectedVolume | Out-Null
        Write-InstallStatus 'complete' 'Codex 已从所选磁盘注册并核实安装位置。'
        exit 0
    } catch {
        Save-InstallError 'Install Codex from selected volume' $_
        $failure = @{ message=$_.Exception.Message } | ConvertTo-Json -Compress
        [IO.File]::WriteAllText(($ErrorPath + '.helper-error.json'), $failure, (New-Object Text.UTF8Encoding($false)))
        exit 1
    }
}

if ($RegisterAsOriginalUser) {
    try {
        if (-not $ExpectedUserSid -or [Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne $ExpectedUserSid) {
            throw 'INSTALL_LOCATION_USER_MISMATCH: 管理员授权必须使用当前 Windows 用户对应的管理员账户，才能注册 Codex。'
        }
        Install-ForCurrentUser
        Write-InstallStatus 'complete' 'Codex 安装位置已核实，后台服务和当前用户登记均已完成。'
        exit 0
    } catch {
        Save-InstallError 'Elevated registration for current user' $_
        $failure = @{ message=$_.Exception.Message } | ConvertTo-Json -Compress
        [IO.File]::WriteAllText(($ErrorPath + '.helper-error.json'), $failure, (New-Object Text.UTF8Encoding($false)))
        exit 1
    }
}

if ($ProvisionOnly -or $PrepareVolumeOnly -or $UnprovisionPackage) {
    try {
        if ($UnprovisionPackage) {
            Write-InstallStatus 'removing-provisioning' '正在清除旧 Codex 的注册和暂存包…'
            Remove-CodexRegistrationForRelocation $UnprovisionPackage
        } elseif ($PrepareVolumeOnly) {
            if (-not $InstallLocation) { throw 'INSTALL_LOCATION_FAILED: No installation disk was selected.' }
            Write-InstallStatus 'preparing-volume' ("正在准备安装磁盘 {0}…" -f $InstallLocation)
            Get-InstallVolume -Create | Out-Null
        } else {
            if ($InstallLocation) {
                Write-InstallStatus 'staging' ("正在将安装文件部署到 {0}…" -f $InstallLocation)
                Add-AppxPackage -Stage -Path $PackagePath -Volume (Get-InstallVolume) -ErrorAction Stop
            }
            Write-InstallStatus 'licensing' '正在登记 Codex 的官方离线安装授权…'
            Add-AppxProvisionedPackage -Online -PackagePath $PackagePath -LicensePath $LicensePath -Regions all -ErrorAction Stop | Out-Null
        }
        exit 0
    } catch {
        Save-InstallError 'Provision package' $_
        $failure = @{ message=$_.Exception.Message } | ConvertTo-Json -Compress
        [IO.File]::WriteAllText(($ErrorPath + '.helper-error.json'), $failure, (New-Object Text.UTF8Encoding($false)))
        exit 1
    }
}

if ($InstallLocation) {
    try {
        # A missing Appx volume is not a missing app license. Do not provision
        # Codex as a system app just to prepare a newly selected disk.
        try { Get-InstallVolume | Out-Null }
        catch { Invoke-InstallPreparation 'volume'; Get-InstallVolume | Out-Null }
        try { Remove-InstalledCodexForRelocation }
        catch {
            if (Test-UacCancelled $_) { throw }
            throw ('CODEX_MIGRATION_REMOVE_FAILED: ' + $_.Exception.Message)
        }
        Remove-CodexDataBindingAfterBackup
        Invoke-InstallPreparation 'target'
        Restore-MigrationData
        Write-InstallStatus 'complete' 'Codex 安装位置已核实，安装完成。'
        exit 0
    } catch {
        Save-InstallError 'Install Codex to selected disk' $_
        if (Test-UacCancelled $_) {
            Save-InstallError 'UAC_CANCELLED' $_
            exit 1223
        }
        exit 1
    }
}

$preparingVolume = $false
try {
    Install-ForCurrentUserSafely
    Restore-MigrationData
    Write-InstallStatus 'complete' 'Codex 安装位置已核实，安装完成。'
    exit 0
} catch {
    Save-InstallError 'Install for current user' $_
    if (Test-UacCancelled $_) {
        Save-InstallError 'UAC_CANCELLED' $_
        exit 1223
    }
}

try {
    # Provisioning and packaged-service registration can require elevation.
    Invoke-InstallPreparation 'license'
    try { Remove-InstalledCodexForRelocation }
    catch {
        if (Test-UacCancelled $_) { throw }
        throw ('CODEX_MIGRATION_REMOVE_FAILED: ' + $_.Exception.Message)
    }
    Install-ForCurrentUserSafely
    Restore-MigrationData
    Write-InstallStatus 'complete' 'Codex 安装位置已核实，安装完成。'
    exit 0
} catch {
    if (Test-UacCancelled $_) {
        Save-InstallError 'UAC_CANCELLED' $_
        exit 1223
    }
    Save-InstallError 'Complete installation' $_
    exit 1
}
