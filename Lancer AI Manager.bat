@echo off
setlocal EnableDelayedExpansion
chcp 65001 >nul 2>&1
title AI Manager v3

:: =========================================================
:: AI Manager v3 — Lanceur Windows tout-en-un
:: Double-cliquez pour lancer. Installe tout automatiquement.
:: =========================================================

set "ROOT=%~dp0"
set "BIN=%ROOT%target\release\ai-manager.exe"

:: --- Si le binaire existe, lancer directement ---
if exist "%BIN%" (
    start "" "%BIN%"
    exit /b 0
)

:: --- Premier lancement ---
echo.
echo  ================================================================
echo    AI Manager v3 -- Premier lancement
echo.
echo    L'application va etre installee et compilee automatiquement.
echo    Cela peut prendre 10-15 minutes.
echo  ================================================================
echo.

:: ===========================================
:: ETAPE 1 : Visual Studio C++ (MSVC)
:: ===========================================
echo [1/4] Verification de Visual Studio C++...

set "HAS_MSVC=0"
set "VSCHECK=%TEMP%\ai-manager-vscheck.txt"

:: Essai 1 : Program Files (x86)
cmd /c ""%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath" >"%VSCHECK%" 2>nul
for /f "usebackq delims=" %%i in ("%VSCHECK%") do if not "%%i"=="" set "HAS_MSVC=1"

:: Essai 2 : Program Files (64-bit, VS 2025+)
if "!HAS_MSVC!"=="0" (
    "%ProgramFiles%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath >"%VSCHECK%" 2>nul
    for /f "usebackq delims=" %%i in ("%VSCHECK%") do if not "%%i"=="" set "HAS_MSVC=1"
)

:: Essai 3 : Chercher les dossiers VS directement
if "!HAS_MSVC!"=="0" (
    for %%V in (18 17 16 15) do (
        if exist "%ProgramFiles%\Microsoft Visual Studio\%%V\Community\VC\Tools\MSVC" set "HAS_MSVC=1"
        if exist "%ProgramFiles%\Microsoft Visual Studio\%%V\BuildTools\VC\Tools\MSVC" set "HAS_MSVC=1"
    )
)
del "%VSCHECK%" 2>nul

if "!HAS_MSVC!"=="1" (
    echo       [OK] Deja installe.
    goto :step2
)

echo       C++ Build Tools non detectes. Installation necessaire.
echo.
echo       Ouverture de la page de telechargement...
echo       1. Cliquez "Telecharger Build Tools"
echo       2. Cochez "Developpement Desktop en C++"
echo       3. Cliquez "Installer"
echo       4. Une fois termine, revenez ici et appuyez sur une touche.
echo.
start "" "https://visualstudio.microsoft.com/visual-cpp-build-tools/"
pause

:: Re-verifier
set "HAS_MSVC=0"
cmd /c ""%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -property installationPath" >"%VSCHECK%" 2>nul
for /f "usebackq delims=" %%i in ("%VSCHECK%") do if not "%%i"=="" set "HAS_MSVC=1"
del "%VSCHECK%" 2>nul

if "!HAS_MSVC!"=="0" (
    echo       [!] Toujours pas detecte. Fermez tout et relancez apres installation.
    pause
    exit /b 1
)
echo       [OK] Build Tools detectes.

:step2
echo.

:: ===========================================
:: ETAPE 2 : Rust
:: ===========================================
echo [2/4] Verification de Rust...

set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

:: Verifier si rustc.exe existe directement (plus fiable que 'where' dans les blocs batch)
if exist "%USERPROFILE%\.cargo\bin\rustc.exe" goto :rust_ok

where rustc >nul 2>&1
if %errorlevel% equ 0 goto :rust_ok

:: --- Rust absent, installation ---
echo       Installation de Rust...
echo       Telechargement de rustup-init.exe...

set "RUSTUP=%TEMP%\rustup-init.exe"
"%SystemRoot%\System32\curl.exe" -sSfLo "%RUSTUP%" "https://win.rustup.rs/x86_64"
if %errorlevel% neq 0 (
    echo       [!] Echec du telechargement de Rust. Verifiez votre connexion.
    pause
    exit /b 1
)

echo       Execution de rustup-init...
"%RUSTUP%" -y --default-toolchain stable
if %errorlevel% neq 0 (
    echo       [!] Installation de Rust echouee.
    pause
    exit /b 1
)

:: Ajouter cargo au PATH et verifier
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
if exist "%USERPROFILE%\.cargo\bin\rustc.exe" (
    echo       [OK] Rust installe.
    goto :step3
)

echo       [!] Rust non detecte apres installation.
echo       Fermez cette fenetre, ouvrez un nouveau terminal, et relancez.
pause
exit /b 1

:rust_ok
for /f "tokens=*" %%v in ('rustc --version') do echo       [OK] %%v

:step3
echo.

:: ===========================================
:: ETAPE 3 : Node.js
:: ===========================================
echo [3/4] Verification de Node.js...

:: Recharger PATH depuis le registre + chemins connus
for /f "tokens=2*" %%A in ('reg query "HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment" /v Path 2^>nul') do set "SYS_PATH=%%B"
for /f "tokens=2*" %%A in ('reg query "HKCU\Environment" /v Path 2^>nul') do set "USR_PATH=%%B"
set "PATH=%USERPROFILE%\.cargo\bin;C:\Program Files\nodejs;%SYS_PATH%;%USR_PATH%"

where node >nul 2>&1
if %errorlevel% equ 0 goto :node_ok

:: --- Node absent, installation ---
echo       Installation de Node.js LTS...

set "NODE_MSI=%TEMP%\node-install.msi"
"%SystemRoot%\System32\curl.exe" -sSfLo "%NODE_MSI%" "https://nodejs.org/dist/v22.15.0/node-v22.15.0-x64.msi"
if %errorlevel% neq 0 (
    echo       [!] Echec du telechargement. Verifiez votre connexion.
    pause
    exit /b 1
)

echo       Installation silencieuse...
msiexec /i "%NODE_MSI%" /qn

:: Recharger PATH
for /f "tokens=2*" %%A in ('reg query "HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment" /v Path 2^>nul') do set "SYS_PATH=%%B"
set "PATH=%USERPROFILE%\.cargo\bin;%SYS_PATH%;%USR_PATH%"

where node >nul 2>&1
if %errorlevel% equ 0 goto :node_installed

echo       [!] Node.js non detecte. Fermez et relancez.
pause
exit /b 1

:node_installed
echo       [OK] Node.js installe.
goto :step4

:node_ok
for /f "tokens=*" %%v in ('node --version') do echo       [OK] Node.js %%v

:step4
echo.

:: ===========================================
:: ETAPE 4 : Compilation
:: ===========================================
echo [4/4] Compilation de AI Manager v3...
echo       (premiere compilation ~5-10 min)
echo.

cd /d "%ROOT%tauri-app"

if not exist "node_modules" (
    echo       npm install...
    call npm install --legacy-peer-deps
    if !errorlevel! neq 0 (
        echo       [!] npm install a echoue.
        pause
        exit /b 1
    )
    echo.
)

echo       Compilation Rust + Frontend en cours...
echo.
call npx tauri build

if !errorlevel! neq 0 (
    echo.
    echo  [!] Compilation echouee. Si Rust/Build Tools viennent d'etre
    echo      installes, fermez TOUT et relancez ce fichier.
    pause
    exit /b 1
)

:: --- Succes ---
if exist "%BIN%" (
    echo.
    echo  ================================================================
    echo    AI Manager v3 compile avec succes !
    echo  ================================================================
    echo.
    echo  Binaire : %BIN%
    for %%f in ("%ROOT%target\release\bundle\msi\*.msi") do echo  MSI    : %%f
    for %%f in ("%ROOT%target\release\bundle\nsis\*.exe") do echo  Setup  : %%f
    echo.
    timeout /t 3 >nul
    start "" "%BIN%"
) else (
    echo  [!] Binaire introuvable apres compilation.
    pause
)

endlocal
