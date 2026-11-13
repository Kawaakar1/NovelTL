#!/bin/sh
# Добавляет NovelTL в меню приложений (для текущего пользователя). Запуск: ./install.sh
# Adds NovelTL to the application menu (current user). Run: ./install.sh
set -e
DIR="$(cd "$(dirname "$0")" && pwd)"
APPS="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICONS="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps"
mkdir -p "$APPS" "$ICONS"
cp "$DIR/noveltl.png" "$ICONS/noveltl.png"
cat > "$APPS/noveltl.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=NovelTL
Comment=Visual novel translator
Exec="$DIR/noveltl"
Icon=noveltl
Terminal=false
Categories=Utility;Game;
EOF
chmod +x "$DIR/noveltl"
echo "NovelTL добавлен в меню приложений / added to the application menu"
