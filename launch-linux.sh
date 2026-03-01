#!/bin/bash
# =========================================================
# AI Manager v3 — Lanceur Linux tout-en-un
# ./launch-linux.sh              → lancer l'app
# ./launch-linux.sh --install-desktop  → créer raccourci bureau
# Installe tout automatiquement si nécessaire.
# =========================================================

set -e
cd "$(dirname "$(readlink -f "$0")")"
ROOT="$(pwd)"

BIN="$ROOT/target/release/ai-manager"
APPIMAGE=$(find "$ROOT/target/release/bundle/appimage" -name "*.AppImage" 2>/dev/null | head -1)

# --- Mode raccourci bureau ---
if [ "$1" = "--install-desktop" ]; then
    DESKTOP_DIR="$HOME/.local/share/applications"
    ICON_DIR="$HOME/.local/share/icons/hicolor/128x128/apps"
    ICON_SRC="$ROOT/tauri-app/src-tauri/icons/128x128.png"

    mkdir -p "$DESKTOP_DIR" "$ICON_DIR"

    if [ -f "$ICON_SRC" ]; then
        cp "$ICON_SRC" "$ICON_DIR/ai-manager.png"
        ICON_PATH="ai-manager"
    else
        ICON_PATH="utilities-terminal"
    fi

    cat > "$DESKTOP_DIR/ai-manager.desktop" << EOF
[Desktop Entry]
Version=1.0
Type=Application
Name=AI Manager
Comment=Gestionnaire multi-comptes Claude AI
Exec=$ROOT/launch-linux.sh
Icon=$ICON_PATH
Terminal=false
Categories=Utility;Network;
StartupWMClass=AI Manager
EOF

    chmod +x "$DESKTOP_DIR/ai-manager.desktop"
    update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
    echo "Raccourci bureau installé. Lancez AI Manager depuis le menu d'applications."
    exit 0
fi

# --- Lancement direct si binaire existe ---
launch_app() {
    echo "Lancement de AI Manager..."
    nohup "$1" &>/dev/null &
    exit 0
}

if [ -f "$BIN" ] && [ -x "$BIN" ]; then
    launch_app "$BIN"
fi

if [ -n "$APPIMAGE" ] && [ -x "$APPIMAGE" ]; then
    launch_app "$APPIMAGE"
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

# Détecter le gestionnaire de paquets
detect_pkg_manager() {
    if command -v apt-get &>/dev/null; then
        echo "apt"
    elif command -v dnf &>/dev/null; then
        echo "dnf"
    elif command -v pacman &>/dev/null; then
        echo "pacman"
    elif command -v zypper &>/dev/null; then
        echo "zypper"
    else
        echo "unknown"
    fi
}

PKG=$(detect_pkg_manager)

# ===========================================
# ETAPE 1 : Dépendances système (WebKitGTK, etc.)
# ===========================================
echo "[1/4] Installation des dépendances système..."

case "$PKG" in
    apt)
        sudo apt-get update -qq
        sudo apt-get install -y \
            build-essential curl wget file \
            libssl-dev libgtk-3-dev libayatana-appindicator3-dev \
            librsvg2-dev libwebkit2gtk-4.1-dev \
            pkg-config libavahi-compat-libdnssd-dev
        ;;
    dnf)
        sudo dnf install -y \
            gcc gcc-c++ make curl wget file \
            openssl-devel gtk3-devel libappindicator-gtk3-devel \
            librsvg2-devel webkit2gtk4.1-devel \
            pkg-config avahi-compat-libdns_sd-devel
        ;;
    pacman)
        sudo pacman -S --needed --noconfirm \
            base-devel curl wget file \
            openssl gtk3 libappindicator-gtk3 \
            librsvg webkit2gtk-4.1 \
            pkgconf avahi
        ;;
    zypper)
        sudo zypper install -y \
            gcc gcc-c++ make curl wget file \
            libopenssl-devel gtk3-devel libappindicator3-devel \
            librsvg-devel webkit2gtk3-devel \
            pkg-config avahi-compat-mDNSResponder-devel
        ;;
    *)
        echo "[!] Gestionnaire de paquets non reconnu."
        echo "    Installez manuellement : build-essential, libwebkit2gtk-4.1-dev, libgtk-3-dev, libssl-dev"
        echo "    Puis relancez ce script."
        exit 1
        ;;
esac
echo "[OK] Dépendances système installées."
echo ""

# ===========================================
# ETAPE 2 : Rust (via rustup)
# ===========================================
if ! command -v rustc &>/dev/null; then
    echo "[2/4] Installation de Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    source "$HOME/.cargo/env"

    if ! command -v rustc &>/dev/null; then
        echo "[!] Rust non détecté après installation. Fermez et rouvrez le terminal, puis relancez."
        exit 1
    fi
    echo "[OK] Rust installé."
    echo ""
else
    echo "[OK] Rust déjà installé."
fi

# ===========================================
# ETAPE 3 : Node.js (via NodeSource ou nvm)
# ===========================================
if ! command -v node &>/dev/null; then
    echo "[3/4] Installation de Node.js LTS..."

    # Utiliser NodeSource pour installer Node 22 LTS
    if [ "$PKG" = "apt" ]; then
        curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash -
        sudo apt-get install -y nodejs
    elif [ "$PKG" = "dnf" ]; then
        curl -fsSL https://rpm.nodesource.com/setup_22.x | sudo bash -
        sudo dnf install -y nodejs
    elif [ "$PKG" = "pacman" ]; then
        sudo pacman -S --needed --noconfirm nodejs npm
    elif [ "$PKG" = "zypper" ]; then
        sudo zypper install -y nodejs22 npm22
    else
        # Fallback : nvm
        curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash
        export NVM_DIR="$HOME/.nvm"
        [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"
        nvm install --lts
    fi

    if ! command -v node &>/dev/null; then
        echo "[!] Node.js non détecté après installation. Fermez et rouvrez le terminal, puis relancez."
        exit 1
    fi
    echo "[OK] Node.js installé ($(node --version))."
    echo ""
else
    echo "[OK] Node.js déjà installé ($(node --version))."
fi

# ===========================================
# ETAPE 4 : Compilation AI Manager
# ===========================================
echo "[4/4] Compilation de AI Manager v3..."
echo "       (première compilation : ~5-10 minutes)"
echo ""

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
# Recalculer les chemins après build
BIN="$ROOT/target/release/ai-manager"
APPIMAGE=$(find "$ROOT/target/release/bundle/appimage" -name "*.AppImage" 2>/dev/null | head -1)

if [ -f "$BIN" ] && [ -x "$BIN" ]; then
    echo ""
    echo "══════════════════════════════════════════════"
    echo "  AI Manager v3 installé avec succès !"
    echo "  Lancement..."
    echo "══════════════════════════════════════════════"
    launch_app "$BIN"
elif [ -n "$APPIMAGE" ]; then
    chmod +x "$APPIMAGE"
    launch_app "$APPIMAGE"
else
    echo "[!] Binaire introuvable après compilation."
    exit 1
fi
