# Read-only discovery. Never changes package registration or WindowsApps permissions.
function Get-CodexDesktopAtRoot {
    param([string]$Root, [string]$Version = '')
    if (-not $Root) { return }
    $manifestPath = Join-Path $Root 'AppxManifest.xml'
    $manifestExecutable = $null
    if (Test-Path -LiteralPath $manifestPath -PathType Leaf) {
        try {
            $manifest = [xml](Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 -ErrorAction Stop)
            if ($manifest.Package.Identity.Name -notlike 'OpenAI.Codex*') { return }
            if (-not $Version) { $Version = [string]$manifest.Package.Identity.Version }
            $application = @($manifest.Package.Applications.Application) | Where-Object {
                $_.Executable -match '(^|[\\/])(ChatGPT|Codex)\.exe$' -and $_.Executable -notmatch '[\\/]resources[\\/]'
            } | Select-Object -First 1
            if ($application) {
                $resolvedRoot = [IO.Path]::GetFullPath($Root).TrimEnd('\') + '\'
                $candidate = [IO.Path]::GetFullPath((Join-Path $Root $application.Executable))
                if ($candidate.StartsWith($resolvedRoot, [StringComparison]::OrdinalIgnoreCase)) { $manifestExecutable = $candidate }
            }
        } catch { return }
    }
    # Never treat the bundled CLI (app/resources/codex.exe) as the desktop app.
    foreach ($executable in @($manifestExecutable, (Join-Path $Root 'app\ChatGPT.exe'), (Join-Path $Root 'app\Codex.exe'), (Join-Path $Root 'ChatGPT.exe'), (Join-Path $Root 'Codex.exe'))) {
        if ($executable -and (Test-Path -LiteralPath $executable -PathType Leaf)) {
            if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
                # Ordinary/portable installs need Codex desktop resources. The
                # standalone ChatGPT app and a CLI named codex.exe do not count.
                $resources = Join-Path (Split-Path -Parent $executable) 'resources'
                if (-not (Test-Path -LiteralPath (Join-Path $resources 'codex.exe') -PathType Leaf)) { continue }
            }
            return [PSCustomObject]@{ root = $Root; executable = $executable; version = $Version }
        }
    }
}

function Get-CodexExternalRoots {
    # Running apps, registration and shortcuts locate arbitrary custom folders
    # without recursively searching the user's disks or changing any ACLs.
    foreach ($process in @(Get-Process -Name 'ChatGPT','Codex' -ErrorAction SilentlyContinue)) {
        try {
            if ($process.Path) {
                $folder = Split-Path -Parent $process.Path
                $folder
                Split-Path -Parent $folder
            }
        } catch {}
    }
    foreach ($hive in @('HKCU:\Software', 'HKLM:\SOFTWARE', 'HKLM:\SOFTWARE\WOW6432Node')) {
        foreach ($name in @('ChatGPT.exe', 'Codex.exe')) {
            try {
                $key = Get-ItemProperty -LiteralPath "$hive\Microsoft\Windows\CurrentVersion\App Paths\$name" -ErrorAction Stop
                if ($key.'(default)') { Split-Path -Parent ([string]$key.'(default)').Trim('"') }
                if ($key.Path) { [string]$key.Path }
            } catch {}
        }
        foreach ($key in @(Get-ChildItem -LiteralPath "$hive\Microsoft\Windows\CurrentVersion\Uninstall" -ErrorAction SilentlyContinue)) {
            $item = Get-ItemProperty -LiteralPath $key.PSPath -ErrorAction SilentlyContinue
            if ($item.DisplayName -notmatch '(?i)codex|chatgpt') { continue }
            if ($item.InstallLocation) { [string]$item.InstallLocation }
            if ($item.DisplayIcon) {
                $icon = ([string]$item.DisplayIcon -replace ',\s*-?\d+$','').Trim('"')
                if ($icon -match '(?i)\.exe$') { Split-Path -Parent $icon }
            }
        }
    }
    foreach ($base in @($env:LOCALAPPDATA, $env:ProgramFiles, ${env:ProgramFiles(x86)})) {
        if (-not $base) { continue }
        foreach ($relative in @('Codex', 'Programs\Codex', 'OpenAI\Codex')) { Join-Path $base $relative }
    }
    foreach ($drive in @(Get-PSDrive -PSProvider FileSystem -ErrorAction SilentlyContinue)) {
        if ($drive.DisplayRoot -like '\\*' -or $drive.Root -like '\\*') { continue }
        foreach ($relative in @('Codex','Programs\Codex','Program Files\Codex','Program Files\OpenAI\Codex')) {
            Join-Path $drive.Root $relative
        }
    }
    $shell = $null
    try {
        $shell = New-Object -ComObject WScript.Shell
        foreach ($folder in @([Environment]::GetFolderPath('StartMenu'), [Environment]::GetFolderPath('CommonStartMenu'), [Environment]::GetFolderPath('Desktop'), [Environment]::GetFolderPath('CommonDesktopDirectory'))) {
            if (-not $folder) { continue }
            foreach ($link in @(Get-ChildItem -LiteralPath $folder -Filter '*.lnk' -Recurse -File -ErrorAction SilentlyContinue | Where-Object BaseName -Match '(?i)codex|chatgpt')) {
                $shortcut = $shell.CreateShortcut($link.FullName)
                $target = $shortcut.TargetPath
                if ($target -match '(?i)[\\/](Codex|ChatGPT)\.exe$') {
                    $directory = Split-Path -Parent $target
                    $directory
                    Split-Path -Parent $directory
                }
                [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shortcut)
            }
        }
    } catch {} finally { if ($shell) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell) } }
}

function Get-CodexFallbackRoots {
    # Package repository lookup also works when Appx module discovery fails.
    foreach ($repository in @(
        'HKCU:\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages',
        'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Appx\PackageRepository\Packages'
    )) {
        foreach ($key in @(Get-ChildItem -LiteralPath $repository -ErrorAction SilentlyContinue | Where-Object PSChildName -Like 'OpenAI.Codex_*')) {
            $properties = Get-ItemProperty -LiteralPath $key.PSPath -ErrorAction SilentlyContinue
            if ($properties.PackageRootFolder) { [string]$properties.PackageRootFolder }
        }
    }
    $stores = @()
    try { $stores += @(Get-AppxVolume -ErrorAction Stop | ForEach-Object { $_.PackageStorePath }) } catch {}
    foreach ($drive in @(Get-PSDrive -PSProvider FileSystem -ErrorAction SilentlyContinue)) {
        # Skip network drives; inspect only the top-level Store directories.
        if ($drive.DisplayRoot -like '\\*' -or $drive.Root -like '\\*') { continue }
        $stores += Join-Path $drive.Root 'WindowsApps'
        $stores += Join-Path $drive.Root 'Program Files\WindowsApps'
    }
    foreach ($store in @($stores | Where-Object { $_ } | Select-Object -Unique)) {
        Get-ChildItem -LiteralPath $store -Filter 'OpenAI.Codex_*' -Directory -ErrorAction SilentlyContinue | ForEach-Object { $_.FullName }
    }
}

function Find-CodexInstallation {
    # Check every registered version: a stale newest entry must not hide a working package.
    try {
        $packages = @(Get-AppxPackage -Name 'OpenAI.Codex*' -ErrorAction Stop | Sort-Object { [version]$_.Version } -Descending)
        foreach ($package in $packages) {
            $desktop = Get-CodexDesktopAtRoot -Root $package.InstallLocation -Version ([string]$package.Version)
            if ($desktop) { return $desktop }
        }
    } catch {}
    $roots = @(Get-CodexFallbackRoots | Select-Object -Unique | Sort-Object {
        if ($_ -match 'OpenAI\.Codex_([0-9.]+)_') { try { [version]$Matches[1] } catch { [version]'0.0' } }
        else { [version]'0.0' }
    } -Descending)
    foreach ($root in $roots) {
        # Unregistered disk/registry remnants alone are not proof of a desktop installation.
        if (-not (Test-Path -LiteralPath (Join-Path $root 'AppxManifest.xml') -PathType Leaf)) { continue }
        $desktop = Get-CodexDesktopAtRoot -Root $root
        if ($desktop) { return $desktop }
    }
    foreach ($root in @(Get-CodexExternalRoots | Where-Object { $_ } | Select-Object -Unique)) {
        $desktop = Get-CodexDesktopAtRoot -Root $root
        if ($desktop) { return $desktop }
    }
}
