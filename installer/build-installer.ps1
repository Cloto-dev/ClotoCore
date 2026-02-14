# ============================================================
# Exiv Installer Build Script
# Compiles the Inno Setup script into an .exe installer.
#
# Usage:
#   .\build-installer.ps1 -Version 0.2.0 -BinaryPath ..\target\release\exiv_system.exe
#
# Requirements:
#   - Inno Setup 6+ (ISCC.exe in PATH or default install location)
# ============================================================

param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [Parameter(Mandatory = $true)]
    [string]$BinaryPath,

    [string]$OutputDir = "output"
)

$ErrorActionPreference = "Stop"

Write-Host "Exiv Installer Build" -ForegroundColor Cyan
Write-Host "  Version:    $Version"
Write-Host "  Binary:     $BinaryPath"
Write-Host "  Output dir: $OutputDir"
Write-Host ""

# --- Locate ISCC.exe ---
$IsccPaths = @(
    "ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
    "C:\Program Files (x86)\Inno Setup 6\ISCC.exe"
)

$Iscc = $null
foreach ($Path in $IsccPaths) {
    if (Get-Command $Path -ErrorAction SilentlyContinue) {
        $Iscc = $Path
        break
    }
    if (Test-Path $Path) {
        $Iscc = $Path
        break
    }
}

if (-not $Iscc) {
    Write-Host "Error: ISCC.exe not found. Install Inno Setup 6." -ForegroundColor Red
    exit 1
}
Write-Host "  ISCC:       $Iscc" -ForegroundColor Green

# --- Prepare build directory ---
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$BuildDir = Join-Path $ScriptDir "build"

if (Test-Path $BuildDir) {
    Remove-Item -Path $BuildDir -Recurse -Force
}
New-Item -ItemType Directory -Path $BuildDir -Force | Out-Null

# Copy binary to build directory
Copy-Item -Path $BinaryPath -Destination (Join-Path $BuildDir "exiv_system.exe") -Force
Write-Host "  Copied binary to build directory" -ForegroundColor Green

# --- Ensure output directory ---
$AbsOutputDir = Join-Path $ScriptDir $OutputDir
if (-not (Test-Path $AbsOutputDir)) {
    New-Item -ItemType Directory -Path $AbsOutputDir -Force | Out-Null
}

# --- Compile ---
Write-Host ""
Write-Host "Compiling installer..." -ForegroundColor Cyan

$IssFile = Join-Path $ScriptDir "exiv-setup.iss"
& $Iscc "/DAppVersion=$Version" "/O$AbsOutputDir" $IssFile

if ($LASTEXITCODE -ne 0) {
    Write-Host "Error: ISCC compilation failed (exit code: $LASTEXITCODE)" -ForegroundColor Red
    exit 1
}

# --- Generate checksum ---
$InstallerName = "exiv-setup-$Version.exe"
$InstallerPath = Join-Path $AbsOutputDir $InstallerName

if (-not (Test-Path $InstallerPath)) {
    Write-Host "Error: Expected output not found: $InstallerPath" -ForegroundColor Red
    exit 1
}

$Hash = (Get-FileHash -Path $InstallerPath -Algorithm SHA256).Hash.ToLower()
$ChecksumFile = Join-Path $AbsOutputDir "$InstallerName.sha256"
"$Hash  $InstallerName" | Set-Content -Path $ChecksumFile -NoNewline

Write-Host ""
Write-Host "Build complete!" -ForegroundColor Green
Write-Host "  Installer: $InstallerPath"
Write-Host "  SHA256:    $Hash"
Write-Host "  Checksum:  $ChecksumFile"

# --- Cleanup ---
Remove-Item -Path $BuildDir -Recurse -Force -ErrorAction SilentlyContinue
