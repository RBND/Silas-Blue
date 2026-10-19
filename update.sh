#!/usr/bin/env bash
# Silas Blue Automated & Manual Updater for Linux and macOS
# Supports both native virtual environments and Docker Compose deployments.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

CHANNEL="stable"
IS_DOCKER=false
AUTO_YES=false

# Parse CLI arguments
while [[ "$#" -gt 0 ]]; do
    case "$1" in
        --channel) CHANNEL="$2"; shift 2 ;;
        --docker) IS_DOCKER=true; shift ;;
        -y|--yes) AUTO_YES=true; shift ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Auto-detect Docker environment if docker-compose.yml exists and Docker daemon is running
if [ "$IS_DOCKER" = false ] && command -v docker &>/dev/null; then
    if [ -f "$SCRIPT_DIR/docker-compose.yml" ]; then
        if docker compose ps -q silas-blue &>/dev/null && [ -n "$(docker compose ps -q silas-blue 2>/dev/null)" ]; then
            IS_DOCKER=true
            echo "[INFO] Detected active Docker Compose container 'silas-blue'."
        fi
    fi
fi

echo "========================================"
echo "       Silas Blue System Updater        "
echo "========================================"
echo "Target Channel: $CHANNEL"
echo "Target Mode:    $([ "$IS_DOCKER" = true ] && echo 'Docker Compose' || echo 'Native Python')"
echo ""

# 1. Snapshot Backups
TIMESTAMP=$(date +"%Y%m%d_%H%M%S")
BACKUP_PATH="$SCRIPT_DIR/backups/backup_$TIMESTAMP"
mkdir -p "$BACKUP_PATH"

echo "[1/4] Creating safety snapshot of configuration & state..."
if [ -d "$SCRIPT_DIR/config" ]; then
    cp -r "$SCRIPT_DIR/config" "$BACKUP_PATH/"
fi
if [ -d "$SCRIPT_DIR/state" ]; then
    mkdir -p "$BACKUP_PATH/state"
    find "$SCRIPT_DIR/state" -maxdepth 1 -not -name "pending_update.json" -not -name "update_status.json" -not -name "trigger_docker_update.sig" -exec cp -r {} "$BACKUP_PATH/state/" \; 2>/dev/null || true
fi
echo "[OK] Backup saved to $BACKUP_PATH"

# 2. Fetch updates from Git remote
echo ""
echo "[2/4] Fetching latest updates from GitHub..."
git fetch origin main --tags --force

PREV_COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")

if [ "$CHANNEL" = "stable" ]; then
    # Find latest tag
    LATEST_TAG=$(git tag -l "v*" "V*" | sort -V | tail -n 1)
    if [ -z "$LATEST_TAG" ]; then
        LATEST_TAG=$(git tag -l | sort -V | tail -n 1)
    fi

    if [ -n "$LATEST_TAG" ]; then
        echo "[INFO] Checking out latest release tag: $LATEST_TAG"
        git checkout "$LATEST_TAG"
    else
        echo "[WARN] No tags found. Falling back to origin/main."
        git checkout main
        git reset --hard origin/main
    fi
else
    echo "[INFO] Pulling latest commits from main branch..."
    git checkout main
    git reset --hard origin/main
fi

NEW_COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
echo "[OK] Updated repository: $PREV_COMMIT -> $NEW_COMMIT"

# 3. Environment-specific application
echo ""
if [ "$IS_DOCKER" = true ]; then
    echo "[3/4] Rebuilding and restarting Docker Compose service..."
    docker compose build --no-cache silas-blue
    docker compose up -d silas-blue
    echo "[OK] Docker container updated and restarted successfully."
else
    echo "[3/4] Checking Python dependencies..."
    PYTHON_EXEC="python3"
    if [ -f "$SCRIPT_DIR/.venv/bin/python" ]; then
        PYTHON_EXEC="$SCRIPT_DIR/.venv/bin/python"
    fi

    if git diff --name-only "$PREV_COMMIT..$NEW_COMMIT" 2>/dev/null | grep -q "requirements.txt"; then
        echo "[INFO] requirements.txt changed. Installing updated dependencies..."
        "$PYTHON_EXEC" -m pip install --no-cache-dir -r requirements.txt
    else
        echo "[INFO] Dependencies up to date."
    fi

    # Check if managed by systemd
    if command -v systemctl &>/dev/null && systemctl is-active --quiet silas-blue 2>/dev/null; then
        echo "[INFO] Restarting systemd service: silas-blue..."
        sudo systemctl restart silas-blue
        echo "[OK] systemd service restarted."
    else
        echo ""
        echo "[INFO] Silas Blue updated successfully."
        echo "Please restart Silas Blue using './start.sh' or your process supervisor."
    fi
fi

# 4. Cleanup old backups (keep last 5)
echo ""
echo "[4/4] Housekeeping..."
ls -dt "$SCRIPT_DIR/backups"/backup_* 2>/dev/null | tail -n +6 | xargs rm -rf 2>/dev/null || true

echo "========================================"
echo "    Update completed successfully!      "
echo "========================================"
