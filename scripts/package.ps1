<#
.SYNOPSIS
    Native Windows packaging script for presence-switch.

.DESCRIPTION
    Windows-native counterpart to scripts/package.sh. Builds the release binary
    with the MSVC toolchain (no mingw cross-compile) and links the MSI with the
    WiX v3 toolset (candle + light), which consumes wix/main.wxs unchanged --
    the same WiX 2006 schema that wixl reads on Linux.

    WiX v3 is located in this order:
      1. $env:WIX            (set by the official WiX 3 installer)
      2. candle.exe on PATH
      3. A per-user cache at %LOCALAPPDATA%\presence-switch\wix3, auto-downloaded
         from the WiX GitHub release if not already present.

.PARAMETER Target
    msi (default) -- cross-platform RPM packaging lives in package.sh; on Windows
    only the MSI target is meaningful.

.EXAMPLE
    pwsh scripts/package.ps1
    pwsh scripts/package.ps1 msi
#>
[CmdletBinding()]
param(
    [ValidateSet('msi')]
    [string]$Target = 'msi'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# Pin the WiX v3 release used when auto-downloading. 3.14.1 is the last v3.
$WixVersion = '3.14.1'
$WixZipUrl  = 'https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip'

function Write-Step([string]$Message) { Write-Host "==> $Message" -ForegroundColor Blue }
function Write-Fail([string]$Message) { Write-Error $Message }

# Repo root is the parent of this script's directory.
$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root

$Name = 'presence-switch'

# Parse version from Cargo.toml ([package] version, first match).
$Version = (Select-String -Path (Join-Path $Root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $Version) { Write-Fail "Could not parse version from Cargo.toml" }

# Short SHA with dirty marker, mirroring package.sh.
$ShortSha = (& git rev-parse --short=7 HEAD 2>$null)
if ($LASTEXITCODE -ne 0 -or -not $ShortSha) {
    $ShortSha = 'nogit'
} else {
    & git diff-index --quiet HEAD -- 2>$null
    if ($LASTEXITCODE -ne 0) { $ShortSha = "$ShortSha.dirty" }
}

function Get-WixBinDir {
    # 1. Official installer sets $env:WIX (with a trailing slash) pointing at the
    #    install root; binaries live under bin\.
    if ($env:WIX) {
        $bin = Join-Path $env:WIX 'bin'
        if (Test-Path (Join-Path $bin 'candle.exe')) { return $bin }
    }

    # 2. candle.exe already on PATH.
    $candle = Get-Command candle.exe -ErrorAction SilentlyContinue
    if ($candle) { return (Split-Path -Parent $candle.Source) }

    # 3. Per-user cache; download on first use.
    $cache = Join-Path $env:LOCALAPPDATA "presence-switch\wix3\$WixVersion"
    if (Test-Path (Join-Path $cache 'candle.exe')) { return $cache }

    Write-Step "WiX v3 not found; downloading $WixVersion to $cache"
    New-Item -ItemType Directory -Force -Path $cache | Out-Null
    $zip = Join-Path $cache 'wix-binaries.zip'
    # TLS 1.2 for older runners; modern Windows defaults are fine but explicit is safe.
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $WixZipUrl -OutFile $zip -UseBasicParsing
    Expand-Archive -Path $zip -DestinationPath $cache -Force
    Remove-Item $zip -Force
    if (-not (Test-Path (Join-Path $cache 'candle.exe'))) {
        Write-Fail "WiX download did not produce candle.exe in $cache"
    }
    return $cache
}

function Build-Msi {
    Write-Step "Building MSI   name=$Name version=$Version sha=$ShortSha"

    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Write-Fail "cargo not found. Install rustup: https://rustup.rs"
    }

    Write-Step "Compiling release binary (x86_64-pc-windows-msvc)"
    & cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { Write-Fail "cargo build failed" }

    $exe = Join-Path $Root "target\release\$Name.exe"
    if (-not (Test-Path $exe)) { Write-Fail "Release binary not found at $exe" }

    $wixBin = Get-WixBinDir
    $candle = Join-Path $wixBin 'candle.exe'
    $light  = Join-Path $wixBin 'light.exe'

    $outDir = Join-Path $Root 'target\wix'
    New-Item -ItemType Directory -Force -Path $outDir | Out-Null
    $wixobj = Join-Path $outDir 'main.wixobj'
    $msi    = Join-Path $outDir "$Name-$Version-$ShortSha.msi"

    # candle compiles .wxs -> .wixobj. -dVersion / -dExePath supply the same
    # preprocessor variables that `wixl -D` does, so wix/main.wxs is shared
    # verbatim between the Linux and Windows builds.
    Write-Step "Compiling WiX source (candle)"
    & $candle -nologo -arch x64 `
        "-dVersion=$Version" `
        "-dExePath=$exe" `
        -out $wixobj `
        (Join-Path $Root 'wix\main.wxs')
    if ($LASTEXITCODE -ne 0) { Write-Fail "candle failed" }

    # light links .wixobj -> .msi. No UI extension: main.wxs intentionally uses
    # no <UIRef> (see the note in the .wxs), so the stock progress UI is used.
    #
    # -sval disables ICE validation. wixl (the Linux linker this build mirrors)
    # runs no ICE validation at all, so the shared wix/main.wxs is authored
    # against wixl's behavior. Native light otherwise fails ICE38 on the
    # per-user MainExecutable component, whose file KeyPath is split from its
    # HKCU registry KeyPath into a sibling component by design (see the comments
    # in main.wxs). -sval makes light produce the same MSI wixl does.
    Write-Step "Linking MSI (light)"
    & $light -nologo -sval -out $msi $wixobj
    if ($LASTEXITCODE -ne 0) { Write-Fail "light failed" }

    Write-Step "Built MSI: $msi"
}

switch ($Target) {
    'msi'   { Build-Msi }
    default { Write-Fail "Unknown target: $Target" }
}
