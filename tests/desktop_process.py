"""Readiness and startup diagnostics for real desktop-process smoke tests."""
import json
import os
from pathlib import Path
import re
import subprocess
import time
import urllib.request

class DesktopProcess:
    def __init__(self, command, root, port):
        self.command, self.port = command, port
        directory = Path(os.environ.get('OPENDOWNLOAD_DIAGNOSTICS', root))
        directory.mkdir(parents=True, exist_ok=True)
        self.log_path = directory / ('windows-desktop.log' if os.name == 'nt' else 'linux-desktop.log')
        self.log = self.log_path.open('ab', buffering=0)
        self.process = None

    def start(self):
        self.process = subprocess.Popen(self.command, stdout=self.log, stderr=subprocess.STDOUT)
        return self.process

    def details(self):
        return self.log_path.read_text(encoding='utf-8', errors='replace')[-12000:]

    def wait(self, condition, message, timeout=20):
        deadline = time.monotonic() + timeout
        last_error = None
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise AssertionError(f'{message}: app exited with code {self.process.returncode}.\n{self.details()}')
            try:
                result = condition()
                if result:
                    return result
            except (OSError, ValueError) as error:
                last_error = error
            time.sleep(.05)
        raise AssertionError(f'{message}. Last observation: {last_error}\n{self.details()}')

    def ready(self):
        base = f'http://127.0.0.1:{self.port}'
        def check():
            with urllib.request.urlopen(base, timeout=1) as response:
                page = response.read().decode()
            match = re.search("const token='([^']+)'", page)
            if not match:
                raise ValueError('Bridge did not return its dashboard')
            token = match[1]
            request = urllib.request.Request(base + '/api/downloads', headers={'X-Token': token})
            with urllib.request.urlopen(request, timeout=1) as response:
                if not isinstance(json.load(response), list):
                    raise ValueError('Authenticated queue is not ready')
            return token
        return self.wait(check, 'Desktop bridge did not become ready')

    def stop(self):
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            self.process.wait(timeout=10)
        self.log.close()

    def report_failure(self, error):
        message = f'{error}\n{self.details()}'
        print(message, flush=True)
        if os.environ.get('GITHUB_ACTIONS') == 'true':
            escaped = message.replace('%', '%25').replace('\r', '%0D').replace('\n', '%0A')
            print(f'::error title=Desktop startup or smoke test failed::{escaped}', flush=True)
