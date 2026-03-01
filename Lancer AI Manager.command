#!/bin/bash
# =========================================================
# AI Manager v3 — Lanceur macOS tout-en-un
# Double-cliquez dans le Finder pour lancer.
# Installe tout automatiquement si nécessaire.
# =========================================================

set -e
cd "$(dirname "$0")"
ROOT="$(pwd)"

BIN="$ROOT/target/release/ai-manager"
APP_BUNDLE="$ROOT/target/release/bundle/macos/AI Manager.app"

# --- Si le .app ou binaire existe, lancer directement ---
if [ -d "$APP_BUNDLE" ]; then
    open "$APP_BUNDLE"
    exit 0
fi

if [ -f "$BIN" ] && [ -x "$BIN" ]; then
    open "$BIN"
    exit 0
fi

# --- Premier lancement : installation + compilation ---
echo ""
echo "╔══════════════════════════════════════════════════╗"
echo "║         AI Manager v3 — Premier lancement       ║"
echo "║                                                  ║"
echo "║  L'application va être installée et compilée.    ║"
echo "║  Cela peut prendre 10-15 minutes.                ║"
echo "╚══════════════════════════════════════════════════╝"
echo ""

notify() {
    osascript -e "display notification \"$1\" with title \"AI Manager\"" 2>/dev/null || true
}

alert_error() {
    osascript -e "display alert \"Erreur\" message \"$1\" as critical" 2>/dev/null
    echo "[!] $1"
}

# ===========================================
# ETAPE 1 : Xcode Command Line Tools
# ===========================================
if ! xcode-select -p &>/dev/null; then
    echo "[1/4] Installation de Xcode Command Line Tools..."
    xcode-select --install
    echo "       Attendez que l'installation se termine, puis relancez ce script."
    osascript -e 'display alert "Xcode CLT requis" message "Installez les Xcode Command Line Tools dans la fenêtre qui s'\''est ouverte, puis relancez ce script." as informational'
    exit 1
else
    echo "[OK] Xcode Command Line Tools installé."
fi

# ===========================================
# ETAPE 2 : Homebrew (si absent)
# ===========================================
if ! command -v brew &>/dev/null; then
    echo "[2/4] Installation de Homebrew..."
    /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

    # Ajouter brew au PATH (Apple Silicon vs Intel)
    if [ -f /opt/homebrew/bin/brew ]; then
        eval "$(/opt/homebrew/bin/brew shellenv)"
    elif [ -f /usr/local/bin/brew ]; then
        eval "$(/usr/local/bin/brew shellenv)"
    fi

    if ! command -v brew &>/dev/null; then
        alert_error "Homebrew n'a pas été détecté après installation. Relancez ce script."
        exit 1
    fi
    echo "[OK] Homebrew installé."
else
    echo "[OK] Homebrew déjà installé."
fi

# ===========================================
# ETAPE 3 : Rust (via rustup)
# ===========================================
if ! command -v rustc &>/dev/null; then
    echo "[3/4] Installation de Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    source "$HOME/.cargo/env"

    if ! command -v rustc &>/dev/null; then
        alert_error "Rust n'a pas été détecté après installation. Relancez ce script."
        exit 1
    fi
    echo "[OK] Rust installé."
else
    echo "[OK] Rust déjà installé."
fi

# ===========================================
# ETAPE 4 : Node.js (via brew)
# ===========================================
if ! command -v node &>/dev/null; then
    echo "[3/4] Installation de Node.js..."
    brew install node
    if ! command -v node &>/dev/null; then
        alert_error "Node.js n'a pas été détecté après installation."
        exit 1
    fi
    echo "[OK] Node.js installé."
else
    echo "[OK] Node.js déjà installé."
fi

# ===========================================
# ETAPE 5 : Compilation AI Manager
# ===========================================
echo "[4/4] Compilation de AI Manager v3..."
echo "       (première compilation : ~5-10 minutes)"
echo ""
notify "Compilation en cours... (5-10 minutes)"

cd "$ROOT/tauri-app"

# Installer les dépendances npm
if [ ! -d "node_modules" ]; then
    echo "       Installation des dépendances npm..."
    npm install --legacy-peer-deps
fi

# Compiler avec Tauri
echo "       Compilation Rust + Frontend..."
npx tauri build

# --- Lancement ---
if [ -d "$APP_BUNDLE" ]; then
    echo ""
    echo "══════════════════════════════════════════════"
    echo "  AI Manager v3 installé avec succès !"
    echo "  Lancement..."
    echo "══════════════════════════════════════════════"
    notify "Installation terminée ! Lancement..."
    open "$APP_BUNDLE"
elif [ -f "$BIN" ]; then
    notify "Installation terminée !"
    open "$BIN"
else
    alert_error "Binaire introuvable après compilation."
    exit 1
fi
