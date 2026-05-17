# Cortex Windows Installation and Service Management Script
# This script installs and manages Cortex as a Windows Service for background operation
# Requires Administrator privileges to install/uninstall services

param(
    [switch]$Install,
    [switch]$Uninstall,
    [switch]$Start,
    [switch]$Stop,
    [switch]$Status,
    [switch]$Build,           # Build from source on Windows
    [string]$InstallDir = "$env:USERPROFILE\.freshloop",
    [string]$SourceDir = "",   # Source code directory for building
    [string]$ConfigSource = ""
)

# Configuration
$ServiceName = "CortexService"
$ServiceDisplayName = "Cortex News Aggregator"
$ServiceDescription = "Background news aggregation and TTS service"
$BinaryName = "cortex.exe"
$LogDir = "$InstallDir\logs"
$CacheDir = "$InstallDir\cache"
$BinaryPath = "$InstallDir\$BinaryName"
$ConfigPath = "$InstallDir\config.toml"

# NSSM (Non-Sucking Service Manager) configuration
$NssmVersion = "2.24"
$NssmUrl = "https://nssm.cc/release/nssm-$NssmVersion.zip"
$NssmDir = "$InstallDir\nssm"
$NssmPath = "$NssmDir\nssm.exe"

function Write-Info {
    param([string]$Message)
    Write-Host "[INFO] $Message" -ForegroundColor Cyan
}

function Write-Success {
    param([string]$Message)
    Write-Host "[SUCCESS] $Message" -ForegroundColor Green
}

function Write-Error {
    param([string]$Message)
    Write-Host "[ERROR] $Message" -ForegroundColor Red
}

function Write-Warning {
    param([string]$Message)
    Write-Host "[WARNING] $Message" -ForegroundColor Yellow
}

function Test-Admin {
    $currentPrincipal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
    return $currentPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Install-Nssm {
    if (Test-Path $NssmPath) {
        Write-Info "NSSM already installed"
        return
    }

    Write-Info "Downloading NSSM (Non-Sucking Service Manager)..."

    $tempZip = "$env:TEMP\nssm.zip"
    $tempExtract = "$env:TEMP\nssm_extract"

    try {
        # Download NSSM
        Invoke-WebRequest -Uri $NssmUrl -OutFile $tempZip -UseBasicParsing

        # Extract
        Expand-Archive -Path $tempZip -DestinationPath $tempExtract -Force

        # Create nssm directory
        New-Item -ItemType Directory -Path $NssmDir -Force | Out-Null

        # Determine architecture
        $arch = if ([Environment]::Is64BitOperatingSystem) { "win64" } else { "win32" }
        $nssmSource = "$tempExtract\nssm-$NssmVersion\$arch\nssm.exe"

        Copy-Item -Path $nssmSource -Destination $NssmPath -Force

        # Cleanup
        Remove-Item -Path $tempZip -Force -ErrorAction SilentlyContinue
        Remove-Item -Path $tempExtract -Recurse -Force -ErrorAction SilentlyContinue

        Write-Success "NSSM installed to $NssmPath"
    }
    catch {
        Write-Error "Failed to install NSSM: $_"
        exit 1
    }
}

function Test-RustInstalled {
    try {
        $rustVersion = rustc --version 2>$null
        if ($rustVersion) {
            Write-Info "Rust found: $rustVersion"
            return $true
        }
    }
    catch {
        return $false
    }
    return $false
}

function Install-Rust {
    Write-Info "Rust not found. Installing Rust..."
    Write-Info "Downloading rustup-init.exe..."

    $rustupInit = "$env:TEMP\rustup-init.exe"

    try {
        Invoke-WebRequest -Uri "https://win.rustup.rs/x86_64" -OutFile $rustupInit -UseBasicParsing

        Write-Info "Running rustup installer (default options)..."
        Start-Process -FilePath $rustupInit -ArgumentList "-y" -Wait -NoNewWindow

        # Refresh PATH
        $env:PATH = [System.Environment]::GetEnvironmentVariable("PATH", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("PATH", "User")

        Remove-Item -Path $rustupInit -Force -ErrorAction SilentlyContinue

        if (Test-RustInstalled) {
            Write-Success "Rust installed successfully"
        }
        else {
            Write-Error "Rust installation may have failed. Please restart PowerShell and try again."
            exit 1
        }
    }
    catch {
        Write-Error "Failed to install Rust: $_"
        exit 1
    }
}

function Build-Cortex {
    param([string]$BuildSourceDir)

    Write-Info "Building Cortex from source..."

    # Check/Install Rust
    if (-not (Test-RustInstalled)) {
        Install-Rust
    }

    # Determine source directory
    $cortexDir = ""

    if ($BuildSourceDir -and (Test-Path "$BuildSourceDir\backend\cortex")) {
        $cortexDir = "$BuildSourceDir\backend\cortex"
    }
    elseif (Test-Path "$PSScriptRoot\..\backend\cortex") {
        $cortexDir = "$PSScriptRoot\..\backend\cortex"
    }
    elseif (Test-Path "$PSScriptRoot\backend\cortex") {
        $cortexDir = "$PSScriptRoot\backend\cortex"
    }
    else {
        Write-Error "Cannot find Cortex source code."
        Write-Info "Expected to find 'backend\cortex' directory."
        Write-Info "Please run this script from the project root or provide -SourceDir"
        exit 1
    }

    Write-Info "Source directory: $cortexDir"

    # Build
    Push-Location $cortexDir
    try {
        Write-Info "Compiling Cortex (this may take several minutes)..."
        cargo build --release -p cortex

        if ($LASTEXITCODE -ne 0) {
            Write-Error "Build failed!"
            exit 1
        }

        $builtExe = "$cortexDir\..\..\target\release\cortex.exe"
        if (-not (Test-Path $builtExe)) {
            Write-Error "Build succeeded but executable not found at: $builtExe"
            exit 1
        }

        Write-Success "Build successful!"
        return $builtExe
    }
    finally {
        Pop-Location
    }
}

function Install-CortexService {
    Write-Info "Installing Cortex Windows Service..."

    # Check if running as admin
    if (-not (Test-Admin)) {
        Write-Error "This script requires Administrator privileges to install services"
        Write-Info "Please run PowerShell as Administrator and try again"
        exit 1
    }

    # Create directories
    Write-Info "Creating directories..."
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    New-Item -ItemType Directory -Path $LogDir -Force | Out-Null
    New-Item -ItemType Directory -Path $CacheDir -Force | Out-Null

    # Determine binary source
    # Priority: 1. Build from source (-Build), 2. Same directory as script, 3. Build directory
    if ($Build) {
        $builtExe = Build-Cortex -BuildSourceDir $SourceDir
        Write-Info "Copying built binary..."
        Copy-Item -Path $builtExe -Destination $BinaryPath -Force
    }
    else {
        # Try to find pre-built binary
        $localBinary = "$PSScriptRoot\cortex.exe"
        $sourceBinary = "$PSScriptRoot\..\backend\target\x86_64-pc-windows-msvc\release\cortex.exe"
        $buildBinary = "$PSScriptRoot\..\backend\target\release\cortex.exe"

        if (Test-Path $localBinary) {
            Write-Info "Copying binary from script directory..."
            Copy-Item -Path $localBinary -Destination $BinaryPath -Force
        }
        elseif (Test-Path $sourceBinary) {
            Write-Info "Copying binary from cross-compile directory..."
            Copy-Item -Path $sourceBinary -Destination $BinaryPath -Force
        }
        elseif (Test-Path $buildBinary) {
            Write-Info "Copying binary from local build directory..."
            Copy-Item -Path $buildBinary -Destination $BinaryPath -Force
        }
        else {
            Write-Warning "No pre-built binary found!"
            Write-Info "You can:"
            Write-Info "  1. Place cortex.exe in the same directory as this script"
            Write-Info "  2. Build from source using: .\install.ps1 -Install -Build"

            $buildChoice = Read-Host "Do you want to build from source now? (y/n)"
            if ($buildChoice -eq 'y' -or $buildChoice -eq 'Y') {
                $builtExe = Build-Cortex -BuildSourceDir $SourceDir
                Write-Info "Copying built binary..."
                Copy-Item -Path $builtExe -Destination $BinaryPath -Force
            }
            else {
                Write-Error "Installation cancelled - no binary available"
                exit 1
            }
        }
    }

    # Copy config - priority: 1. Specified, 2. Same dir as script, 3. Project root
    $localConfig = "$PSScriptRoot\config.toml"
    $projectConfig = "$PSScriptRoot\..\config.toml"

    if ($ConfigSource -and (Test-Path $ConfigSource)) {
        Write-Info "Copying configuration file from specified location..."
        Copy-Item -Path $ConfigSource -Destination $ConfigPath -Force
    }
    elseif (Test-Path $localConfig) {
        Write-Info "Copying configuration file from script directory..."
        Copy-Item -Path $localConfig -Destination $ConfigPath -Force
    }
    elseif (Test-Path $projectConfig) {
        Write-Info "Copying configuration file from project root..."
        Copy-Item -Path $projectConfig -Destination $ConfigPath -Force
    }
    else {
        Write-Warning "Config file not found, creating default config..."
        @"
[nexus]
api_url = "http://localhost:8899"
auth_key = "CHANGE_ME_NEXUS_KEY"

[llm]
model = "llama3"
api_url = "http://localhost:11434"

[tts]
model_path = ".\zh_CN-huayan-medium.onnx"

[[sources]]
name = "Hacker News"
url = "https://news.ycombinator.com/rss"
interval_min = 60
tags = ["Tech", "Global"]
"@ | Out-File -FilePath $ConfigPath -Encoding UTF8
    }

    # Install NSSM
    Install-Nssm

    # Remove existing service if present
    $existingService = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if ($existingService) {
        Write-Info "Removing existing service..."
        & $NssmPath remove $ServiceName confirm 2>$null
    }

    # Install service using NSSM
    Write-Info "Installing Windows Service..."
    & $NssmPath install $ServiceName $BinaryPath

    # Configure service
    & $NssmPath set $ServiceName DisplayName $ServiceDisplayName
    & $NssmPath set $ServiceName Description $ServiceDescription
    & $NssmPath set $ServiceName AppDirectory $InstallDir

    # Configure logging
    & $NssmPath set $ServiceName AppStdout "$LogDir\cortex.out.log"
    & $NssmPath set $ServiceName AppStderr "$LogDir\cortex.err.log"
    & $NssmPath set $ServiceName AppRotateFiles 1
    & $NssmPath set $ServiceName AppRotateBytes 10485760  # 10MB

    # Configure restart behavior
    & $NssmPath set $ServiceName AppExit Default Restart
    & $NssmPath set $ServiceName AppThrottle 5000

    # Set service to auto-start
    & $NssmPath set $ServiceName Start SERVICE_AUTO_START

    Write-Success "Service installed successfully!"
    Write-Info ""
    Write-Info "Installation details:"
    Write-Info "  Binary: $BinaryPath"
    Write-Info "  Config: $ConfigPath"
    Write-Info "  Logs:   $LogDir"
    Write-Info "  Cache:  $CacheDir"
    Write-Info ""
    Write-Info "To start the service, run:"
    Write-Info "  $(Split-Path -Leaf $PSCommandPath) -Start"
}

function Uninstall-CortexService {
    Write-Info "Uninstalling Cortex Windows Service..."

    if (-not (Test-Admin)) {
        Write-Error "This script requires Administrator privileges"
        exit 1
    }

    if (-not (Test-Path $NssmPath)) {
        Write-Error "NSSM not found. Cannot uninstall service."
        exit 1
    }

    $service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if ($service) {
        if ($service.Status -eq "Running") {
            Write-Info "Stopping service..."
            Stop-Service -Name $ServiceName -Force
        }

        Write-Info "Removing service..."
        & $NssmPath remove $ServiceName confirm

        Write-Success "Service uninstalled successfully"
    }
    else {
        Write-Warning "Service not found"
    }

    Write-Info ""
    Write-Info "Note: The following directories were NOT removed:"
    Write-Info "  - $InstallDir (binary, config, data)"
    Write-Info "  - $LogDir (log files)"
    Write-Info ""
    Write-Info "To completely remove all data, manually delete:"
    Write-Info "  $InstallDir"
}

function Start-CortexService {
    Write-Info "Starting Cortex service..."

    if (-not (Test-Admin)) {
        Write-Error "This script requires Administrator privileges"
        exit 1
    }

    $service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if (-not $service) {
        Write-Error "Service not found. Please install first: $(Split-Path -Leaf $PSCommandPath) -Install"
        exit 1
    }

    if ($service.Status -eq "Running") {
        Write-Warning "Service is already running"
        return
    }

    Start-Service -Name $ServiceName
    Start-Sleep -Seconds 2

    $service = Get-Service -Name $ServiceName
    if ($service.Status -eq "Running") {
        Write-Success "Service started successfully"
    }
    else {
        Write-Error "Failed to start service. Check logs at: $LogDir"
    }
}

function Stop-CortexService {
    Write-Info "Stopping Cortex service..."

    if (-not (Test-Admin)) {
        Write-Error "This script requires Administrator privileges"
        exit 1
    }

    $service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if (-not $service) {
        Write-Warning "Service not found"
        return
    }

    if ($service.Status -ne "Running") {
        Write-Warning "Service is not running"
        return
    }

    Stop-Service -Name $ServiceName -Force
    Write-Success "Service stopped"
}

function Get-CortexStatus {
    $service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue

    if (-not $service) {
        Write-Warning "Service not installed"
        Write-Info "To install, run: $(Split-Path -Leaf $PSCommandPath) -Install"
        return
    }

    Write-Info "Service Name:    $ServiceName"
    Write-Info "Display Name:    $($service.DisplayName)"
    Write-Info "Status:          $($service.Status)"
    Write-Info "Start Type:      $($service.StartType)"
    Write-Info ""
    Write-Info "Paths:"
    Write-Info "  Install Dir:   $InstallDir"
    Write-Info "  Binary:        $BinaryPath"
    Write-Info "  Config:        $ConfigPath"
    Write-Info "  Logs:          $LogDir"
    Write-Info ""

    if (Test-Path "$LogDir\cortex.out.log") {
        Write-Info "Recent log entries (last 10 lines):"
        Get-Content "$LogDir\cortex.out.log" -Tail 10 | ForEach-Object { Write-Host "  $_" }
    }
}

# Main script logic
Write-Info "Cortex Windows Service Manager"
Write-Info "=============================="
Write-Info ""

# Get the script name for dynamic help
$ScriptName = Split-Path -Leaf $PSCommandPath

# Show help if no parameters provided
if (-not ($Install -or $Uninstall -or $Start -or $Stop -or $Status -or $Build)) {
    Write-Host "Usage: .\$ScriptName [options]" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Options:" -ForegroundColor Yellow
    Write-Host "  -Install      Install Cortex as a Windows Service"
    Write-Host "  -Uninstall    Remove the Windows Service"
    Write-Host "  -Start        Start the service"
    Write-Host "  -Stop         Stop the service"
    Write-Host "  -Status       Show service status and recent logs"
    Write-Host "  -Build        Build from source (used with -Install)"
    Write-Host ""
    Write-Host "Examples:" -ForegroundColor Yellow
    Write-Host "  # Install with pre-built binary (requires Admin):"
    Write-Host "  .\$ScriptName -Install"
    Write-Host ""
    Write-Host "  # Build and install from source:"
    Write-Host "  .\$ScriptName -Install -Build"
    Write-Host ""
    Write-Host "  # Start the service:"
    Write-Host "  .\$ScriptName -Start"
    Write-Host ""
    Write-Host "Installation Directory: $InstallDir" -ForegroundColor Gray
    exit 0
}

# Execute requested actions
if ($Uninstall) {
    Uninstall-CortexService
}

if ($Install) {
    Install-CortexService
}

if ($Start) {
    Start-CortexService
}

if ($Stop) {
    Stop-CortexService
}

if ($Status) {
    Get-CortexStatus
}

Write-Info "Done!"
