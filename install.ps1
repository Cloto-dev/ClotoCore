# ============================================================
# Exiv Quick Installer for Windows
# Downloads a pre-built binary from GitHub Releases and installs it.
#
# Usage:
#   .\install.ps1
#
# Environment variables:
#   EXIV_PREFIX   Install directory (default: C:\ProgramData\Exiv)
#   EXIV_VERSION  Version to install (default: latest)
#   EXIV_SERVICE  Set to "true" to register as Windows service
# ============================================================

$ErrorActionPreference = "Stop"

# --- Logging ---
$LogFile = Join-Path $env:TEMP "exiv-install.log"

function Write-Log {
    param(
        [string]$Message,
        [string]$Level = "INFO"
    )
    $Timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
    $Entry = "[$Timestamp] [$Level] $Message"
    Add-Content -Path $LogFile -Value $Entry -ErrorAction SilentlyContinue
}

function Write-Step {
    param(
        [string]$Message,
        [ConsoleColor]$Color = "White"
    )
    Write-Host $Message -ForegroundColor $Color
    Write-Log $Message
}

function Write-Err {
    param([string]$Message)
    Write-Host "Error: $Message" -ForegroundColor Red
    Write-Log $Message "ERROR"
}

# --- Admin elevation ---
function Assert-Administrator {
    $CurrentPrincipal = New-Object Security.Principal.WindowsPrincipal(
        [Security.Principal.WindowsIdentity]::GetCurrent()
    )
    if (-not $CurrentPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        Write-Step "Requesting administrator privileges..." -Color Yellow
        Write-Log "Not running as admin, attempting elevation"
        try {
            $ScriptPath = $MyInvocation.ScriptName
            if (-not $ScriptPath) {
                Write-Err "Please save install.ps1 to a file and run it with: powershell -ExecutionPolicy Bypass -File install.ps1"
                exit 1
            }
            Start-Process powershell.exe -Verb RunAs -ArgumentList "-ExecutionPolicy Bypass -File `"$ScriptPath`""
            exit 0
        } catch {
            Write-Err "Administrator privileges are required. Please run as Administrator."
            exit 1
        }
    }
    Write-Log "Running with administrator privileges"
}

# --- Retry logic ---
function Invoke-DownloadWithRetry {
    param(
        [string]$Uri,
        [string]$OutFile,
        [int]$MaxRetries = 3
    )
    $WebRequestParams = @{
        Uri             = $Uri
        OutFile         = $OutFile
        UseBasicParsing = $true
    }
    # Proxy support
    if ($env:HTTPS_PROXY) {
        $WebRequestParams.Proxy = $env:HTTPS_PROXY
        Write-Log "Using HTTPS proxy: $env:HTTPS_PROXY"
    } elseif ($env:HTTP_PROXY) {
        $WebRequestParams.Proxy = $env:HTTP_PROXY
        Write-Log "Using HTTP proxy: $env:HTTP_PROXY"
    }

    for ($Attempt = 1; $Attempt -le $MaxRetries; $Attempt++) {
        try {
            Invoke-WebRequest @WebRequestParams
            Write-Log "Download succeeded: $Uri (attempt $Attempt)"
            return
        } catch {
            Write-Log "Download attempt $Attempt/$MaxRetries failed: $_" "WARN"
            if ($Attempt -eq $MaxRetries) {
                throw "Download failed after $MaxRetries attempts: $Uri`n$_"
            }
            $BackoffSec = [math]::Pow(2, $Attempt)
            Write-Step "  Retrying in ${BackoffSec}s... (attempt $Attempt/$MaxRetries)" -Color Yellow
            Start-Sleep -Seconds $BackoffSec
        }
    }
}

# --- PATH management ---
function Add-ToSystemPath {
    param([string]$Directory)
    $CurrentPath = [System.Environment]::GetEnvironmentVariable("Path", [System.EnvironmentVariableTarget]::Machine)
    if ($CurrentPath -split ";" | Where-Object { $_ -eq $Directory }) {
        Write-Log "PATH already contains $Directory"
        return
    }
    $NewPath = "$CurrentPath;$Directory"
    [System.Environment]::SetEnvironmentVariable("Path", $NewPath, [System.EnvironmentVariableTarget]::Machine)
    $env:Path = "$env:Path;$Directory"
    Write-Step "  Added to system PATH: $Directory" -Color Green
    Write-Log "Added to system PATH: $Directory"
}

# --- Registry (Add/Remove Programs) ---
function Register-Uninstaller {
    param(
        [string]$InstallDir,
        [string]$Version
    )
    $RegKey = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Exiv"
    $UninstallScript = Join-Path $InstallDir "uninstall.ps1"

    New-Item -Path $RegKey -Force | Out-Null
    Set-ItemProperty -Path $RegKey -Name "DisplayName" -Value "Exiv"
    Set-ItemProperty -Path $RegKey -Name "DisplayVersion" -Value $Version
    Set-ItemProperty -Path $RegKey -Name "Publisher" -Value "Exiv Project"
    Set-ItemProperty -Path $RegKey -Name "InstallLocation" -Value $InstallDir
    Set-ItemProperty -Path $RegKey -Name "UninstallString" -Value "powershell.exe -ExecutionPolicy Bypass -File `"$UninstallScript`""
    Set-ItemProperty -Path $RegKey -Name "DisplayIcon" -Value (Join-Path $InstallDir "exiv_system.exe")
    Set-ItemProperty -Path $RegKey -Name "NoModify" -Value 1 -Type DWord
    Set-ItemProperty -Path $RegKey -Name "NoRepair" -Value 1 -Type DWord
    Set-ItemProperty -Path $RegKey -Name "InstallDate" -Value (Get-Date -Format "yyyyMMdd")
    Write-Step "  Registered in Add/Remove Programs" -Color Green
    Write-Log "Registry entry created: $RegKey"
}

# --- Rollback ---
$Script:RollbackActions = @()

function Add-RollbackAction {
    param([scriptblock]$Action, [string]$Description)
    $Script:RollbackActions += @{ Action = $Action; Description = $Description }
}

function Invoke-Rollback {
    if ($Script:RollbackActions.Count -eq 0) { return }
    Write-Step "Rolling back changes..." -Color Yellow
    Write-Log "Starting rollback ($($Script:RollbackActions.Count) actions)"
    for ($i = $Script:RollbackActions.Count - 1; $i -ge 0; $i--) {
        $rb = $Script:RollbackActions[$i]
        try {
            Write-Log "Rollback: $($rb.Description)"
            & $rb.Action
        } catch {
            Write-Log "Rollback action failed: $($rb.Description) - $_" "WARN"
        }
    }
    Write-Step "Rollback complete." -Color Yellow
}

# ============================================================
# Main
# ============================================================

Write-Log "=== Exiv Installer started ==="

Assert-Administrator

$Repo = "Exiv-ai/Exiv"
$InstallDir = if ($env:EXIV_PREFIX) { $env:EXIV_PREFIX } else { "C:\ProgramData\Exiv" }
$Version = if ($env:EXIV_VERSION) { $env:EXIV_VERSION } else { "latest" }
$SetupService = if ($env:EXIV_SERVICE -eq "true") { $true } else { $false }
$Platform = "windows-x64"

Write-Host ""
Write-Host "  ______        _       " -ForegroundColor Cyan
Write-Host " |  ____|      (_)      " -ForegroundColor Cyan
Write-Host " | |__   __  __ ___   __" -ForegroundColor Cyan
Write-Host " |  __|  \ \/ /| \ \ / /" -ForegroundColor Cyan
Write-Host " | |____  >  < | |\ V / " -ForegroundColor Cyan
Write-Host " |______|/_/\_\|_| \_/  " -ForegroundColor Cyan
Write-Host ""
Write-Host "  Exiv Installer" -ForegroundColor Cyan
Write-Host "  Platform: $Platform"
Write-Host "  Log file: $LogFile"
Write-Host ""

Write-Log "Config: InstallDir=$InstallDir Version=$Version Service=$SetupService Platform=$Platform"

# --- Resolve version ---
if ($Version -eq "latest") {
    Write-Step "  Resolving latest version..." -Color White
    try {
        $ApiParams = @{
            Uri     = "https://api.github.com/repos/$Repo/releases/latest"
            Headers = @{ "User-Agent" = "Exiv-Installer" }
        }
        if ($env:HTTPS_PROXY) { $ApiParams.Proxy = $env:HTTPS_PROXY }
        elseif ($env:HTTP_PROXY) { $ApiParams.Proxy = $env:HTTP_PROXY }

        $Release = Invoke-RestMethod @ApiParams
        $Version = $Release.tag_name
    } catch {
        Write-Err "Failed to fetch latest release. Set EXIV_VERSION explicitly."
        exit 1
    }
}
$VersionNum = $Version -replace "^v", ""
# Validate version format (semver)
if ($VersionNum -notmatch '^\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?$') {
    Write-Err "Invalid version format '$VersionNum'. Expected semver (e.g., 1.2.3 or 1.2.3-beta.1)"
    exit 1
}
Write-Step "  Version:  v$VersionNum" -Color Green

# --- Download ---
$Archive = "exiv-$VersionNum-$Platform.zip"
$Url = "https://github.com/$Repo/releases/download/v$VersionNum/$Archive"
$ChecksumUrl = "$Url.sha256"

$TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) "exiv-install-$(Get-Random)"
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null
Write-Log "Temp directory: $TmpDir"

try {
    Write-Host ""
    Write-Step "Downloading $Archive..." -Color Cyan
    try {
        Invoke-DownloadWithRetry -Uri $Url -OutFile (Join-Path $TmpDir $Archive)
    } catch {
        Write-Err "Download failed. Check that version v$VersionNum exists at https://github.com/$Repo/releases"
        exit 1
    }

    # --- Verify checksum ---
    try {
        Invoke-DownloadWithRetry -Uri $ChecksumUrl -OutFile (Join-Path $TmpDir "$Archive.sha256")
        $ExpectedLine = (Get-Content (Join-Path $TmpDir "$Archive.sha256") -Raw).Trim()
        $ExpectedHash = ($ExpectedLine -split "\s+")[0].ToLower()
        $ActualHash = (Get-FileHash (Join-Path $TmpDir $Archive) -Algorithm SHA256).Hash.ToLower()

        if ($ExpectedHash -ne $ActualHash) {
            Write-Err "Checksum verification failed."
            Write-Host "  Expected: $ExpectedHash" -ForegroundColor Red
            Write-Host "  Actual:   $ActualHash" -ForegroundColor Red
            Write-Log "Checksum mismatch: expected=$ExpectedHash actual=$ActualHash" "ERROR"
            exit 1
        }
        Write-Step "Checksum verified." -Color Green
    } catch {
        Write-Step "  (checksum file not available, skipping verification)" -Color Yellow
        Write-Log "Checksum file not available, skipping" "WARN"
    }

    # --- Extract ---
    Write-Step "Extracting..." -Color White
    $ExtractDir = Join-Path $TmpDir "extracted"
    Expand-Archive -Path (Join-Path $TmpDir $Archive) -DestinationPath $ExtractDir -Force

    $Binary = Join-Path $ExtractDir "exiv_system.exe"
    if (-not (Test-Path $Binary)) {
        # Archive may contain a subdirectory
        $Binary = Get-ChildItem -Path $ExtractDir -Recurse -Filter "exiv_system.exe" | Select-Object -First 1 -ExpandProperty FullName
        if (-not $Binary) {
            Write-Err "Binary not found in archive."
            exit 1
        }
    }
    Write-Log "Binary found: $Binary"

    # --- Install via the binary's self-install command ---
    Write-Host ""
    Write-Step "Installing to $InstallDir..." -Color Cyan

    $InstallArgs = @("install", "--prefix", $InstallDir)
    if ($SetupService) {
        $InstallArgs += "--service"
    }
    Write-Log "Running: $Binary $($InstallArgs -join ' ')"

    & $Binary @InstallArgs
    if ($LASTEXITCODE -ne 0) {
        Write-Err "Installation failed (exit code: $LASTEXITCODE)."
        Invoke-Rollback
        exit 1
    }
    Add-RollbackAction -Description "Remove install directory" -Action {
        if (Test-Path $InstallDir) { Remove-Item -Path $InstallDir -Recurse -Force -ErrorAction SilentlyContinue }
    }

    # --- Add to PATH ---
    Write-Step "Configuring system PATH..." -Color Cyan
    try {
        Add-ToSystemPath -Directory $InstallDir
        Add-RollbackAction -Description "Remove from PATH" -Action {
            $p = [System.Environment]::GetEnvironmentVariable("Path", [System.EnvironmentVariableTarget]::Machine)
            $p = ($p -split ";" | Where-Object { $_ -ne $InstallDir }) -join ";"
            [System.Environment]::SetEnvironmentVariable("Path", $p, [System.EnvironmentVariableTarget]::Machine)
        }
    } catch {
        Write-Step "  (PATH setup skipped: $_)" -Color Yellow
        Write-Log "PATH setup failed: $_" "WARN"
    }

    # --- Deploy uninstall.ps1 ---
    $UninstallSource = Join-Path (Split-Path -Parent $MyInvocation.MyCommand.Definition) "uninstall.ps1"
    $UninstallDest = Join-Path $InstallDir "uninstall.ps1"
    if (Test-Path $UninstallSource) {
        Copy-Item -Path $UninstallSource -Destination $UninstallDest -Force
        Write-Step "  Uninstaller deployed to $UninstallDest" -Color Green
        Write-Log "Uninstaller copied to $UninstallDest"
    }

    # --- Register in Add/Remove Programs ---
    Write-Step "Registering in Windows..." -Color Cyan
    try {
        Register-Uninstaller -InstallDir $InstallDir -Version $VersionNum
        Add-RollbackAction -Description "Remove registry entry" -Action {
            Remove-Item -Path "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Exiv" -Force -ErrorAction SilentlyContinue
        }
    } catch {
        Write-Step "  (Registry setup skipped: $_)" -Color Yellow
        Write-Log "Registry setup failed: $_" "WARN"
    }

    # --- Success ---
    Write-Host ""
    Write-Step "Exiv v$VersionNum installed successfully!" -Color Green
    Write-Host ""
    Write-Host "  Binary:    $InstallDir\exiv_system.exe" -ForegroundColor Cyan
    Write-Host "  Dashboard: http://localhost:8081" -ForegroundColor Cyan
    Write-Host "  Manage:    exiv_system.exe service start|stop|status" -ForegroundColor Cyan
    Write-Host "  Uninstall: powershell -File `"$InstallDir\uninstall.ps1`"" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "  NOTE: Restart your terminal to use 'exiv_system' from PATH." -ForegroundColor Yellow
    Write-Host ""
    Write-Log "Installation completed successfully: v$VersionNum -> $InstallDir"

} finally {
    # Cleanup
    if (Test-Path $TmpDir) {
        Remove-Item -Path $TmpDir -Recurse -Force -ErrorAction SilentlyContinue
        Write-Log "Cleaned up temp directory: $TmpDir"
    }
    Write-Log "=== Exiv Installer finished ==="
}
