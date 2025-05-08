"""
Ollama API integration for Silas Blue.
Handles sending prompts, listing models, downloading models, and status.
"""

import requests
import threading
import subprocess
import logging
import time
import re
import json
import config

logger = logging.getLogger("silasblue")

class OllamaClient:
    def __init__(self, base_url=None):
        self.base_url = base_url or "http://localhost:11434"
        self.ollama_process = None

    def send_prompt(self, prompt, model):
        """
        Sends a prompt to Ollama and returns the response.
        """
        url = f"{self.base_url}/api/generate"
        data = {"model": model, "prompt": prompt}
        try:
            resp = requests.post(url, json=data, timeout=60)
            resp.raise_for_status()
            # Always decode as UTF-8
            text = resp.content.decode('utf-8')
            responses = []
            for line in text.strip().splitlines():
                try:
                    obj = json.loads(line)
                    if 'response' in obj:
                        responses.append(obj['response'])
                except Exception:
                    continue
            return ''.join(responses) if responses else resp.json().get("response", "")
        except Exception as e:
            return f"Error: {e}"

    def list_models(self):
        """
        Returns a list of available models.
        """
        url = f"{self.base_url}/api/tags"
        if config.DEBUG:
            print(f"[DEBUG] OllamaClient.list_models() requesting: {url}")
        try:
            resp = requests.get(url, timeout=5)
            if config.DEBUG:
                print(f"[DEBUG] OllamaClient.list_models() response status: {resp.status_code}")
            if resp.status_code == 200:
                if config.DEBUG:
                    print(f"[DEBUG] OllamaClient.list_models() response text: {resp.text[:200]}")
                return [m['name'] for m in resp.json().get('models', [])]
            else:
                if config.DEBUG:
                    print(f"[DEBUG] OllamaClient.list_models() error: {resp.text}")
                return []
        except Exception as e:
            if config.DEBUG:
                print(f"[DEBUG] OllamaClient.list_models() error: {e}")
            return []

    def download_model(self, model_name, progress_callback=None):
        """
        Downloads a model using the ollama CLI and pipes output to logger and callback.
        """
        logger.info(f"Starting download for model: {model_name}")
        start_time = time.time()
        try:
            process = subprocess.Popen(
                ["ollama", "pull", model_name],
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
                text=True
            )
            for line in process.stdout:
                logger.info(line.strip())
                if progress_callback:
                    # Try to parse percentage from line, e.g. "pulling manifest:  50% ..."
                    match = re.search(r'(\d+)%', line)
                    if match:
                        percent = int(match.group(1))
                        elapsed = time.time() - start_time
                        speed = f"{percent / elapsed:.2f}%/s" if elapsed > 0 else ""
                        progress_callback(percent, speed)
            process.wait()
            if process.returncode == 0:
                logger.info(f"Model {model_name} downloaded successfully.")
                if progress_callback:
                    progress_callback(100, "done")
                return True
            else:
                logger.error(f"Failed to download model {model_name}.")
                if progress_callback:
                    progress_callback(-1, "error")
                return False
        except Exception as e:
            logger.error(f"Error downloading model {model_name}: {e}")
            if progress_callback:
                progress_callback(-1, "error")
            return False

    def status(self):
        """
        Returns True if Ollama is running (by checking /api/tags), False otherwise.
        """
        url = f"{self.base_url}/api/tags"
        if config.DEBUG:
            print(f"[DEBUG] OllamaClient.status() requesting: {url}")
        try:
            resp = requests.get(url, timeout=5)
            if config.DEBUG:
                print(f"[DEBUG] OllamaClient.status() response status: {resp.status_code}")
            return resp.status_code == 200
        except Exception as e:
            if config.DEBUG:
                print(f"[DEBUG] OllamaClient.status() error: {e}")
            return False

    def start(self):
        """
        Starts Ollama as a subprocess (if not already running).
        """
        if self.status():
            return True  # Already running
        try:
            # Start Ollama in server mode
            self.ollama_process = subprocess.Popen(
                ["ollama", "serve"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL
            )
            return True
        except Exception as e:
            print(f"Failed to start Ollama: {e}")
            return False

    def stop(self):
        """
        Stops Ollama subprocess (if started by this client).
        """
        if self.ollama_process:
            self.ollama_process.terminate()
            self.ollama_process.wait(timeout=10)
            self.ollama_process = None
            return True
        # If not started by this client, try to kill by port (advanced: not implemented here)
        return False

    def restart(self):
        """
        Restarts Ollama subprocess.
        """
        self.stop()
        return self.start() 