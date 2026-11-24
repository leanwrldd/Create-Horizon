# Installs a test build of Create for Horizon into an instance of its own, then builds it.
#
#   powershell -ExecutionPolicy Bypass -File install.ps1
#   powershell -ExecutionPolicy Bypass -File install.ps1 -Instance my-create -Release v0.1.0-test3
#   powershell -ExecutionPolicy Bypass -File install.ps1 -File C:\path\to\create.hmod     (a local copy instead)
#
# Needs the Horizon Launcher (https://playhorizon.dev), signed in with an account that owns Minecraft: Java Edition.
# The mod is Rust source: the launcher compiles it on this computer. The first build downloads Rust and takes about
# 20 to 30 minutes and a few GB; later ones are quicker.
param(
    [string]$Instance = "create-test",
    [string]$Release = "v0.1.0-test3",
    [string]$File = ""
)

$ErrorActionPreference = "Stop"
$Repo = "https://github.com/leanwrldd/Create-Horizon"
# The Horizon this build is made for. The mod pins the exact game build (horizon-mod.toml [native]); this nightly
# provides the game files the build reuses.
$Horizon = "nightly-2026-11-22-f1bb66f"

$launcher = Join-Path $env:LOCALAPPDATA "Programs\Horizon Launcher\horizon-launcher.exe"
if (-not (Test-Path $launcher)) {
    Write-Host "The Horizon Launcher isn't installed. Get it from https://playhorizon.dev, open it once and sign in,"
    Write-Host "then run this script again."
    exit 1
}

# Start-Process passes arguments as one string: quote the ones with spaces.
function Invoke-Launcher {
    param([string[]]$Arguments, [switch]$AllowFailure)
    $quoted = $Arguments | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } }
    Write-Host ">> horizon-launcher $($quoted -join ' ')" -ForegroundColor DarkGray
    $p = Start-Process -FilePath $launcher -ArgumentList $quoted -NoNewWindow -Wait -PassThru
    if ($p.ExitCode -ne 0 -and -not $AllowFailure) {
        throw "horizon-launcher $($Arguments[0]) failed (exit code $($p.ExitCode)). The messages above say why."
    }
    return $p.ExitCode
}

function Get-InstanceHome {
    param([string]$Name)
    $out = Join-Path $env:TEMP "create-horizon-instances.json"
    $p = Start-Process -FilePath $launcher -ArgumentList "instance", "list", "--json" -NoNewWindow -Wait -PassThru `
        -RedirectStandardOutput $out
    if ($p.ExitCode -ne 0) { throw "Couldn't list the launcher's instances." }
    $list = Get-Content $out -Raw | ConvertFrom-Json
    $found = $list | Where-Object { $_.id -eq $Name -or $_.name -eq $Name } | Select-Object -First 1
    if ($found) { return $found.home }
    return $null
}

Write-Host ""
Write-Host "Create for Horizon: test build" -ForegroundColor Cyan
Write-Host ""
Write-Host "  - This is a test build. Expect bugs, and keep it away from worlds you care about: it goes into its own"
Write-Host "    instance, '$Instance'. A world opened later without the mod loses its Create blocks."
Write-Host "  - It's a native mod: Rust code that runs with your rights, outside any sandbox, and isn't reviewed by"
Write-Host "    the Horizon registry. The source is at $Repo."
Write-Host "  - The first build downloads Rust and takes about 20 to 30 minutes."
Write-Host ""
$answer = Read-Host "Install it? Type yes to go on"
if ($answer -ne "yes") { Write-Host "Nothing was installed."; exit 0 }

# 1. The Horizon version, with resources prepared from your own Minecraft.
Invoke-Launcher @("install", $Horizon) -AllowFailure | Out-Null
Invoke-Launcher @("prepare", "--version", $Horizon) | Out-Null

# 2. An instance of its own, kept on that version.
if (-not (Get-InstanceHome $Instance)) {
    Invoke-Launcher @("instance", "create", $Instance) | Out-Null
}
Invoke-Launcher @("instance", "version", $Instance, $Horizon) | Out-Null
$home_ = Get-InstanceHome $Instance
if (-not $home_) { throw "The instance '$Instance' wasn't found after creating it." }

# 3. The mod: from the GitHub release (its checksum checked by the launcher), or a local file.
if ($File) {
    if (-not (Test-Path $File)) { throw "No file at $File" }
    $mods = Join-Path $home_ "horizon-native-mods"
    New-Item -ItemType Directory -Force $mods | Out-Null
    Copy-Item $File (Join-Path $mods "create.hmod") -Force
    $list = Join-Path $home_ "horizon-mods.txt"
    $lines = @()
    if (Test-Path $list) { $lines = @(Get-Content $list | Where-Object { $_ -notmatch '^mod\.create:' }) }
    $lines += "mod.create:true"
    Set-Content -Path $list -Value $lines -Encoding utf8
    Write-Host "Copied $File into $mods"
} else {
    Invoke-Launcher @("mods", "add-github", "$Repo/releases/tag/$Release", "--instance", $Instance,
                      "--trust-native-code") | Out-Null
}

# 4. Compile it for the instance.
Write-Host ""
Write-Host "Building. The first time takes about 20 to 30 minutes; leave this window open." -ForegroundColor Cyan
$started = Get-Date
Invoke-Launcher @("native-build", "--instance", $Instance, "--version", $Horizon, "--trust-native-code") | Out-Null
$took = (Get-Date) - $started

Write-Host ""
Write-Host ("Done in {0:mm\:ss}. Play it from the launcher (instance '{1}'), or with:" -f $took, $Instance) -ForegroundColor Green
Write-Host "  & `"$launcher`" play --instance $Instance --world create-test --creative"
Write-Host ""
Write-Host "Found a bug? Report it at $Repo/issues/new/choose"
Write-Host "Your log is in $(Join-Path $home_ 'logs\latest.log')"
