# Build the Hetao Capsule installer with consistency checks.
# Usage: powershell -ExecutionPolicy Bypass -File build-installer.ps1
#
# Checks performed:
#   1. AppVer in hetao-capsule.iss must equal the version in capsule/Cargo.toml.
#   2. Both hetao-capsule.exe and hetao-screen.exe are rebuilt before packaging.
#   3. Both executables must exist after the build.
#   4. Inno Setup compiler ISCC.exe must be available.
#   5. The expected installer output file must be produced.

$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$capsuleDir = Split-Path -Parent $scriptDir
$rootDir = Split-Path -Parent $capsuleDir
$issPath = Join-Path $scriptDir "hetao-capsule.iss"

function Read-Version([string]$path, [string]$pattern, [string]$label) {
    $text = Get-Content -Raw -LiteralPath $path
    if ($text -notmatch $pattern) {
        throw "Cannot find version in $label ($path)"
    }
    return $Matches[1]
}

$capsuleVersion = Read-Version (Join-Path $capsuleDir "Cargo.toml") '(?m)^version\s*=\s*"([^"]+)"' "capsule Cargo.toml"
$screenVersion = Read-Version (Join-Path $rootDir "windows-screen\Cargo.toml") '(?m)^version\s*=\s*"([^"]+)"' "screen workspace Cargo.toml"
$issVersion = Read-Version $issPath '(?m)^#define\s+AppVer\s+"([^"]+)"' "hetao-capsule.iss"

if ($issVersion -ne $capsuleVersion) {
    throw "Version mismatch: installer AppVer=$issVersion, capsule Cargo.toml=$capsuleVersion. Update #define AppVer."
}

Write-Host "capsule version : $capsuleVersion"
Write-Host "installer AppVer: $issVersion"
Write-Host "screen version  : $screenVersion (independent)"

Write-Host "Building capsule..."
& cargo build --release --manifest-path (Join-Path $capsuleDir "Cargo.toml")
if ($LASTEXITCODE -ne 0) {
    throw "cargo build failed for capsule"
}

Write-Host "Building screen app..."
& cargo build --release -p screen-app --manifest-path (Join-Path $rootDir "windows-screen\Cargo.toml")
if ($LASTEXITCODE -ne 0) {
    throw "cargo build failed for screen-app"
}

$capsuleExe = Join-Path $capsuleDir "target\release\hetao-capsule.exe"
$screenExe = Join-Path $rootDir "windows-screen\target\release\hetao-screen.exe"
foreach ($exe in @($capsuleExe, $screenExe)) {
    if (-not (Test-Path -LiteralPath $exe)) {
        throw "Missing executable: $exe"
    }
    $item = Get-Item -LiteralPath $exe
    $hash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
    Write-Host ("{0}  {1} bytes  SHA256 {2}" -f $item.Name, $item.Length, $hash)
}

$iscc = Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"
if (-not (Test-Path -LiteralPath $iscc)) {
    $iscc = Join-Path $env:ProgramFiles "Inno Setup 6\ISCC.exe"
}
if (-not (Test-Path -LiteralPath $iscc)) {
    throw "ISCC.exe not found. Install Inno Setup 6."
}

Write-Host "Compiling installer..."
& $iscc $issPath
if ($LASTEXITCODE -ne 0) {
    throw "ISCC failed"
}

$setupExe = Join-Path $scriptDir ("output\hetao-capsule-setup-{0}.exe" -f $issVersion)
if (-not (Test-Path -LiteralPath $setupExe)) {
    throw "Installer output missing: $setupExe"
}
$setupItem = Get-Item -LiteralPath $setupExe
$setupHash = (Get-FileHash -LiteralPath $setupExe -Algorithm SHA256).Hash
Write-Host ("installer: {0}  {1} bytes  SHA256 {2}" -f $setupItem.FullName, $setupItem.Length, $setupHash)
