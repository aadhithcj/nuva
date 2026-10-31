<#
.SYNOPSIS
    Builds Nuva and packages the standalone NuvaSetup.exe installer.

.DESCRIPTION
    1. Compiles the release binary using cargo (`cargo build --release`).
    2. Embeds version metadata and the app icon using rcedit.
    3. Compiles the modern installer using NSIS (`makensis.exe`).

.EXAMPLE
    .\build-installer.ps1
    .\build-installer.ps1 -NoBuild
#>
[CmdletBinding()]
param(
    [switch]$NoBuild
)

$ErrorActionPreference = 'Stop'
$rootDir = $PSScriptRoot

Write-Host "=== Building Nuva Setup Bundle ===" -ForegroundColor Cyan

# 1. Compile Nuva
$exePath = Join-Path $rootDir "target\release\nuva.exe"
if (-not $NoBuild -or -not (Test-Path $exePath)) {
    Write-Host "[1/3] Building release binary with Cargo..." -ForegroundColor Yellow
    Push-Location $rootDir
    try {
        & cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "Cargo build failed." }
    } finally {
        Pop-Location
    }
} else {
    Write-Host "[1/3] Skipping build (-NoBuild specified and binary exists)..." -ForegroundColor DarkGray
}

if (-not (Test-Path $exePath)) {
    throw "Binary not found at: $exePath"
}

# 2. Stamp metadata & icon with rcedit if available
$iconPath = Join-Path $rootDir "assets\app.ico"
$rcedit = Get-Command rcedit.exe -ErrorAction SilentlyContinue
if ($rcedit -and (Test-Path $iconPath)) {
    Write-Host "[2/3] Embedding version metadata and icon into nuva.exe..." -ForegroundColor Yellow
    & $rcedit.Source $exePath `
        --set-icon $iconPath `
        --set-file-version "1.2.0" `
        --set-product-version "1.2.0" `
        --set-version-string "ProductName" "Nuva" `
        --set-version-string "FileDescription" "Nuva - A living notch for Windows" `
        --set-version-string "LegalCopyright" "Copyright (C) 2026 Nuva" `
        --set-version-string "OriginalFilename" "nuva.exe"
} else {
    Write-Host "[2/3] Skipping rcedit (rcedit not found in PATH or icon missing)..." -ForegroundColor DarkGray
}

# 3. Compile NSIS installer
$makensis = Get-Command makensis.exe -ErrorAction SilentlyContinue
if (-not $makensis) {
    throw "NSIS (makensis.exe) not found in PATH. Please install it (e.g. scoop install nsis)."
}

Write-Host "[3/3] Compiling installer with NSIS..." -ForegroundColor Yellow
Push-Location $rootDir
try {
    & $makensis.Source (Join-Path $rootDir "installer.nsi")
    if ($LASTEXITCODE -ne 0) { throw "makensis compilation failed." }
} finally {
    Pop-Location
}

$setupPath = Join-Path $rootDir "NuvaSetup.exe"
if (Test-Path $setupPath) {
    $item = Get-Item $setupPath
    $sizeKb = [math]::Round($item.Length / 1KB, 1)
    Write-Host "`nSuccessfully created: $setupPath ($sizeKb KB)" -ForegroundColor Green
} else {
    throw "Setup executable was not created."
}

