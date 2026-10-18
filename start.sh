#!/usr/bin/env bash
# Silas Blue v3.1.0 Launcher for Linux and macOS

set -e

# Change directory to the folder containing this script
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "========================================"
echo "       Starting Silas Blue v3.1.0"
echo "========================================"
echo ""

# 1. Check for project virtual environment (POSIX path: .venv/bin/python)
if [ -f "$SCRIPT_DIR/.venv/bin/python" ]; then
    echo "[INFO] Using virtual environment: .venv"
    if [ -f "$SCRIPT_DIR/.venv/bin/activate" ]; then
        # shellcheck disable=SC1091
        source "$SCRIPT_DIR/.venv/bin/activate"
    fi
    exec "$SCRIPT_DIR/.venv/bin/python" SilasBlue.py "$@"
fi

# 2. Check for system python3
if command -v python3 &>/dev/null; then
    echo "[INFO] Using system python3..."
    exec python3 SilasBlue.py "$@"
fi

# 3. Check for system python
if command -v python &>/dev/null; then
    echo "[INFO] Using system python..."
    exec python SilasBlue.py "$@"
fi

# 4. No Python found
echo "[ERROR] Python 3 was not found on your system!"
echo "Please install Python 3 (3.10+) via your package manager or from:"
echo "https://www.python.org/downloads"
echo ""
exit 1
