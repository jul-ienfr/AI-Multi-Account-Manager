@echo off
:: =========================================================
:: Ajoute la regle pare-feu pour AI Manager P2P Sync (port 9090)
:: Cliquez droit → Executer en tant qu'administrateur
:: =========================================================

net session >nul 2>&1
if %errorlevel% neq 0 (
    echo Droits administrateur requis.
    echo Cliquez droit sur ce fichier et choisissez "Executer en tant qu'administrateur".
    pause
    exit /b 1
)

netsh advfirewall firewall show rule name="AI Manager Sync" >nul 2>&1
if %errorlevel% equ 0 (
    echo La regle "AI Manager Sync" existe deja.
) else (
    netsh advfirewall firewall add rule name="AI Manager Sync" dir=in action=allow protocol=TCP localport=9090
    echo Regle ajoutee avec succes.
)

pause
