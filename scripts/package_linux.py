#!/usr/bin/env python3
"""Compatibility wrapper for the desktop packager."""
import subprocess
import sys
from pathlib import Path
subprocess.run([sys.executable, str(Path(__file__).with_name('package_desktop.py')), '--platform', 'linux'], check=True)
