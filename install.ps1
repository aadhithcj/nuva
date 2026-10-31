<#
  Installs (or removes) your personal Nuva build for the current user. No admin rights needed.

    .\install.ps1                 build if needed, install, start with Windows, launch
    .\install.ps1 -NoStartup      same, without the sign-in autostart
    .\install.ps1 -Uninstall      remove the app, shortcut and autostart (keeps your data)
    .\install.ps1 -Uninstall -RemoveData   also delete tasks, water log, shelf and settings
#>
param([switch]$Uninstall, [switch]$NoStartup, [switch]$RemoveData)
$ErrorActionPreference = 'Stop'

$dest     = Join-Path $env:LOCALAPPDATA 'Programs\Nuva'
$exe      = Join-Path $dest 'nuva.exe'
$shortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Nuva.lnk'
$runKey   = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'

function Stop-Nuva {
    if (-not (Get-Process nuva, nusa -ErrorAction SilentlyContinue)) { return }
    # Ask politely first so the tray icon is removed, then make sure it's gone.
    try { & taskkill.exe /IM nuva.exe *> $null } catch {}
    try { & taskkill.exe /IM nusa.exe *> $null } catch {}
    Start-Sleep -Milliseconds 800
    Get-Process nuva, nusa -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}

if ($Uninstall) {
    Stop-Nuva
    Remove-ItemProperty -Path $runKey -Name 'Nuva' -ErrorAction SilentlyContinue
    Remove-ItemProperty -Path $runKey -Name 'Nusa' -ErrorAction SilentlyContinue
    Remove-Item $shortcut -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Nusa.lnk') -ErrorAction SilentlyContinue
    Remove-Item $dest -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $env:LOCALAPPDATA 'Programs\Nusa') -Recurse -Force -ErrorAction SilentlyContinue
    if ($RemoveData) {
        Remove-Item (Join-Path $env:APPDATA 'Nuva') -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item (Join-Path $env:LOCALAPPDATA 'Nuva') -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item (Join-Path $env:APPDATA 'Nusa') -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item (Join-Path $env:LOCALAPPDATA 'Nusa') -Recurse -Force -ErrorAction SilentlyContinue
        Write-Host 'Nuva and its data were removed.'
    } else {
        Write-Host 'Nuva was removed. Your data is still in %APPDATA%\Nuva and %LOCALAPPDATA%\Nuva.'
    }
    return
}

$built = Join-Path $PSScriptRoot 'target\release\nuva.exe'
if (-not (Test-Path $built)) {
    Write-Host 'Building release...'
    Push-Location $PSScriptRoot
    try { & cargo build --release } finally { Pop-Location }
    if (-not (Test-Path $built)) { throw 'Build failed: target\release\nuva.exe not found.' }
}

Stop-Nuva
New-Item -ItemType Directory -Force $dest | Out-Null
Copy-Item $built $exe -Force

$ws = New-Object -ComObject WScript.Shell
$lnk = $ws.CreateShortcut($shortcut)
$lnk.TargetPath = $exe
$lnk.WorkingDirectory = $dest
$lnk.Description = 'A living notch for Windows'
$lnk.Save()

# Clean up legacy Nusa shortcut/run key if present
Remove-ItemProperty -Path $runKey -Name 'Nusa' -ErrorAction SilentlyContinue
Remove-Item (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Nusa.lnk') -ErrorAction SilentlyContinue

if ($NoStartup) {
    Remove-ItemProperty -Path $runKey -Name 'Nuva' -ErrorAction SilentlyContinue
} else {
    Set-ItemProperty -Path $runKey -Name 'Nuva' -Value "`"$exe`""
}

Start-Process $exe
Write-Host "Installed to $dest and launched. Right-click the tray icon for settings."
