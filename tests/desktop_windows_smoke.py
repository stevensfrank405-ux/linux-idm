"""Native Windows window/bridge/shutdown smoke test; run on a Windows desktop runner."""
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import urllib.request
from desktop_process import DesktopProcess

if os.name != 'nt':
    raise SystemExit('This test requires native Windows, not Wine.')
PAYLOAD = b'OpenDownload native Windows fixture\n' * 4096
class Fixture(BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-Length', str(len(PAYLOAD)))
        self.end_headers()
        try: self.wfile.write(PAYLOAD)
        except (BrokenPipeError, ConnectionResetError): pass

user32=ctypes.WinDLL('user32',use_last_error=True)
callback_type=ctypes.WINFUNCTYPE(wintypes.BOOL,wintypes.HWND,wintypes.LPARAM)
user32.EnumWindows.argtypes=[callback_type,wintypes.LPARAM]
user32.GetWindowThreadProcessId.argtypes=[wintypes.HWND,ctypes.POINTER(wintypes.DWORD)]
user32.IsWindowVisible.argtypes=[wintypes.HWND]
user32.GetWindowTextW.argtypes=[wintypes.HWND,wintypes.LPWSTR,ctypes.c_int]
user32.PostMessageW.argtypes=[wintypes.HWND,wintypes.UINT,wintypes.WPARAM,wintypes.LPARAM]

def find_window(pid):
    windows=[]
    @callback_type
    def callback(hwnd,_):
        process_id=wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd,ctypes.byref(process_id))
        title=ctypes.create_unicode_buffer(512)
        user32.GetWindowTextW(hwnd,title,len(title))
        if process_id.value==pid and user32.IsWindowVisible(hwnd) and title.value=='OpenDownload': windows.append(hwnd)
        return True
    user32.EnumWindows(callback,0)
    return windows[0] if windows else None

with tempfile.TemporaryDirectory(prefix='opendownload-windows-') as directory:
    root=Path(directory)
    fixture=ThreadingHTTPServer(('127.0.0.1',0),Fixture)
    threading.Thread(target=fixture.serve_forever,daemon=True).start()
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
    command=[str(Path('target/release/opendownload.exe').resolve()),'--port',str(port),'--downloads',str(root/'files'),'--state',str(root/'queue.json')]
    desktop=DesktopProcess(command,root,port)
    process=desktop.start()
    try:
        token=desktop.ready()
        hwnd=desktop.wait(lambda:find_window(process.pid),'Native Windows window did not appear')
        base=f'http://127.0.0.1:{port}'
        request=urllib.request.Request(base+'/api/add',data=json.dumps({'url':f'http://127.0.0.1:{fixture.server_port}/windows.bin'}).encode(),headers={'X-Token':token,'Content-Type':'application/json'})
        assert json.load(urllib.request.urlopen(request))['ok']
        def completed():
            return (root/'files/windows.bin').exists() and list(json.loads((root/'queue.json').read_text()).values())[0]['status']=='completed'
        desktop.wait(completed,'Windows desktop engine transfer failed')
        assert (root/'files/windows.bin').read_bytes()==PAYLOAD
        user32.PostMessageW(hwnd,0x0010,0,0)  # WM_CLOSE: normal native close.
        assert process.wait(timeout=15)==0
        assert (root/'queue.bridge-key').read_text()==token
        process=desktop.start()
        assert desktop.ready()==token
        hwnd=desktop.wait(lambda:find_window(process.pid),'Windows window restart failed')
        assert completed()
        user32.PostMessageW(hwnd,0x0010,0,0)
        assert process.wait(timeout=15)==0
        print('PASS: native Windows window, desktop-engine file integrity, graceful close, queue restoration, and persistent pairing.')
    except Exception as error:
        desktop.report_failure(error)
        raise
    finally:
        desktop.stop()
        fixture.shutdown();fixture.server_close()
