"""
Silas Blue - Automated & Manual Update Engine
Handles version checking, release channels (Stable vs Latest), git operations,
snapshots/backups, automated dependency installation, health checks, and rollbacks.
Supports both Docker Compose and Native environments.
"""

import os
import sys
import json
import shutil
import logging
import asyncio
import datetime
import subprocess
import urllib.request
import urllib.error
import re
from typing import Optional, Dict, Any, Tuple, List

from utils import load_app_config, save_app_config

# Remove potentially conflicting multi-path PYTHONHOME in Windows environments
os.environ.pop("PYTHONHOME", None)

logger = logging.getLogger("silasblue.updater")

GITHUB_REPO = "RBND/Silas-Blue"
STATE_DIR = "state"
CONFIG_DIR = "config"
BACKUP_DIR = "backups"
PENDING_UPDATE_FILE = os.path.join(STATE_DIR, "pending_update.json")
UPDATE_STATUS_FILE = os.path.join(STATE_DIR, "update_status.json")
DOCKER_SIGNAL_FILE = os.path.join(STATE_DIR, "trigger_docker_update.sig")

def get_current_version() -> str:
    """Reads __version__ dynamically from SilasBlue.py."""
    try:
        sb_path = os.path.join(os.path.dirname(__file__), "SilasBlue.py")
        with open(sb_path, "r", encoding="utf-8") as f:
            for line in f:
                if line.startswith("__version__"):
                    parts = line.split("=")
                    if len(parts) >= 2:
                        return parts[1].strip().strip('"').strip("'")
    except Exception as e:
        logger.debug(f"Could not read __version__ from SilasBlue.py: {e}")
    return "3.5.3"

def parse_semver(v_str: str) -> Tuple[int, ...]:
    """Extracts numeric semver tuple (major, minor, patch) from version string."""
    cleaned = re.sub(r'^[vV]', '', v_str.strip())
    # Match standard X.Y.Z
    match = re.match(r'^(\d+)(?:\.(\d+))?(?:\.(\d+))?', cleaned)
    if match:
        parts = [int(p) if p is not None else 0 for p in match.groups()]
        return tuple(parts)
    return (0, 0, 0)

def is_docker() -> bool:
    """Detects if Silas Blue is currently running inside a Docker container."""
    if os.path.exists("/.dockerenv"):
        return True
    if os.environ.get("DOCKER_CONTAINER") == "1":
        return True
    try:
        with open("/proc/1/cgroup", "rt") as f:
            return "docker" in f.read()
    except Exception:
        pass
    return False

def is_git_repo() -> bool:
    """Checks if current directory is a git repository and git command is available."""
    return bool(shutil.which("git") and os.path.isdir(".git"))

def run_git(args: List[str], timeout: int = 45) -> Tuple[int, str, str]:
    """Executes a git command safely and returns (returncode, stdout, stderr)."""
    try:
        result = subprocess.run(
            ["git"] + args,
            capture_output=True,
            text=True,
            timeout=timeout,
            cwd=os.path.abspath(os.path.dirname(__file__))
        )
        return result.returncode, result.stdout.strip(), result.stderr.strip()
    except subprocess.TimeoutExpired:
        return -1, "", "Git command timed out"
    except Exception as e:
        return -1, "", str(e)

def get_current_commit() -> str:
    """Returns the current HEAD commit SHA (short 7-char)."""
    if not is_git_repo():
        return "unknown"
    rc, out, _ = run_git(["rev-parse", "--short", "HEAD"])
    return out if rc == 0 else "unknown"

def get_current_branch() -> str:
    """Returns current active git branch name."""
    if not is_git_repo():
        return "main"
    rc, out, _ = run_git(["rev-parse", "--abbrev-ref", "HEAD"])
    return out if rc == 0 else "main"

class BotUpdater:
    """Core updater engine for Silas Blue."""

    @staticmethod
    def query_github_api(endpoint: str) -> Optional[Any]:
        """Queries the GitHub REST API with user-agent."""
        url = f"https://api.github.com/repos/{GITHUB_REPO}/{endpoint.lstrip('/')}"
        req = urllib.request.Request(
            url,
            headers={
                "User-Agent": "SilasBlue-Updater",
                "Accept": "application/vnd.github.v3+json"
            }
        )
        try:
            with urllib.request.urlopen(req, timeout=10) as resp:
                if resp.status == 200:
                    return json.loads(resp.read().decode("utf-8"))
        except Exception as e:
            logger.debug(f"GitHub API query failed for {url}: {e}")
        return None

    @classmethod
    def check_for_updates(cls, channel: Optional[str] = None) -> Dict[str, Any]:
        """
        Checks for available updates on the specified or configured channel.
        channel: 'stable' or 'latest'
        """
        cfg = load_app_config()
        if not channel:
            channel = cfg.get("update_channel", "stable").lower()

        current_ver = get_current_version()
        current_sha = get_current_commit()

        status: Dict[str, Any] = {
            "channel": channel,
            "current_version": current_ver,
            "current_commit": current_sha,
            "latest_version": current_ver,
            "latest_commit": current_sha,
            "update_available": False,
            "commits_behind": 0,
            "changelog": [],
            "release_url": f"https://github.com/{GITHUB_REPO}",
            "last_check": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "error": None
        }

        try:
            if channel == "stable":
                # Check GitHub Releases
                candidates = []
                rel_data = cls.query_github_api("releases/latest")
                if rel_data and "tag_name" in rel_data:
                    tag = rel_data["tag_name"]
                    remote_ver = tag.lstrip("vV")
                    candidates.append({
                        "version": remote_ver,
                        "tag": tag,
                        "url": rel_data.get("html_url", status["release_url"]),
                        "body": rel_data.get("body", "")
                    })

                # Also inspect tags
                tags_data = cls.query_github_api("tags")
                if tags_data and isinstance(tags_data, list):
                    for t in tags_data[:10]:
                        tag_name = t.get("name", "")
                        remote_ver = tag_name.lstrip("vV")
                        candidates.append({
                            "version": remote_ver,
                            "tag": tag_name,
                            "url": f"https://github.com/{GITHUB_REPO}/releases/tag/{tag_name}",
                            "body": ""
                        })

                if candidates:
                    # Pick candidate with highest parsed semver
                    candidates.sort(key=lambda c: parse_semver(c["version"]), reverse=True)
                    best = candidates[0]
                    status["latest_version"] = best["version"]
                    status["release_url"] = best["url"]
                    if best["body"]:
                        status["changelog"] = [line.strip() for line in best["body"].splitlines() if line.strip()][:10]

                    if parse_semver(best["version"]) > parse_semver(current_ver):
                        status["update_available"] = True

            else:
                # Latest channel: checks git origin/main
                if is_git_repo():
                    rc, _, _ = run_git(["fetch", "origin", "main", "--force"])
                    if rc == 0:
                        # Get remote commit hash
                        rc_rev, remote_sha, _ = run_git(["rev-parse", "--short", "origin/main"])
                        if rc_rev == 0:
                            status["latest_commit"] = remote_sha
                        
                        # Count commits behind
                        rc_cnt, cnt_str, _ = run_git(["rev-list", "--count", "HEAD..origin/main"])
                        if rc_cnt == 0 and cnt_str.isdigit():
                            behind = int(cnt_str)
                            status["commits_behind"] = behind
                            if behind > 0:
                                status["update_available"] = True
                                # Get recent commit messages
                                rc_log, log_str, _ = run_git(["log", "HEAD..origin/main", "--oneline", "-n", "10"])
                                if rc_log == 0 and log_str:
                                    status["changelog"] = log_str.splitlines()
                else:
                    # Fallback to GitHub API if not a git repo
                    branch_data = cls.query_github_api("commits/main")
                    if branch_data and "sha" in branch_data:
                        remote_sha = branch_data["sha"][:7]
                        status["latest_commit"] = remote_sha
                        msg = branch_data.get("commit", {}).get("message", "")
                        if msg:
                            status["changelog"] = [msg.splitlines()[0]]
                        if remote_sha != current_sha and current_sha != "unknown":
                            status["update_available"] = True

        except Exception as e:
            logger.error(f"Error checking for updates: {e}")
            status["error"] = str(e)

        # Save update status to state file
        try:
            os.makedirs(STATE_DIR, exist_ok=True)
            with open(UPDATE_STATUS_FILE, "w", encoding="utf-8") as f:
                json.dump(status, f, indent=2)
        except Exception as e:
            logger.debug(f"Failed to write {UPDATE_STATUS_FILE}: {e}")

        return status

    @classmethod
    def create_backup(cls) -> str:
        """
        Creates a timestamped snapshot of config/ and state/ in backups/.
        Rotates backups, keeping the 5 most recent snapshots.
        Returns the path to the created backup directory.
        """
        os.makedirs(BACKUP_DIR, exist_ok=True)
        timestamp = datetime.datetime.now().strftime("%Y%m%d_%H%M%S")
        backup_path = os.path.join(BACKUP_DIR, f"backup_{timestamp}")
        os.makedirs(backup_path, exist_ok=True)

        # Backup config/
        if os.path.exists(CONFIG_DIR):
            shutil.copytree(CONFIG_DIR, os.path.join(backup_path, "config"), dirs_exist_ok=True)

        # Backup state/ (excluding pending_update and backups themselves)
        if os.path.exists(STATE_DIR):
            state_dest = os.path.join(backup_path, "state")
            os.makedirs(state_dest, exist_ok=True)
            for item in os.listdir(STATE_DIR):
                if item in ("pending_update.json", "update_status.json", "trigger_docker_update.sig"):
                    continue
                src = os.path.join(STATE_DIR, item)
                dst = os.path.join(state_dest, item)
                if os.path.isfile(src):
                    shutil.copy2(src, dst)
                elif os.path.isdir(src):
                    shutil.copytree(src, dst, dirs_exist_ok=True)

        logger.info(f"Snapshot backup successfully created at {backup_path}")

        # Rotate old backups (keep newest 5)
        try:
            existing = [
                os.path.join(BACKUP_DIR, d) for d in os.listdir(BACKUP_DIR)
                if os.path.isdir(os.path.join(BACKUP_DIR, d)) and d.startswith("backup_")
            ]
            existing.sort(key=os.path.getmtime)
            while len(existing) > 5:
                oldest = existing.pop(0)
                shutil.rmtree(oldest, ignore_errors=True)
                logger.debug(f"Rotated out old backup: {oldest}")
        except Exception as e:
            logger.debug(f"Backup rotation notice: {e}")

        return backup_path

    @classmethod
    def apply_update(
        cls,
        channel: Optional[str] = None,
        initiator: str = "system",
        notify_context: Optional[Dict[str, Any]] = None
    ) -> Tuple[bool, str]:
        """
        Applies update using strict clean pull (git fetch + reset --hard).
        Backs up config and state first, checks dependencies, and creates pending_update.json.
        """
        if not is_git_repo():
            return False, "Not running in a Git repository. Cannot perform git-based update."

        cfg = load_app_config()
        if not channel:
            channel = cfg.get("update_channel", "stable").lower()

        # 1. Create pre-update backup snapshot
        try:
            backup_path = cls.create_backup()
        except Exception as e:
            return False, f"Failed to create pre-update backup: {e}"

        prev_commit = get_current_commit()
        prev_version = get_current_version()

        # 2. Fetch updates from remote (use --force so updating tags like 'Latest' do not fail)
        rc, _, err = run_git(["fetch", "origin", "main", "--tags", "--force"])
        if rc != 0:
            rc, _, err = run_git(["fetch", "--tags", "--force", "origin"])
            if rc != 0:
                return False, f"Git fetch failed: {err}"

        # 3. Pull / Checkout target
        target_ref = "origin/main"
        target_version = prev_version

        if channel == "stable":
            # Find the latest tag
            check_res = cls.check_for_updates("stable")
            latest_tag_name = check_res.get("latest_version")
            if latest_tag_name:
                # Try checkout tag (e.g. v3.3.4 or 3.3.4)
                possible_tags = [f"v{latest_tag_name}", latest_tag_name, f"V{latest_tag_name}"]
                tag_found = None
                for t in possible_tags:
                    rc_tag, _, _ = run_git(["rev-parse", f"tags/{t}"])
                    if rc_tag == 0:
                        tag_found = t
                        break
                
                if tag_found:
                    target_ref = f"tags/{tag_found}"
                    target_version = latest_tag_name
                    rc_co, _, err_co = run_git(["checkout", target_ref])
                    if rc_co != 0:
                        return False, f"Failed to checkout tag {tag_found}: {err_co}"
                else:
                    # Fallback to main if specific tag is not indexed locally
                    target_ref = "origin/main"
                    run_git(["checkout", "main"])
                    run_git(["reset", "--hard", "origin/main"])
        else:
            # Latest channel: checkout main and reset hard to origin/main
            rc_co, _, err_co = run_git(["checkout", "main"])
            if rc_co != 0:
                return False, f"Failed to checkout main branch: {err_co}"
            rc_rst, _, err_rst = run_git(["reset", "--hard", "origin/main"])
            if rc_rst != 0:
                return False, f"Failed to reset --hard to origin/main: {err_rst}"
            target_version = get_current_version()

        new_commit = get_current_commit()

        # 4. Check if requirements.txt changed; if so, update dependencies
        rc_diff, diff_out, _ = run_git(["diff", f"{prev_commit}..{new_commit}", "--", "requirements.txt"])
        if rc_diff == 0 and diff_out.strip():
            logger.info("requirements.txt modified in update. Installing updated Python packages...")
            try:
                subprocess.run(
                    [sys.executable, "-m", "pip", "install", "--no-cache-dir", "-r", "requirements.txt"],
                    check=True,
                    timeout=180
                )
            except Exception as e:
                logger.warning(f"Dependency installation encountered a warning/error: {e}")

        # 5. Record pending update marker for post-boot health verification
        pending_data = {
            "previous_commit": prev_commit,
            "previous_version": prev_version,
            "target_commit": new_commit,
            "target_version": target_version,
            "channel": channel,
            "backup_dir": backup_path,
            "initiator": initiator,
            "notify_context": notify_context or {},
            "timestamp": datetime.datetime.now(datetime.timezone.utc).isoformat()
        }

        os.makedirs(STATE_DIR, exist_ok=True)
        with open(PENDING_UPDATE_FILE, "w", encoding="utf-8") as f:
            json.dump(pending_data, f, indent=2)

        # Mark update as applied / up-to-date in update_status.json
        try:
            status_data = {
                "channel": channel,
                "current_version": target_version,
                "current_commit": new_commit,
                "latest_version": target_version,
                "latest_commit": new_commit,
                "update_available": False,
                "commits_behind": 0,
                "changelog": [],
                "release_url": f"https://github.com/{GITHUB_REPO}",
                "last_check": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                "error": None
            }
            with open(UPDATE_STATUS_FILE, "w", encoding="utf-8") as f:
                json.dump(status_data, f, indent=2)
        except Exception as e:
            logger.debug(f"Failed to reset update_status.json in apply_update: {e}")

        logger.info(f"Update successfully applied: {prev_version} ({prev_commit}) -> {target_version} ({new_commit})")
        return True, f"Successfully updated from {prev_version} ({prev_commit}) to {target_version} ({new_commit}). Restart required."

    @classmethod
    def trigger_restart(cls):
        """Restarts the process or signals Docker container restart."""
        logger.info("Initiating Silas Blue restart...")
        
        # If running inside Docker, write signal file for host watcher / trigger exit
        if is_docker():
            try:
                os.makedirs(STATE_DIR, exist_ok=True)
                with open(DOCKER_SIGNAL_FILE, "w", encoding="utf-8") as f:
                    f.write(datetime.datetime.now(datetime.timezone.utc).isoformat())
                logger.info(f"Docker signal written to {DOCKER_SIGNAL_FILE}. Terminating container for restart...")
            except Exception as e:
                logger.warning(f"Failed to write Docker signal file: {e}")
            try:
                import signal
                os.kill(1, signal.SIGTERM)
            except Exception:
                pass
            # Exit process cleanly at OS level so Docker restart policy restarts the container
            os._exit(0)

        # For Native installations (Windows / Linux / macOS)
        try:
            # Spawn new process
            args = [sys.executable] + sys.argv
            logger.info(f"Spawning restart process: {args}")
            if sys.platform.startswith("win"):
                # Windows spawn detached process
                subprocess.Popen(
                    args,
                    creationflags=subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.DETACHED_PROCESS,
                    close_fds=True
                )
            else:
                # Unix spawn detached process
                subprocess.Popen(
                    args,
                    preexec_fn=os.setpgrp,
                    close_fds=True
                )
            # Exit current process at OS level
            os._exit(0)
        except Exception as e:
            logger.error(f"Failed to spawn restart process: {e}")
            # Fallback to direct execv
            os.execv(sys.executable, [sys.executable] + sys.argv)

    @classmethod
    def verify_boot_health(cls) -> Optional[Dict[str, Any]]:
        """
        Called on bot startup when reaching on_ready().
        If pending_update.json exists, marks update as successful and returns notification payload.
        """
        if not os.path.exists(PENDING_UPDATE_FILE):
            return None

        try:
            with open(PENDING_UPDATE_FILE, "r", encoding="utf-8") as f:
                data = json.load(f)

            # Successfully reached on_ready()! Remove pending update marker
            os.remove(PENDING_UPDATE_FILE)
            logger.info(f"Post-update health check passed! Upgraded to {get_current_version()} ({get_current_commit()})")

            # Update update_status.json so cached status reports UP TO DATE
            try:
                curr_v = get_current_version()
                curr_c = get_current_commit()
                status_data = {
                    "channel": data.get("channel", "stable"),
                    "current_version": curr_v,
                    "current_commit": curr_c,
                    "latest_version": curr_v,
                    "latest_commit": curr_c,
                    "update_available": False,
                    "commits_behind": 0,
                    "changelog": [],
                    "release_url": f"https://github.com/{GITHUB_REPO}",
                    "last_check": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    "error": None
                }
                with open(UPDATE_STATUS_FILE, "w", encoding="utf-8") as f:
                    json.dump(status_data, f, indent=2)
            except Exception as e:
                logger.debug(f"Failed to reset update_status.json in verify_boot_health: {e}")

            # Collect commit log changelog
            prev = data.get("previous_commit")
            curr = get_current_commit()
            changelog_lines = []
            if prev and prev != "unknown" and curr != "unknown":
                rc, log_out, _ = run_git(["log", f"{prev}..{curr}", "--oneline", "-n", "8"])
                if rc == 0 and log_out:
                    changelog_lines = log_out.splitlines()

            data["changelog"] = changelog_lines
            data["new_version"] = get_current_version()
            data["new_commit"] = curr
            return data

        except Exception as e:
            logger.error(f"Error reading pending update file: {e}")
            return None

    @classmethod
    def rollback(cls) -> Tuple[bool, str]:
        """
        Rolls back to the previous commit/tag and restores config/state from backup.
        Triggered when repeated crashes occur after an update.
        """
        if not os.path.exists(PENDING_UPDATE_FILE):
            return False, "No pending update record found to rollback."

        try:
            with open(PENDING_UPDATE_FILE, "r", encoding="utf-8") as f:
                data = json.load(f)

            prev_commit = data.get("previous_commit")
            backup_dir = data.get("backup_dir")

            if not prev_commit or prev_commit == "unknown":
                return False, "Invalid previous commit in pending update record."

            logger.warning(f"ROLLBACK TRIGGERED: Reverting to {prev_commit}...")

            # 1. Reset Git to previous commit
            rc, _, err = run_git(["checkout", prev_commit])
            if rc != 0:
                rc_rst, _, err_rst = run_git(["reset", "--hard", prev_commit])
                if rc_rst != 0:
                    return False, f"Git rollback failed: {err} | {err_rst}"

            # 2. Restore config and state from backup snapshot
            if backup_dir and os.path.isdir(backup_dir):
                cfg_backup = os.path.join(backup_dir, "config")
                if os.path.isdir(cfg_backup):
                    shutil.copytree(cfg_backup, CONFIG_DIR, dirs_exist_ok=True)
                
                state_backup = os.path.join(backup_dir, "state")
                if os.path.isdir(state_backup):
                    shutil.copytree(state_backup, STATE_DIR, dirs_exist_ok=True)
                logger.info("Restored config and state directories from pre-update snapshot.")

            # Remove pending marker
            if os.path.exists(PENDING_UPDATE_FILE):
                os.remove(PENDING_UPDATE_FILE)

            logger.info("Rollback complete.")
            return True, f"Successfully rolled back to commit {prev_commit}."

        except Exception as e:
            logger.error(f"Rollback encountered error: {e}")
            return False, str(e)


async def auto_update_background_loop(bot_client=None):
    """
    Background worker that runs periodically to check for updates and
    execute either notification or auto-apply based on user configuration.
    """
    logger.info("Starting Auto-Update background scheduler...")
    
    # Wait 60 seconds after initial boot before first check to avoid congestion
    await asyncio.sleep(60)

    while True:
        try:
            cfg = load_app_config()
            enabled = cfg.get("auto_update_enabled", False)
            interval_hours = max(1, int(cfg.get("auto_update_interval_hours", 24)))
            mode = cfg.get("auto_update_mode", "notify").lower()

            if enabled:
                logger.info(f"Running scheduled auto-update check (mode: {mode})...")
                res = await asyncio.to_thread(BotUpdater.check_for_updates)

                if res.get("update_available"):
                    logger.info(f"Auto-updater detected new update! Latest: {res.get('latest_version')} ({res.get('latest_commit')})")
                    
                    if mode == "apply":
                        logger.info("Auto-update mode is 'apply'. Automatically applying update...")
                        ok, msg = await asyncio.to_thread(
                            BotUpdater.apply_update,
                            channel=cfg.get("update_channel", "stable"),
                            initiator="auto_scheduler"
                        )
                        if ok:
                            logger.info("Auto-update applied. Triggering restart...")
                            await asyncio.sleep(2)
                            BotUpdater.trigger_restart()
                        else:
                            logger.error(f"Auto-update apply failed: {msg}")
                    else:
                        # Notify mode: send alert to Discord owner / channel if bot client available
                        logger.info("Auto-update mode is 'notify'. Sending notification alerts...")
                        if bot_client and hasattr(bot_client, "dispatch_update_alert"):
                            await bot_client.dispatch_update_alert(res)

            # Sleep for configured interval
            await asyncio.sleep(interval_hours * 3600)

        except asyncio.CancelledError:
            logger.info("Auto-update background scheduler stopped.")
            break
        except Exception as e:
            logger.error(f"Unexpected error in auto-update scheduler: {e}")
            await asyncio.sleep(300)
