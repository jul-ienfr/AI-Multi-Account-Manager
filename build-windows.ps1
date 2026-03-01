# =========================================================
# AI Manager v3 — Build Windows (PowerShell)
# Exécutez en tant qu'administrateur :
#   powershell -ExecutionPolicy Bypass -File "build-windows.ps1"
# =========================================================

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$Bin = Join-Path $Root "target\release\ai-manager.exe"

Write-Host ""
Write-Host "  ╔══════════════════════════════════════════════════╗" -ForegroundColor Cyan
Write-Host "  ║         AI Manager v3 — Build Windows            ║" -ForegroundColor Cyan
Write-Host "  ╚══════════════════════════════════════════════════╝" -ForegroundColor Cyan
Write-Host ""

# ---- ETAPE 1 : Visual Studio Build Tools ----
$hasCompiler = $null -ne (Get-Command "cl.exe" -ErrorAction SilentlyContinue)
if (-not $hasCompiler) {
    # Vérifier via vswhere
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (Test-Path $vswhere) {
        $vsPath = & $vswhere -latest -property installationPath 2>$null
        if ($vsPath) { $hasCompiler = $true }
    }
}

if (-not $hasCompiler) {
    Write-Host "[1/4] Installation de Visual Studio Build Tools..." -ForegroundColor Yellow

    $hasWinget = $null -ne (Get-Command "winget" -ErrorAction SilentlyContinue)
    if ($hasWinget) {
        winget install Microsoft.VisualStudio.2022.BuildTools `
            --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended" `
            --accept-source-agreements --accept-package-agreements
    } else {
        Write-Host "  winget indisponible. Telechargement direct..." -ForegroundColor Yellow
        $installerUrl = "https://aka.ms/vs/17/release/vs_BuildTools.exe"
        $installerPath = Join-Path $env:TEMP "vs_BuildTools.exe"
        Invoke-WebRequest -Uri $installerUrl -OutFile $installerPath
        Start-Process -FilePath $installerPath -ArgumentList `
            "--quiet", "--wait", "--add", "Microsoft.VisualStudio.Workload.VCTools", "--includeRecommended" `
            -Wait -NoNewWindow
    }
    Write-Host "[OK] Build Tools installes." -ForegroundColor Green
} else {
    Write-Host "[OK] Visual Studio Build Tools deja installe." -ForegroundColor Green
}

# ---- ETAPE 2 : Rust ----
$hasRust = $null -ne (Get-Command "rustc" -ErrorAction SilentlyContinue)
if (-not $hasRust) {
    # Vérifier dans le chemin par défaut
    $cargoPath = Join-Path $env:USERPROFILE ".cargo\bin"
    if (Test-Path (Join-Path $cargoPath "rustc.exe")) {
        $env:PATH = "$cargoPath;$env:PATH"
        $hasRust = $true
    }
}

if (-not $hasRust) {
    Write-Host "[2/4] Installation de Rust..." -ForegroundColor Yellow

    $hasWinget = $null -ne (Get-Command "winget" -ErrorAction SilentlyContinue)
    if ($hasWinget) {
        winget install Rustlang.Rustup --accept-source-agreements --accept-package-agreements
    } else {
        $rustupUrl = "https://win.rustup.rs/x86_64"
        $rustupPath = Join-Path $env:TEMP "rustup-init.exe"
        Invoke-WebRequest -Uri $rustupUrl -OutFile $rustupPath
        Start-Process -FilePath $rustupPath -ArgumentList "-y", "--default-toolchain", "stable" -Wait -NoNewWindow
    }

    $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

    if (-not (Get-Command "rustc" -ErrorAction SilentlyContinue)) {
        Write-Host "[!] Rust non detecte. Fermez et rouvrez PowerShell, puis relancez." -ForegroundColor Red
        Read-Host "Appuyez sur Entree"
        exit 1
    }
    Write-Host "[OK] Rust installe." -ForegroundColor Green
} else {
    Write-Host "[OK] Rust deja installe." -ForegroundColor Green
}

# ---- ETAPE 3 : Node.js ----
$hasNode = $null -ne (Get-Command "node" -ErrorAction SilentlyContinue)
if (-not $hasNode) {
    Write-Host "[3/4] Installation de Node.js LTS..." -ForegroundColor Yellow

    $hasWinget = $null -ne (Get-Command "winget" -ErrorAction SilentlyContinue)
    if ($hasWinget) {
        winget install OpenJS.NodeJS.LTS --accept-source-agreements --accept-package-agreements
    } else {
        $nodeUrl = "https://nodejs.org/dist/v22.15.0/node-v22.15.0-x64.msi"
        $nodePath = Join-Path $env:TEMP "node-install.msi"
        Invoke-WebRequest -Uri $nodeUrl -OutFile $nodePath
        Start-Process msiexec -ArgumentList "/i", $nodePath, "/qn" -Wait -NoNewWindow
    }

    # Recharger PATH depuis le registre
    $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $env:PATH = "$machinePath;$userPath"

    if (-not (Get-Command "node" -ErrorAction SilentlyContinue)) {
        Write-Host "[!] Node.js non detecte. Fermez et rouvrez PowerShell, puis relancez." -ForegroundColor Red
        Read-Host "Appuyez sur Entree"
        exit 1
    }
    Write-Host "[OK] Node.js installe." -ForegroundColor Green
} else {
    Write-Host "[OK] Node.js deja installe." -ForegroundColor Green
}

# ---- ETAPE 4 : Build ----
Write-Host ""
Write-Host "[4/4] Compilation de AI Manager v3..." -ForegroundColor Yellow
Write-Host "       (premiere compilation : ~5-10 minutes)" -ForegroundColor Gray
Write-Host ""

Set-Location (Join-Path $Root "tauri-app")

# npm install si nécessaire
if (-not (Test-Path "node_modules")) {
    Write-Host "       Installation des dependances npm..."
    npm install --legacy-peer-deps
}

# Build Tauri
Write-Host "       Compilation Rust + Frontend..."
npx tauri build

if (Test-Path $Bin) {
    Write-Host ""
    Write-Host "  ══════════════════════════════════════════════" -ForegroundColor Green
    Write-Host "    AI Manager v3 compile avec succes !" -ForegroundColor Green
    Write-Host "    Binaire : $Bin" -ForegroundColor Green
    Write-Host "  ══════════════════════════════════════════════" -ForegroundColor Green
    Write-Host ""

    # Lister tous les artefacts
    $msi = Get-ChildItem -Path (Join-Path $Root "target\release\bundle\msi") -Filter "*.msi" -ErrorAction SilentlyContinue | Select-Object -First 1
    $nsis = Get-ChildItem -Path (Join-Path $Root "target\release\bundle\nsis") -Filter "*.exe" -ErrorAction SilentlyContinue | Select-Object -First 1

    if ($msi) { Write-Host "    MSI : $($msi.FullName)" -ForegroundColor Cyan }
    if ($nsis) { Write-Host "    Setup : $($nsis.FullName)" -ForegroundColor Cyan }

    Write-Host ""
    Write-Host "  Lancement..." -ForegroundColor Green
    Start-Process $Bin
} else {
    Write-Host "[!] Binaire introuvable apres compilation." -ForegroundColor Red
    Read-Host "Appuyez sur Entree"
    exit 1
}
