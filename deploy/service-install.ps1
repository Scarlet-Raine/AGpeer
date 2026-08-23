# Installs agpeer as a Windows service (NSSM) and starts it.
#
# Requirements:
#   - the one-binary release build (agpeer.exe, built with `--features webui`);
#   - NSSM on PATH (https://nssm.cc/download).
#
# Usage (as Administrator):
#   powershell -ExecutionPolicy Bypass -File .\deploy\service-install.ps1 `
#       -BinaryPath "C:\Program Files\agpeer\agpeer.exe" `
#       -ConfigPath "C:\ProgramData\agpeer\agpeer.toml"
#
# The service restarts on crash (NSSM AppExit Default Restart) and rotates its
# stdout/log captures. agpeer's own rotating file logs stay in <data_dir>\logs.

param(
    [Parameter(Mandatory = $true)]
    [string]$BinaryPath,
    [Parameter(Mandatory = $true)]
    [string]$ConfigPath,
    [string]$ServiceName = "agpeer",
    [string]$DisplayName = "agpeer P2P transfer client"
)

$ErrorActionPreference = "Stop"

if (-not (Get-Command nssm -ErrorAction SilentlyContinue)) {
    throw "nssm not found on PATH. Download from https://nssm.cc and place it on PATH."
}
$exe = Resolve-Path $BinaryPath
if (-not $exe) { throw "binary not found: $BinaryPath" }
$cfg = Resolve-Path $ConfigPath
if (-not $cfg) { throw "config not found: $ConfigPath" }

$appDir = Split-Path -Parent $exe.path

Write-Host "installing service '$ServiceName' ($($exe.path))..."

& nssm install $ServiceName $exe.path "serve" "--config" $cfg.path
if ($LASTEXITCODE -ne 0) { throw "nssm install failed" }

& nssm set $ServiceName DisplayName $DisplayName | Out-Null
& nssm set $ServiceName AppDirectory $appDir | Out-Null
& nssm set $ServiceName AppStdout (Join-Path $appDir "agpeer-service.log") | Out-Null
& nssm set $ServiceName AppStderr (Join-Path $appDir "agpeer-service-err.log") | Out-Null
& nssm set $ServiceName AppRotateFiles 1 | Out-Null
& nssm set $ServiceName AppRotateBytes 10485760 | Out-Null   # 10 MB per capture file
& nssm set $ServiceName Start SERVICE_AUTO_START | Out-Null
& nssm set $ServiceName AppExit Default Restart | Out-Null   # restart on crash
& nssm set $ServiceName AppRestartDelay 5000 | Out-Null

Write-Host "starting service '$ServiceName'..."
& nssm start $ServiceName
if ($LASTEXITCODE -ne 0) { throw "nssm start failed" }

Write-Host "agpeer service '$ServiceName' installed + started."
Write-Host "Manage it with: sc.exe start/stop/delete $ServiceName"