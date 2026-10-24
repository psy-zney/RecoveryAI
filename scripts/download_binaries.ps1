# PowerShell Script to Download and Setup Portable Forensic Binaries (TSK & PhotoRec)
# Usage: powershell -ExecutionPolicy Bypass -File scripts\download_binaries.ps1

$ErrorActionPreference = "Stop"

$BinDir = Join-Path $PSScriptRoot "..\binaries"
if (!(Test-Path $BinDir)) {
    New-Item -ItemType Directory -Path $BinDir | Out-Null
}

Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "  RecoveryAI — Portable Forensic Tools Downloader" -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan

# 1. Download TestDisk & PhotoRec
$TestDiskDir = Join-Path $BinDir "testdisk"
if (!(Test-Path (Join-Path $TestDiskDir "photorec_win.exe"))) {
    Write-Host "[1/2] Downloading TestDisk / PhotoRec 7.2 for Windows..." -ForegroundColor Yellow
    $TestDiskZip = Join-Path $BinDir "testdisk.zip"
    $TestDiskUrl = "https://www.cgsecurity.org/testdisk-7.2.win.zip"
    
    try {
        Invoke-WebRequest -Uri $TestDiskUrl -OutFile $TestDiskZip -UserAgent "Mozilla/5.0"
        Write-Host "Extracting TestDisk to $BinDir..." -ForegroundColor Green
        Expand-Archive -Path $TestDiskZip -DestinationPath $BinDir -Force
        Remove-Item $TestDiskZip -Force
        
        # Rename extracted folder to 'testdisk'
        $Extracted = Get-ChildItem -Path $BinDir -Directory -Filter "testdisk-*" | Select-Object -First 1
        if ($Extracted) {
            Rename-Item -Path $Extracted.FullName -NewName "testdisk" -Force
        }
        Write-Host "PhotoRec successfully installed at binaries\testdisk\photorec_win.exe" -ForegroundColor Green
    } catch {
        Write-Warning "Could not automatically download TestDisk: $_"
    }
} else {
    Write-Host "[1/2] PhotoRec is already installed." -ForegroundColor Green
}

# 2. Setup Sleuth Kit Instructions
$TskDir = Join-Path $BinDir "sleuthkit"
if (!(Test-Path (Join-Path $TskDir "bin\fls.exe"))) {
    Write-Host "[2/2] Setting up Sleuth Kit (TSK) placeholder..." -ForegroundColor Yellow
    New-Item -ItemType Directory -Path (Join-Path $TskDir "bin") -Force | Out-Null
    Write-Host "Please place official sleuthkit win32 binaries into: binaries\sleuthkit\bin\" -ForegroundColor Yellow
} else {
    Write-Host "[2/2] Sleuth Kit (TSK) is already installed." -ForegroundColor Green
}

Write-Host "`nSetup complete! Binaries directory is ready." -ForegroundColor Cyan
