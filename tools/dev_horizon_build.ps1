# Builds Horizon from a clean checkout and installs it as a local developer base, with the SDK native mods
# compile against (Horizon docs: docs/modding/src/how-to/native-mods/07-local-developer-builds.md). Long: run it
# in a visible window. Stops at the first failing step; the log is target\dev-horizon-build.log.
#   powershell -NoExit -File tools\dev_horizon_build.ps1 [-Checkout C:\...\horizon-latest] [-Sdk C:\...\horizon-sdk]
param(
    [string]$Checkout = "C:\Users\latif\Claude\Projects\horizon-latest",
    [string]$Sdk = "C:\Users\latif\Claude\Projects\horizon-sdk",
    [switch]$SkipGame,      # the game programs are already built
    [switch]$SkipInstall    # developer-install already done; only build the SDK
)
$ErrorActionPreference = "Stop"
$log = Join-Path (Split-Path $PSScriptRoot -Parent) "target\dev-horizon-build.log"
Start-Transcript -Path $log -Append | Out-Null
$launcher = Join-Path $env:LOCALAPPDATA "Programs\Horizon Launcher\horizon-launcher.exe"
$started = Get-Date

function Step($name, [scriptblock]$body) {
    $t = Get-Date
    Write-Host ""
    Write-Host ("==> [{0:HH:mm}] {1}" -f $t, $name) -ForegroundColor Cyan
    & $body
    if ($LASTEXITCODE -and $LASTEXITCODE -ne 0) { throw "$name failed (exit $LASTEXITCODE)" }
    Write-Host ("    done in {0:mm\:ss}" -f ((Get-Date) - $t)) -ForegroundColor DarkGray
}

try {
    Set-Location $Checkout
    $commit = (git rev-parse HEAD).Trim()
    Write-Host "Horizon checkout $Checkout at $commit"
    if (git status --porcelain) { throw "The checkout has uncommitted changes; developer-install needs a clean one." }
    $bin = Join-Path $Checkout "target\release"

    if (-not $SkipGame) {
        # As Horizon's CI builds the Windows game: stable Rust (MSVC), release, the official game under its name.
        Step "Build the game, horizon-ctl, horizon-prepare and horizon-server (release, the longest step)" {
            cargo build --release --locked -p horizon-official --bin horizon-official -p horizon-client-lib --bin horizon-ctl -p horizon-prepare --bin horizon-prepare -p horizon-server --bin horizon-server
        }
        Copy-Item (Join-Path $bin "horizon-official.exe") (Join-Path $bin "horizon-client.exe") -Force
        Step "Generate horizon-wasm-std.wasm beside the game" {
            cargo run --locked -p cargo-horizon-mod -- wasm-std (Join-Path $bin "horizon-wasm-std.wasm")
        }
    }

    if (-not $SkipInstall) {
        # The exporters need a Java 25 JDK. Mojang's Java 25 runtime, which the launcher downloads, includes javac,
        # but horizon-prepare only looks in the Minecraft folder's runtimes, so point JAVA_HOME at it.
        $mojang = Get-ChildItem (Join-Path $env:APPDATA "horizon-launcher\minecraft\runtime") -Recurse -Filter javac.exe -ErrorAction SilentlyContinue |
            Where-Object { (& $_.FullName -version 2>&1 | Out-String) -match 'javac 25' } | Select-Object -First 1
        if ($mojang) { $env:JAVA_HOME = Split-Path (Split-Path $mojang.FullName -Parent) -Parent; Write-Host "JAVA_HOME: $env:JAVA_HOME" }
        # compile-exporters reads the pinned jars imported beside the checkout; import them from the Horizon launcher's
        # own linked copy of Minecraft and its server jar when they aren't there yet.
        if (-not (Test-Path (Join-Path $Checkout ".local\minecraft\26.3\manifest.json"))) {
            Step "Import the pinned Minecraft 26.3 files into the checkout (horizon-prepare --prepare)" {
                & (Join-Path $bin "horizon-prepare.exe") --prepare --root $Checkout `
                    --minecraft-dir (Join-Path $env:APPDATA "horizon-launcher\linked-minecraft") `
                    --server-jar (Join-Path $env:APPDATA "horizon-launcher\server-jars\26.3.jar")
            }
        }
        Step "Compile the exporters from the pinned Minecraft jars" {
            $out = & (Join-Path $bin "horizon-prepare.exe") compile-exporters --root $Checkout
            if ($LASTEXITCODE -ne 0) { return }
            $bundle = ($out | Select-Object -Last 1).Trim()
            Write-Host "    bundle: $bundle"
            $dest = Join-Path $Checkout "exporters"
            New-Item -ItemType Directory -Force $dest | Out-Null
            Copy-Item -Recurse -Force (Join-Path $bundle "server"), (Join-Path $bundle "client"), (Join-Path $bundle "manifest.json") $dest
        }
        Step "Install it as a developer base (developer-install)" {
            & $launcher developer-install $Checkout --program (Join-Path $bin "horizon-client.exe") --trust-unsigned
        }
    }

    # The SDK must match the launcher's own Rust (its GNU toolchain with MinGW), so that toolchain goes first on PATH.
    $rust = Get-ChildItem (Join-Path $env:APPDATA "horizon-launcher\rust") -Directory | Where-Object Name -like "*windows-gnu" | Sort-Object Name | Select-Object -Last 1
    if (-not $rust) { throw "The launcher's Rust toolchain is missing: run 'horizon-launcher rust install' first." }
    $env:PATH = "$($rust.FullName)\bin;$($rust.FullName)\mingw64\bin;$env:PATH"
    Write-Host "SDK toolchain: $(rustc --version) ($($rust.Name))"
    Step "Build the SDK native mods compile against (prebuilt_game.py)" {
        python tools/prebuilt_game.py build $Sdk
    }

    Write-Host ""
    Write-Host ("All done in {0:hh\:mm\:ss}." -f ((Get-Date) - $started)) -ForegroundColor Green
    # `versions` lists releases only, and prints notes on stderr, which Windows PowerShell turns into an error here.
    Write-Host "Developer bases installed:"
    Get-ChildItem (Join-Path $env:APPDATA "horizon-launcher\versions") -Directory -Filter "developer-*" | ForEach-Object { "  $($_.Name)" }
    Write-Host "SDK: $Sdk   (native-build with HORIZON_LAUNCHER_DEVELOPER_SDK=$Sdk and --version <developer tag>)"
} catch {
    Write-Host ""
    Write-Host "FAILED: $_" -ForegroundColor Red
} finally {
    Stop-Transcript | Out-Null
}
