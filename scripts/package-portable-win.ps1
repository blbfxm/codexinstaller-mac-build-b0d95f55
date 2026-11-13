param(
    [switch]$CreateZip,
    [switch]$SkipMsi,
    [string]$PackageSuffix = ""
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent $PSScriptRoot
$appName = 'Codex' + [char]0x4F7F + [char]0x7528 + [char]0x52A9 + [char]0x624B
$displayName = 'Codex' + [char]0x0020 + [char]0x4F7F + [char]0x7528 + [char]0x52A9 + [char]0x624B
$exeName = "$appName.exe"
$exe = Join-Path $root "src-tauri\target\release\$exeName"
$runtime = Join-Path $root "src-tauri\bundled\v2rayn\win-x64"
$output = Join-Path $root "dist-portable"
$packageName = "codexinstaller-windows-x64"
if (-not [string]::IsNullOrWhiteSpace($PackageSuffix)) {
    $packageName = "$packageName-$PackageSuffix"
}
$portable = Join-Path $output $packageName
$portableExe = Join-Path $portable $exeName
$portableLegacyExe = Join-Path $portable "codexinstaller.exe"
$msiDirectory = Join-Path $root "src-tauri\target\release\bundle\msi"
$msiFileName = "$appName.msi"
if (-not [string]::IsNullOrWhiteSpace($PackageSuffix)) {
    $msiFileName = "$appName-$PackageSuffix.msi"
}
$msiOutput = Join-Path $output $msiFileName
$archive = Join-Path $output "$packageName.zip"
$resourceBundle = Join-Path $portable "resources\bundled"

if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw "Release executable was not found: $exe"
}
$msi = $null
if (-not $SkipMsi) {
    $msi = Get-ChildItem -LiteralPath $msiDirectory -Filter "$displayName*.msi" -File -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
}
if (-not $msi -and -not $SkipMsi) {
    throw "The MSI installer was not found: $msiDirectory"
}
$api = Join-Path $runtime "api\v2rayN.WebAPI.exe"
$core = Join-Path $runtime "bin\xray\xray.exe"
if (-not (Test-Path -LiteralPath $api -PathType Leaf) -or -not (Test-Path -LiteralPath $core -PathType Leaf)) {
    throw "The bundled v2rayN WebAPI and matching Core are required before creating a portable package. Run npm run v2rayn:runtime:win."
}

New-Item -ItemType Directory -Path $portable -Force | Out-Null
try {
    Copy-Item -LiteralPath $exe -Destination $portableExe -Force
} catch {
    # A tray instance can keep the old image mapped. Preserve it without killing the user's proxy.
    $portableRootFull = [System.IO.Path]::GetFullPath($portable).TrimEnd('\') + '\'
    $portableExeFull = [System.IO.Path]::GetFullPath($portableExe)
    if (-not $portableExeFull.StartsWith($portableRootFull, [System.StringComparison]::OrdinalIgnoreCase) -or
        -not (Test-Path -LiteralPath $portableExe -PathType Leaf)) { throw }
    $runningBackup = Join-Path $portable ($appName + '.running-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff') + '.exe')
    $runningBackupFull = [System.IO.Path]::GetFullPath($runningBackup)
    if (-not $runningBackupFull.StartsWith($portableRootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'The running executable backup must stay inside the portable folder.'
    }
    Move-Item -LiteralPath $portableExe -Destination $runningBackup -ErrorAction Stop
    try { Copy-Item -LiteralPath $exe -Destination $portableExe -Force }
    catch {
        if (-not (Test-Path -LiteralPath $portableExe)) { Move-Item -LiteralPath $runningBackup -Destination $portableExe }
        throw
    }
    Write-Output "Running executable preserved: $runningBackup"
    Write-Output 'Exit the previous tray instance before opening the updated executable.'
}
if (Test-Path -LiteralPath $portableLegacyExe -PathType Leaf) {
    Remove-Item -LiteralPath $portableLegacyExe -Force
}
if ($msi) {
    Copy-Item -LiteralPath $msi.FullName -Destination $msiOutput -Force
    & powershell -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "fix-msi-install-directory.ps1") -MsiPath $msiOutput
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to remove the previous installation directory from the MSI."
    }
}
New-Item -ItemType Directory -Path $resourceBundle -Force | Out-Null
$bundled = Join-Path $root "src-tauri\bundled"
foreach ($asset in @("codex", "codexpp")) {
    $assetPath = Join-Path $bundled $asset
    if (Test-Path -LiteralPath $assetPath -PathType Container) {
        Copy-Item -LiteralPath $assetPath -Destination $resourceBundle -Recurse -Force
    }
}
$proxyRuntime = Join-Path $bundled "v2rayn\win-x64"
$proxyDestination = Join-Path $resourceBundle "v2rayn\win-x64"
Copy-Item -LiteralPath $proxyRuntime -Destination $proxyDestination -Recurse -Force

if ($CreateZip) {
    if (Test-Path -LiteralPath $archive) {
        Remove-Item -LiteralPath $archive -Force
    }
    Compress-Archive -Path (Join-Path $portable "*") -DestinationPath $archive -CompressionLevel Optimal
    Write-Output "Portable package: $archive"
} else {
    Write-Output "Portable folder updated: $portable"
    if ($msi) {
        Write-Output "MSI installer updated: $msiOutput"
    } else {
        Write-Output "MSI installer not built; WiX Toolset is required."
    }
}
