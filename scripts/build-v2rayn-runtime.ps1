param(
    [ValidateSet("win-x64", "osx-x64", "osx-arm64")]
    [string]$RuntimeIdentifier = "win-x64"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$commit = "57f17d5939e88c8e643ab49b90869d637765e75a"
$coreCommit = "dd13a61184af1981545fe5f34a1283286959e51d"
$nugetSource = if ($env:NUGET_SOURCE) { $env:NUGET_SOURCE } else { "https://api.nuget.org/v3/index.json" }
$sourceName = "v2rayN-$commit"
$sourceRoot = Join-Path $root "src-tauri\bundled\v2rayn\source\$sourceName"
$release = Join-Path $root "src-tauri\bundled\v2rayn\$RuntimeIdentifier"
$tempRoot = [IO.Path]::GetTempPath()
$work = Join-Path $tempRoot "codexinstaller-v2rayn-$RuntimeIdentifier-$([guid]::NewGuid().ToString('N'))"
$sourceZip = Join-Path $work "v2rayN-source.zip"
$coreZip = Join-Path $work "v2rayN-core.zip"
$unpack = Join-Path $work "source"
$publish = Join-Path $work "publish"
$coreUnpack = Join-Path $work "core"
$stage = Join-Path $work "package"
$repoRoot = $sourceRoot

if (Test-Path -LiteralPath $release) {
    $runtimeExecutable = if ($RuntimeIdentifier.StartsWith("win-")) { "v2rayN.WebAPI.exe" } else { "v2rayN.WebAPI" }
    if ((Test-Path -LiteralPath (Join-Path $release "api\$runtimeExecutable")) -and (Test-Path -LiteralPath (Join-Path $release "bin"))) {
        Write-Output "v2rayN runtime already exists for $RuntimeIdentifier at $release"
        exit 0
    }
    throw "Runtime output exists but is incomplete; preserve or move it aside before rebuilding: $release"
}
New-Item -ItemType Directory -Path $work,$unpack,$publish,$coreUnpack,(Join-Path $stage "api"),(Join-Path $stage "source") -Force | Out-Null

function Download-VerifiedTransportFile([string]$Url, [string]$OutputPath) {
    $curl = if ($IsWindows) { "curl.exe" } else { "curl" }
    $candidateUrls = @($Url)
    if ($Url -match "^https://raw\.githubusercontent\.com/(.+)$") {
        $candidateUrls += "https://gh-proxy.com/$Url"
        $candidateUrls += "https://ghfast.top/$Url"
        $candidateUrls += "https://ghproxy.net/$Url"
    }
    $downloaded = $false
    foreach ($candidateUrl in $candidateUrls) {
        & $curl --fail --location --retry 2 --connect-timeout 20 --max-time 7200 --silent --show-error --continue-at - --output $OutputPath $candidateUrl
        if ($LASTEXITCODE -eq 0 -and (Test-Path -LiteralPath $OutputPath -PathType Leaf)) {
            $downloaded = $true
            break
        }
    }
    if (-not $downloaded) {
        throw "Download failed: $Url"
    }
    if ((Get-Item -LiteralPath $OutputPath).Length -lt 1MB) {
        throw "Download was unexpectedly small: $Url"
    }
}

if (-not (Test-Path -LiteralPath $sourceRoot -PathType Container)) {
    throw "Pinned v2rayN source is missing: $sourceRoot"
}
$sourceProject = Join-Path $sourceRoot "v2rayN\v2rayN.WebAPI\v2rayN.WebAPI.csproj"
$sourceLicense = Join-Path $sourceRoot "LICENSE"
$sourceVersion = Join-Path $sourceRoot "v2rayN\Directory.Build.props"
if (-not (Test-Path -LiteralPath $sourceProject -PathType Leaf) -or
    -not (Test-Path -LiteralPath $sourceLicense -PathType Leaf) -or
    -not (Select-String -LiteralPath $sourceVersion -Pattern "<Version>7\.25\.6</Version>" -Quiet)) {
    throw "Pinned v2rayN source layout, license, or version check failed."
}
$project = Join-Path $repoRoot "v2rayN\v2rayN.WebAPI\v2rayN.WebAPI.csproj"
if (-not (Test-Path -LiteralPath $project -PathType Leaf)) {
    throw "Pinned v2rayN source archive did not contain the WebAPI project."
}

dotnet restore $project --runtime $RuntimeIdentifier --source $nugetSource --disable-parallel --verbosity minimal
if ($LASTEXITCODE -ne 0) {
    throw "v2rayN WebAPI dependency restore failed for $RuntimeIdentifier"
}
dotnet publish $project --no-restore --configuration Release --runtime $RuntimeIdentifier --self-contained true `
    -p:PublishSingleFile=true -p:IncludeNativeLibrariesForSelfExtract=true --output $publish
if ($LASTEXITCODE -ne 0) {
    throw "v2rayN WebAPI publish failed for $RuntimeIdentifier"
}

$coreBundle = switch ($RuntimeIdentifier) {
    "win-x64" { "v2rayN-windows-64" }
    "osx-x64" { "v2rayN-macos-64" }
    "osx-arm64" { "v2rayN-macos-arm64" }
}
$coreUrl = "https://raw.githubusercontent.com/2dust/v2rayN-core-bin/$coreCommit/$coreBundle.zip"
Download-VerifiedTransportFile $coreUrl $coreZip
$expectedCoreBlob = switch ($RuntimeIdentifier) {
    "win-x64" { "05613bf125b799096081696c89a7f706bcf1994e" }
    "osx-x64" { "57d4809850c562c8bb46930f88ca13dc79cfbfa9" }
    "osx-arm64" { "ad64687744d5f3400aed4923f1c88315a5166c86" }
}
$coreLength = (Get-Item -LiteralPath $coreZip).Length
$blobHash = [Security.Cryptography.IncrementalHash]::CreateHash([Security.Cryptography.HashAlgorithmName]::SHA1)
$header = [Text.Encoding]::ASCII.GetBytes("blob $coreLength`0")
$blobHash.AppendData($header)
$coreStream = [IO.File]::OpenRead($coreZip)
try {
    $buffer = [byte[]]::new(1MB)
    while (($read = $coreStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
        $blobHash.AppendData($buffer, 0, $read)
    }
} finally {
    $coreStream.Dispose()
}
$actualCoreBlob = [BitConverter]::ToString($blobHash.GetHashAndReset()).Replace("-", "").ToLowerInvariant()
$blobHash.Dispose()
if ($actualCoreBlob -ne $expectedCoreBlob) {
    throw "Pinned v2rayN Core archive checksum mismatch."
}
Expand-Archive -LiteralPath $coreZip -DestinationPath $coreUnpack
$coreBin = Join-Path $coreUnpack "$coreBundle\bin"
if (-not (Test-Path -LiteralPath (Join-Path $coreBin "xray") -PathType Container)) {
    throw "Official v2rayN Core bundle is missing its xray payload."
}

Copy-Item -Path (Join-Path $publish "*") -Destination (Join-Path $stage "api") -Recurse -Force
Copy-Item -LiteralPath $coreBin -Destination (Join-Path $stage "bin") -Recurse -Force
Copy-Item -LiteralPath $sourceLicense -Destination (Join-Path $stage "LICENSE-v2rayN") -Force
Copy-Item -LiteralPath $sourceLicense -Destination (Join-Path $stage "source\LICENSE") -Force
$sourceCopy = Join-Path $stage "source\v2rayN-$commit"
if ($IsWindows -or $env:OS -eq "Windows_NT") {
    & robocopy.exe $repoRoot $sourceCopy /E /R:1 /W:1 /NFL /NDL /NJH /NJS /NP /XD bin obj .NET .packages TestResults publish
    if ($LASTEXITCODE -ge 8) {
        throw "Failed to stage the corresponding v2rayN source archive (robocopy exit $LASTEXITCODE)."
    }
} else {
    New-Item -ItemType Directory -Path $sourceCopy -Force | Out-Null
    & rsync -a --exclude=bin --exclude=obj --exclude=.NET --exclude=.packages --exclude=TestResults --exclude=publish "$repoRoot/" "$sourceCopy/"
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to stage the corresponding v2rayN source archive."
    }
}
$licenseCurl = if ($env:OS -eq "Windows_NT") { "curl.exe" } else { "curl" }
& $licenseCurl --fail --location --retry 3 --connect-timeout 20 --max-time 120 --silent --show-error `
    --output (Join-Path $stage "LICENSE-v2rayN-core-bin") `
    "https://raw.githubusercontent.com/2dust/v2rayN-core-bin/$coreCommit/LICENSE"
if ($LASTEXITCODE -ne 0) {
    throw "Failed to download the pinned v2rayN Core license."
}
if ($RuntimeIdentifier.StartsWith("osx-") -and $IsMacOS) {
    & chmod +x (Join-Path $stage "api\v2rayN.WebAPI")
    & chmod +x (Join-Path $stage "bin\xray\xray")
    & chmod +x (Join-Path $stage "bin\sing_box\sing-box")
    & chmod +x (Join-Path $stage "bin\mihomo\mihomo")
}
New-Item -ItemType Directory -Path (Split-Path -Parent $release) -Force | Out-Null
Move-Item -LiteralPath $stage -Destination $release

Write-Output "v2rayN WebAPI and Core packaged for $RuntimeIdentifier at $release"
$resolvedTemp = [IO.Path]::GetFullPath($tempRoot).TrimEnd([IO.Path]::DirectorySeparatorChar) + [IO.Path]::DirectorySeparatorChar
$resolvedWork = [IO.Path]::GetFullPath($work)
if (-not $resolvedWork.StartsWith($resolvedTemp, [StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to clean a build directory outside TEMP: $resolvedWork"
}
Remove-Item -LiteralPath $resolvedWork -Recurse -Force
