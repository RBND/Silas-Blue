import subprocess
import time
import os
import signal

# Path to your target script
TARGET_SCRIPT = "bot.py"

def start_target():
    return subprocess.Popen(["python", TARGET_SCRIPT])

def stop_target(process):
    os.kill(process.pid, signal.SIGTERM)

def main():
    while True:
        print("Starting target app...")
        process = start_target()
        time.sleep(86400)  #Time to wait in seconds between restarts
        print("Restarting target app...")
        stop_target(process)

if __name__ == "__main__":
    main()
