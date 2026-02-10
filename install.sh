#!/bin/sh
set -e

REPO="https://github.com/emphoni/cch.git"
INSTALL_DIR="$HOME/.cch-src"

echo "Installing cch..."

if [ -d "$INSTALL_DIR" ]; then
  echo "Updating existing install..."
  git -C "$INSTALL_DIR" pull -q
else
  git clone -q "$REPO" "$INSTALL_DIR"
fi

# Detect shell config
if [ -n "$ZSH_VERSION" ] || [ -f "$HOME/.zshrc" ]; then
  RC="$HOME/.zshrc"
elif [ -f "$HOME/.bashrc" ]; then
  RC="$HOME/.bashrc"
else
  RC="$HOME/.profile"
fi

# Add alias if not already present
if ! grep -q 'alias cch=' "$RC" 2>/dev/null; then
  echo "" >> "$RC"
  echo "# cch - Claude Code Helper" >> "$RC"
  echo "alias cch=\"python3 $INSTALL_DIR/cch.py\"" >> "$RC"
fi

echo "Done. Run: source $RC"
echo "Then: cch --help"
