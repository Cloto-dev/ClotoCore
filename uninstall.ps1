# ============================================================
# Cloto Uninstaller for Windows
# Removes ClotoCore installation, service, PATH entry, and registry.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File uninstall.ps1
# ============================================================

$ErrorActionPreference = "Stop"

$LogFile = Join-Path $env:TEMP "cloto-uninstall.log"

function Write-Log {
    param([string]$Message, [string]$Level = "INFO")
    $Entry = "[$(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')] [$Level] $Message"
    Add-Content -Path $LogFile -Value $Entry -ErrorAction SilentlyContinue
}

function Write-Step {
    param([string]$Message, [ConsoleColor]$Color = "White")
    Write-Host $Message -ForegroundColor $Color
    Write-Log $Message
}

function Write-Err {
    param([string]$Message)
    Write-Host "Error: $Message" -ForegroundColor Red
    Write-Log $Message "ERROR"
}

# --- Admin check ---
$CurrentPrincipal = New-Object Security.Principal.WindowsPrincipal(
    [Security.Principal.WindowsIdentity]::GetCurrent()
)
if (-not $CurrentPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Step "Requesting administrator privileges..." -Color Yellow
    try {
        Start-Process powershell.exe -Verb RunAs -ArgumentList "-ExecutionPolicy Bypass -File `"$($MyInvocation.MyCommand.Definition)`""
        exit 0
    } catch {
        Write-Err "Administrator privileges are required."
        exit 1
    }
}

# --- Determine install directory ---
$RegKey = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\ClotoCore"
if (Test-Path $RegKey) {
    $InstallDir = (Get-ItemProperty -Path $RegKey -Name "InstallLocation" -ErrorAction SilentlyContinue).InstallLocation
}
if (-not $InstallDir) {
    # Fallback: script is in the install directory
    $ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
    if (Test-Path (Join-Path $ScriptDir "cloto_system.exe")) {
        $InstallDir = $ScriptDir
    } else {
        $InstallDir = "C:\ProgramData\Cloto"
    }
}

Write-Host ""
Write-Host "  Cloto Uninstaller" -ForegroundColor Cyan
Write-Host "  Install directory: $InstallDir"
Write-Host "  Log file: $LogFile"
Write-Host ""

Write-Log "=== Cloto Uninstaller started ==="
Write-Log "Install directory: $InstallDir"

# --- Confirmation ---
$Confirm = Read-Host "Are you sure you want to uninstall ClotoCore? (y/N)"
if ($Confirm -notin @("y", "Y", "yes", "Yes")) {
    Write-Step "Uninstall cancelled." -Color Yellow
    exit 0
}

# --- Stop and remove Windows Service ---
Write-Step "Stopping ClotoCore service..." -Color Cyan
try {
    $ServiceStatus = sc.exe query ClotoCoreService 2>&1
    if ($LASTEXITCODE -eq 0) {
        sc.exe stop ClotoCoreService 2>&1 | Out-Null
        Start-Sleep -Seconds 2
        sc.exe delete ClotoCoreService 2>&1 | Out-Null
        Write-Step "  Service removed." -Color Green
        Write-Log "Windows Service 'ClotoCoreService' stopped and deleted"
    } else {
        Write-Step "  Service not found (skipping)." -Color Yellow
        Write-Log "Service not found, skipping"
    }
} catch {
    Write-Step "  Service removal failed: $_" -Color Yellow
    Write-Log "Service removal failed: $_" "WARN"
}

# --- Remove from PATH ---
Write-Step "Removing from system PATH..." -Color Cyan
try {
    $CurrentPath = [System.Environment]::GetEnvironmentVariable("Path", [System.EnvironmentVariableTarget]::Machine)
    $PathEntries = $CurrentPath -split ";"
    $FilteredEntries = $PathEntries | Where-Object { $_ -ne $InstallDir -and $_ -ne "" }
    if ($PathEntries.Count -ne $FilteredEntries.Count) {
        $NewPath = $FilteredEntries -join ";"
        [System.Environment]::SetEnvironmentVariable("Path", $NewPath, [System.EnvironmentVariableTarget]::Machine)
        Write-Step "  Removed from PATH: $InstallDir" -Color Green
        Write-Log "Removed from system PATH: $InstallDir"
    } else {
        Write-Step "  Not found in PATH (skipping)." -Color Yellow
    }
} catch {
    Write-Step "  PATH cleanup failed: $_" -Color Yellow
    Write-Log "PATH cleanup failed: $_" "WARN"
}

# --- Remove registry entry ---
Write-Step "Removing registry entry..." -Color Cyan
try {
    if (Test-Path $RegKey) {
        Remove-Item -Path $RegKey -Force
        Write-Step "  Registry entry removed." -Color Green
        Write-Log "Registry entry removed: $RegKey"
    } else {
        Write-Step "  Registry entry not found (skipping)." -Color Yellow
    }
} catch {
    Write-Step "  Registry cleanup failed: $_" -Color Yellow
    Write-Log "Registry cleanup failed: $_" "WARN"
}

# --- Remove install directory ---
Write-Step "Removing install directory..." -Color Cyan
try {
    if (Test-Path $InstallDir) {
        # Ask about data directory preservation
        $DataDir = Join-Path $InstallDir "data"
        if (Test-Path $DataDir) {
            $KeepData = Read-Host "Keep user data ($DataDir)? (y/N)"
            if ($KeepData -in @("y", "Y", "yes", "Yes")) {
                $BackupDir = Join-Path $env:USERPROFILE "ClotoCore-data-backup"
                Copy-Item -Path $DataDir -Destination $BackupDir -Recurse -Force
                Write-Step "  Data backed up to: $BackupDir" -Color Green
                Write-Log "Data directory backed up to $BackupDir"
            }
        }
        Remove-Item -Path $InstallDir -Recurse -Force
        Write-Step "  Install directory removed." -Color Green
        Write-Log "Install directory removed: $InstallDir"
    } else {
        Write-Step "  Install directory not found (skipping)." -Color Yellow
    }
} catch {
    Write-Err "Failed to remove install directory: $_"
    Write-Host "  You may need to manually delete: $InstallDir" -ForegroundColor Yellow
}

Write-Host ""
Write-Step "ClotoCore has been uninstalled." -Color Green
Write-Host ""
Write-Host "  NOTE: Restart your terminal to apply PATH changes." -ForegroundColor Yellow
Write-Host ""
Write-Log "=== Cloto Uninstaller finished ==="
