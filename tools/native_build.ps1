# Builds the native mod into a Horizon instance (default: the disposable create-dev one).
#   powershell -File tools\native_build.ps1 [-Instance create-dev] [-Profile dev]
param([string]$Instance = "create-dev", [ValidateSet("performance", "dev", "release")][string]$Profile = "dev")

$launcher = Join-Path $env:LOCALAPPDATA "Programs\Horizon Launcher\horizon-launcher.exe"
$native = Join-Path (Split-Path $PSScriptRoot -Parent) "native"
# The module lists follow the files that exist.
python (Join-Path $PSScriptRoot "gen_modules.py")
# A bundle may not ship a Cargo.lock (it builds against Horizon's); `cargo test` leaves one behind.
Get-ChildItem $native -Recurse -Filter Cargo.lock | Remove-Item -Force
$done = Join-Path (Split-Path $PSScriptRoot -Parent) "target\native-build.exit"
Remove-Item $done -ErrorAction SilentlyContinue
$started = Get-Date
$buildArgs = @("native-build", "`"$native`"", "--instance", $Instance, "--trust-native-code", "--profile", $Profile)
# `& $launcher` returns before the launcher finishes, with no exit code. Start-Process -Wait waits
# for it and for any process it starts, and -NoNewWindow keeps its progress in this window.
$process = Start-Process -FilePath $launcher -NoNewWindow -Wait -PassThru `
    -ArgumentList $buildArgs
$code = $process.ExitCode
Set-Content -Path $done -Value $code -Encoding ascii
$took = (Get-Date) - $started
Write-Host ("native-build exited with code {0} after {1:mm\:ss}" -f $code, $took)
