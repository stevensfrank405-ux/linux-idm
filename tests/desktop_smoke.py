"""Drive the real native Linux desktop window. Run under xvfb-run or a test DISPLAY."""
import json
import ctypes
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

PAYLOAD = bytes(range(256)) * 16384
class Fixture(BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-Length', str(len(PAYLOAD)))
        self.end_headers()
        try:
            for offset in range(0, len(PAYLOAD), 8192):
                self.wfile.write(PAYLOAD[offset:offset+8192])
                time.sleep(.003)
        except (BrokenPipeError, ConnectionResetError): pass

def xdo(*arguments):
    return subprocess.check_output(['xdotool', *map(str, arguments)], text=True).strip()

def close_window(window_id):
    # Send the same WM_DELETE_WINDOW protocol event as a window manager's close button.
    # xdotool windowclose destroys the X window directly and is not a normal close.
    x11 = ctypes.CDLL('libX11.so.6')
    x11.XOpenDisplay.restype = ctypes.c_void_p
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
    x11.XInternAtom.restype = ctypes.c_ulong
    x11.XInternAtom.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_int]
    class Message(ctypes.Structure):
        _fields_ = [('type',ctypes.c_int),('serial',ctypes.c_ulong),('send_event',ctypes.c_int),('display',ctypes.c_void_p),('window',ctypes.c_ulong),('message_type',ctypes.c_ulong),('format',ctypes.c_int),('data',ctypes.c_long*5)]
    class Event(ctypes.Union):
        _fields_ = [('message',Message),('pad',ctypes.c_long*24)]
    display=x11.XOpenDisplay(None)
    if not display: raise RuntimeError('Cannot connect to DISPLAY')
    event=Event()
    event.message=Message(type=33,send_event=1,display=display,window=int(window_id),message_type=x11.XInternAtom(display,b'WM_PROTOCOLS',0),format=32)
    event.message.data[0]=x11.XInternAtom(display,b'WM_DELETE_WINDOW',0)
    x11.XSendEvent.argtypes=[ctypes.c_void_p,ctypes.c_ulong,ctypes.c_int,ctypes.c_long,ctypes.POINTER(Event)]
    x11.XSendEvent(display,int(window_id),0,0,ctypes.byref(event))
    x11.XFlush.argtypes=[ctypes.c_void_p];x11.XFlush(display)
    x11.XCloseDisplay.argtypes=[ctypes.c_void_p];x11.XCloseDisplay(display)

with tempfile.TemporaryDirectory(prefix='opendownload-native-') as directory:
    root = Path(directory)
    fixture = ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
    threading.Thread(target=fixture.serve_forever, daemon=True).start()
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0)); port = sock.getsockname()[1]
    command = [str(Path(os.environ.get('OPENDOWNLOAD_BIN', 'target/release/opendownload')).resolve()), '--port', str(port), '--downloads', str(root/'files'), '--state', str(root/'queue.json')]
    desktop = DesktopProcess(command, root, port)
    process = desktop.start()
    try:
        def window():
            try: return xdo('search', '--all', '--onlyvisible', '--pid', process.pid, '--name', '^OpenDownload$').splitlines()[0]
            except subprocess.CalledProcessError: return None
        desktop.ready()
        window_id = desktop.wait(window, 'Native window did not appear')
        xdo('windowfocus', '--sync', window_id)
        time.sleep(.3)
        xdo('key', 'ctrl+l')
        xdo('type', '--clearmodifiers', '--delay', '1', f'http://127.0.0.1:{fixture.server_port}/native-download.bin')
        xdo('key', 'Return')
        def queue():
            try: return list(json.loads((root/'queue.json').read_text()).values())
            except (FileNotFoundError, json.JSONDecodeError): return []
        desktop.wait(lambda: any(j['status']=='downloading' for j in queue()), 'Native add action did not start a transfer')
        def progress_window():
            try: return xdo('search', '--all', '--onlyvisible', '--pid', process.pid, '--name', '^Download ').splitlines()[0]
            except subprocess.CalledProcessError: return None
        popup = desktop.wait(progress_window, 'Separate native download progress window did not appear')
        assert popup != window_id
        desktop.wait(lambda: any(j['status']=='completed' for j in queue()), 'Native transfer did not complete')
        popup_screenshot = os.environ.get('OPENDOWNLOAD_POPUP_SCREENSHOT')
        if popup_screenshot: subprocess.run(['import', '-window', popup, popup_screenshot], check=True)
        close_window(popup)
        desktop.wait(lambda: not progress_window(), 'Closing progress window did not dismiss it')
        assert process.poll() is None
        xdo('windowfocus', '--sync', window_id)
        assert (root/'files/native-download.bin').read_bytes() == PAYLOAD
        # Both native dialogs must open without crashing or losing the engine.
        xdo('key', 'ctrl+b')
        time.sleep(.2)
        xdo('key', 'ctrl+b')
        xdo('key', 'ctrl+comma')
        time.sleep(.2)
        assert process.poll() is None
        xdo('key', 'ctrl+comma')
        time.sleep(.2)
        screenshot = os.environ.get('OPENDOWNLOAD_SCREENSHOT')
        if screenshot: subprocess.run(['import', '-window', window_id, screenshot], check=True)
        close_window(window_id)
        assert process.wait(timeout=10) == 0
        # The same native executable must reopen the queue and keep pairing stable.
        key = (root/'queue.bridge-key').read_text()
        process = desktop.start()
        desktop.ready()
        desktop.wait(window, 'Native restart failed')
        assert (root/'queue.bridge-key').read_text() == key
        assert queue()[0]['status'] == 'completed'
        close_window(window())
        assert process.wait(timeout=10) == 0
        print('PASS: native desktop window, keyboard URL entry, real file integrity, separate progress popup, dialogs, graceful close, queue restore, and persistent browser pairing.')
    except Exception as error:
        if process.poll() is None:
            subprocess.run(['import', '-window', 'root', str(desktop.log_path.parent/'linux-desktop-failure.png')], check=False)
        desktop.report_failure(error)
        raise
    finally:
        desktop.stop()
        fixture.shutdown();fixture.server_close()
