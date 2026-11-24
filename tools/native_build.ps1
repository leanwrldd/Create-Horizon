# Stages the package and builds it into a Horizon instance. By default it builds on the official nightly the manifest
# pins (mod/horizon-mod.toml [native] horizon), whose SDK the launcher downloads. For the home-built developer version,
# pass both -Version developer-<hash> and -Sdk <folder of the local SDK>.
#   powershell -File tools\native_build.ps1 [-Instance create-new] [-Version nightly-...] [-Sdk C:\...\horizon-sdk]
# A build takes 15 to 30 minutes, a refusal about 10: run it in its own window and leave it alone
#   Start-Process powershell -ArgumentList '-NoExit','-File','tools\native_build.ps1' -PassThru
param(
    [string]$Instance = "create-new",
    [string]$Version = "nightly-2026-11-22-f1bb66f",
    [string]$Sdk = ""
)

$root = Split-Path $PSScriptRoot -Parent
$launcher = Join-Path $env:LOCALAPPDATA "Programs\Horizon Launcher\horizon-launcher.exe"
Set-Location $root
# The module lists and the Parts follow the files that exist, then the package is staged with Create's art.
python tools\gen_modules.py | Out-Null
python tools\gen_forward.py
python tools\gen_parts.py
python tools\stage_package.py
if ($LASTEXITCODE -ne 0) { Write-Host "staging failed"; exit 1 }
# A package takes only the registry lock as a Cargo.lock; `cargo test` leaves others behind.
Get-ChildItem (Join-Path $root "target\package") -Recurse -Filter Cargo.lock |
    Where-Object { $_.DirectoryName -ne (Join-Path $root "target\package") } | Remove-Item -Force
if ($Sdk) { $env:HORIZON_LAUNCHER_DEVELOPER_SDK = $Sdk } else { Remove-Item Env:HORIZON_LAUNCHER_DEVELOPER_SDK -ErrorAction SilentlyContinue }
$done = Join-Path $root "target\native-build.exit"
Remove-Item $done -ErrorAction SilentlyContinue
$started = Get-Date
$buildArgs = @("native-build", "`"$(Join-Path $root 'target\package')`"", "--instance", $Instance,
               "--version", $Version, "--trust-native-code")
# `& $launcher` returns before the launcher finishes; Start-Process -Wait waits for it and what it starts.
$process = Start-Process -FilePath $launcher -NoNewWindow -Wait -PassThru -ArgumentList $buildArgs
$code = $process.ExitCode
Set-Content -Path $done -Value $code -Encoding ascii
$took = (Get-Date) - $started
Write-Host ("native-build exited with code {0} after {1:mm\:ss}" -f $code, $took)
Start-Sleep -Seconds 20   # leave the result on screen a moment before the window closes
