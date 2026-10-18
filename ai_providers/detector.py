"""
Auto-detection utility for local AI backends (Llama.cpp and Ollama).
Detects running server instances, installed binaries, and local model files (.gguf).
"""

import os
import sys
import glob
import json
import socket
import shutil
import logging
import urllib.request
import urllib.error
import subprocess
from typing import Optional, Dict, Any, List, Tuple

logger = logging.getLogger("silasblue")

COMMON_LLAMACPP_PORTS = [9931, 8080, 8000, 8081, 5000]
OLLAMA_DEFAULT_PORT = 11434


def probe_tcp_port(host: str, port: int, timeout: float = 0.25) -> bool:
    """Quickly tests if a TCP port is open."""
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.settimeout(timeout)
        res = s.connect_ex((host, port))
        s.close()
        return res == 0
    except Exception:
        return False


def verify_llamacpp_url(url: str, timeout: float = 0.8) -> Tuple[bool, Dict[str, Any]]:
    """
    Verifies if a given HTTP endpoint is a responsive Llama.cpp (or OpenAI-compatible) server.
    Checks /props, /health, and /v1/models endpoints.
    """
    clean_url = url.rstrip("/")

    # 1. Probe /props (definitive for llama.cpp / llama-server)
    try:
        req = urllib.request.Request(f"{clean_url}/props", headers={"User-Agent": "SilasBlue/1.0"})
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            if resp.status == 200:
                data = json.loads(resp.read().decode("utf-8", errors="ignore"))
                if any(k in data for k in ("build_info", "model_alias", "default_generation_settings", "models_autoload")):
                    return True, data
    except Exception:
        pass

    # 2. Probe /health
    try:
        req = urllib.request.Request(f"{clean_url}/health", headers={"User-Agent": "SilasBlue/1.0"})
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            if resp.status in (200, 503):
                return True, {}
    except Exception:
        pass

    # 3. Probe /v1/models (OpenAI compatible endpoint)
    try:
        req = urllib.request.Request(f"{clean_url}/v1/models", headers={"User-Agent": "SilasBlue/1.0"})
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            if resp.status == 200:
                data = json.loads(resp.read().decode("utf-8", errors="ignore"))
                return True, data
    except Exception:
        pass

    return False, {}


def detect_llama_app_config() -> Optional[Dict[str, Any]]:
    """
    Checks for the Windows Llama App settings.json configuration.
    Extracts ServerPort, ListenAddress, ServerApiKey, and CacheDirectory.
    """
    if sys.platform != "win32":
        return None

    patterns = [
        os.path.expandvars(r"%LOCALAPPDATA%\Packages\*llama*\LocalCache\Local\Llama\settings.json"),
        os.path.expandvars(r"%LOCALAPPDATA%\Packages\*4e440e29*\LocalCache\Local\Llama\settings.json")
    ]
    for pattern in patterns:
        for match in glob.glob(pattern):
            try:
                with open(match, "r", encoding="utf-8") as f:
                    return json.load(f)
            except Exception:
                continue
    return None


def detect_listening_ports_from_processes() -> List[int]:
    """Finds listening ports for running llama.exe / llama-server processes on Windows."""
    ports: List[int] = []
    if sys.platform != "win32":
        return ports

    try:
        # Find PIDs of processes matching llama*
        out = subprocess.check_output(
            'tasklist /FI "IMAGENAME eq llama*" /FO CSV /NH',
            shell=True,
            stderr=subprocess.DEVNULL
        )
        lines = out.decode("utf-8", errors="ignore").strip().splitlines()
        pids = []
        for line in lines:
            parts = [p.strip(' "') for p in line.split(",")]
            if len(parts) >= 2 and parts[1].isdigit():
                pids.append(parts[1])

        if not pids:
            return ports

        # Check netstat for listening ports owned by these PIDs
        netstat_out = subprocess.check_output("netstat -ano", shell=True, stderr=subprocess.DEVNULL)
        for n_line in netstat_out.decode("utf-8", errors="ignore").splitlines():
            n_parts = n_line.strip().split()
            if len(n_parts) >= 5 and n_parts[3].upper() == "LISTENING":
                pid = n_parts[4]
                if pid in pids:
                    addr_port = n_parts[1]
                    if ":" in addr_port:
                        port_str = addr_port.split(":")[-1]
                        if port_str.isdigit():
                            port = int(port_str)
                            if port not in ports:
                                ports.append(port)
    except Exception:
        pass
    return ports


def detect_running_llamacpp() -> Optional[Dict[str, Any]]:
    """
    Auto-detects an active, running local Llama.cpp instance.
    Probes configured LlamaApp ports, process-associated ports, and standard ports.
    """
    candidates = []

    # 1. Check LlamaApp configuration on Windows
    app_cfg = detect_llama_app_config()
    if app_cfg:
        port = app_cfg.get("ServerPort", 9931)
        addr = app_cfg.get("ListenAddress", "127.0.0.1")
        if addr == "0.0.0.0":
            addr = "127.0.0.1"
        candidates.append((addr, int(port), app_cfg.get("ServerApiKey", "")))

    # 2. Check ports discovered from running processes
    for port in detect_listening_ports_from_processes():
        candidates.append(("127.0.0.1", port, ""))

    # 3. Add standard ports
    for port in COMMON_LLAMACPP_PORTS:
        candidates.append(("127.0.0.1", port, ""))

    seen_endpoints = set()
    for host, port, api_key in candidates:
        endpoint = f"http://{host}:{port}"
        if endpoint in seen_endpoints:
            continue
        seen_endpoints.add(endpoint)

        if probe_tcp_port(host, port, timeout=0.2):
            ok, data = verify_llamacpp_url(endpoint, timeout=0.8)
            if ok:
                binary = detect_installed_llamacpp()
                models = detect_local_models()
                return {
                    "running": True,
                    "url": endpoint,
                    "host": host,
                    "port": port,
                    "api_key": api_key,
                    "binary_path": binary or "",
                    "model_path": models[0] if models else "",
                    "models": models,
                    "details": f"Online on port {port}" + (f" ({data.get('build_info', '')})" if data.get("build_info") else ""),
                    "props": data
                }

    return None


def detect_installed_llamacpp() -> Optional[str]:
    """
    Finds the absolute path to an installed llama executable on the system.
    Searches PATH, WindowsApps, and standard install directories for llama.exe or llama-server.exe.
    """
    # 1. Search PATH
    for name in ["llama-server", "llama-server.exe", "llama", "llama.exe"]:
        found = shutil.which(name)
        if found and os.path.isfile(found):
            return os.path.abspath(found)

    # 2. Common Windows paths
    if sys.platform == "win32":
        candidates = [
            os.path.expandvars(r"%LOCALAPPDATA%\Microsoft\WindowsApps\llama.exe"),
            os.path.expandvars(r"%LOCALAPPDATA%\Microsoft\WindowsApps\llama-server.exe"),
            os.path.expandvars(r"%USERPROFILE%\llama.cpp\llama-server.exe"),
            os.path.expandvars(r"%USERPROFILE%\llama.cpp\llama.exe"),
            os.path.expandvars(r"%LOCALAPPDATA%\Programs\llama.cpp\llama-server.exe"),
            os.path.expandvars(r"%LOCALAPPDATA%\Programs\llama.cpp\llama.exe"),
            r"C:\Program Files\llama.cpp\llama-server.exe",
            r"C:\Program Files\llama.cpp\llama.exe",
            os.path.expandvars(r"%USERPROFILE%\scoop\shims\llama-server.exe"),
            os.path.expandvars(r"%USERPROFILE%\scoop\shims\llama.exe"),
        ]
        for path in candidates:
            if os.path.isfile(path):
                return os.path.abspath(path)

    # 3. Common Unix / Linux / macOS paths
    unix_candidates = [
        "/usr/local/bin/llama-server",
        "/usr/local/bin/llama",
        "/opt/homebrew/bin/llama-server",
        "/opt/homebrew/bin/llama",
        os.path.expanduser("~/llama.cpp/llama-server"),
        os.path.expanduser("~/llama.cpp/llama"),
    ]
    for path in unix_candidates:
        if os.path.isfile(path):
            return os.path.abspath(path)

    return None


def detect_local_models() -> List[str]:
    """
    Searches for locally available GGUF model files (.gguf).
    Checks project models/, LlamaApp CacheDirectory, hermes, user home, and HF cache.
    """
    found_models: List[str] = []
    seen = set()

    search_dirs = [
        os.path.abspath("models"),
        os.path.abspath("."),
    ]

    # Check LlamaApp CacheDirectory from settings.json
    llama_cfg = detect_llama_app_config()
    if llama_cfg and llama_cfg.get("CacheDirectory"):
        search_dirs.append(llama_cfg["CacheDirectory"])

    # User cache directories
    if sys.platform == "win32":
        search_dirs.extend([
            os.path.expandvars(r"%LOCALAPPDATA%\hermes\models"),
            os.path.expandvars(r"%USERPROFILE%\models"),
            os.path.expandvars(r"%USERPROFILE%\.cache\huggingface\hub"),
        ])
    else:
        search_dirs.extend([
            os.path.expanduser("~/.cache/huggingface/hub"),
            os.path.expanduser("~/models"),
        ])

    for base_dir in search_dirs:
        if not os.path.exists(base_dir):
            continue
        try:
            # Check direct folder
            for entry in os.scandir(base_dir):
                if entry.is_file() and entry.name.lower().endswith(".gguf"):
                    full = os.path.abspath(entry.path)
                    if full not in seen:
                        seen.add(full)
                        found_models.append(full)
            # Check 2 levels deep (for HF hub snapshots)
            pattern = os.path.join(base_dir, "*", "*.gguf")
            for f in glob.glob(pattern):
                full = os.path.abspath(f)
                if full not in seen:
                    seen.add(full)
                    found_models.append(full)
            pattern_deep = os.path.join(base_dir, "*", "*", "*.gguf")
            for f in glob.glob(pattern_deep):
                full = os.path.abspath(f)
                if full not in seen:
                    seen.add(full)
                    found_models.append(full)
        except Exception:
            continue

    return found_models


def detect_running_ollama() -> Optional[Dict[str, Any]]:
    """Checks if Ollama is running on localhost."""
    url = f"http://127.0.0.1:{OLLAMA_DEFAULT_PORT}"
    if probe_tcp_port("127.0.0.1", OLLAMA_DEFAULT_PORT, timeout=0.2):
        try:
            req = urllib.request.Request(f"{url}/api/tags")
            with urllib.request.urlopen(req, timeout=0.8) as resp:
                if resp.status == 200:
                    data = json.loads(resp.read().decode("utf-8", errors="ignore"))
                    models = [m.get("name") for m in data.get("models", []) if m.get("name")]
                    return {
                        "running": True,
                        "url": url,
                        "models": models,
                        "details": f"Online ({len(models)} model{'s' if len(models) != 1 else ''})"
                    }
        except Exception:
            pass
    return None


def detect_installed_ollama() -> Optional[str]:
    """Finds Ollama executable if installed."""
    found = shutil.which("ollama")
    if found:
        return os.path.abspath(found)
    if sys.platform == "win32":
        candidates = [
            os.path.expandvars(r"%LOCALAPPDATA%\Programs\Ollama\ollama.exe"),
            r"C:\Program Files\Ollama\ollama.exe"
        ]
        for c in candidates:
            if os.path.isfile(c):
                return os.path.abspath(c)
    return None


def detect_all_ai_providers() -> Dict[str, Any]:
    """
    Performs comprehensive auto-detection across Llama.cpp and Ollama.
    Returns recommended backend and detected parameters.
    """
    llama_running = detect_running_llamacpp()
    llama_binary = detect_installed_llamacpp()
    models = detect_local_models()

    ollama_running = detect_running_ollama()
    ollama_binary = detect_installed_ollama()

    recommended = None
    if llama_running and not ollama_running:
        recommended = "llamacpp"
    elif ollama_running and not llama_running:
        recommended = "ollama"
    elif llama_running and ollama_running:
        recommended = "llamacpp"
    elif llama_binary and not ollama_binary:
        recommended = "llamacpp"
    elif ollama_binary:
        recommended = "ollama"

    return {
        "llamacpp": {
            "running": bool(llama_running),
            "url": llama_running.get("url", "http://localhost:8080") if llama_running else "http://localhost:8080",
            "binary_path": (llama_running.get("binary_path") if llama_running else None) or llama_binary or "",
            "model_path": (llama_running.get("model_path") if llama_running else None) or (models[0] if models else ""),
            "models": (llama_running.get("models") if llama_running else None) or models,
            "details": llama_running.get("details", "Offline") if llama_running else "Offline"
        },
        "ollama": {
            "running": bool(ollama_running),
            "url": ollama_running.get("url", f"http://localhost:{OLLAMA_DEFAULT_PORT}") if ollama_running else f"http://localhost:{OLLAMA_DEFAULT_PORT}",
            "binary_path": ollama_binary or "",
            "models": ollama_running.get("models", []) if ollama_running else [],
            "details": ollama_running.get("details", "Offline") if ollama_running else "Offline"
        },
        "recommended_backend": recommended
    }
