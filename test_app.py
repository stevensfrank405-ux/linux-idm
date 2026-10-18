import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import subprocess
import urllib.request
import urllib.error
import json
import re
import socket

PAYLOAD = bytes(range(256)) * 8192


class Source(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        if self.path == '/missing':
            self.send_error(404)
            return
        offset = int(self.headers.get('Range', 'bytes=0-')[6:-1]) if self.path != '/no-range' else 0
        self.send_response(206 if offset else 200)
        self.send_header('ETag', '"fixture-v1"')
        self.send_header('Content-Length', str(len(PAYLOAD) - offset))
        if offset:
            self.send_header('Content-Range', f'bytes {offset}-{len(PAYLOAD)-1}/{len(PAYLOAD)}')
        self.end_headers()
        try:
            for i in range(offset, len(PAYLOAD), 8192):
                self.wfile.write(PAYLOAD[i:i+8192])
                self.wfile.flush()
                time.sleep(.002)
        except (BrokenPipeError, ConnectionResetError):
            pass


class Downloads(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ThreadingHTTPServer(('127.0.0.1', 0), Source)
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.url = f'http://127.0.0.1:{cls.server.server_port}'

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        self.base = f'http://127.0.0.1:{port}'
        self.command = [str(Path('target/release/opendownload').resolve()), '--port', str(port), '--downloads', str(self.root / 'files'), '--state', str(self.root / 'queue.json')]
        self.start()
        self.addCleanup(self.stop)

    def start(self):
        self.process = subprocess.Popen(self.command, stdout=subprocess.DEVNULL)
        for _ in range(200):
            try:
                page = urllib.request.urlopen(self.base, timeout=1).read().decode()
                self.token = re.search("const token='([^']+)'", page)[1]
                return
            except OSError:
                time.sleep(.02)
        self.fail('App did not start')

    def stop(self):
        self.process.terminate()
        self.process.wait(timeout=5)

    def api(self, path, data=None, token=True):
        request = urllib.request.Request(self.base + '/api/' + path,
            data=json.dumps(data).encode() if data is not None else None,
            headers={'Content-Type': 'application/json', **({'X-Token': self.token} if token else {})})
        return json.load(urllib.request.urlopen(request, timeout=5))

    def job(self):
        return self.api('downloads')[0]

    def wait_for(self, condition):
        deadline = time.monotonic() + 8
        while not condition():
            if time.monotonic() > deadline:
                self.fail('Download did not reach expected state')
            time.sleep(.01)

    def test_pause_resume_and_persistence(self):
        self.api('add', {'url': self.url + '/file'})
        self.wait_for(lambda: self.job()['downloaded'] > 0)
        self.api('pause', {'id': self.job()['id']})
        self.wait_for(lambda: (self.root / 'files/file.part').stat().st_size > 0)
        self.stop()
        self.start()
        self.assertEqual(self.job()['status'], 'paused')
        self.api('resume', {'id': self.job()['id']})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        self.assertEqual((self.root / 'files/file').read_bytes(), PAYLOAD)

    def test_server_ignores_range(self):
        self.api('add', {'url': self.url + '/no-range'})
        self.wait_for(lambda: self.job()['downloaded'] > 0)
        self.api('pause', {'id': self.job()['id']})
        time.sleep(.1)
        self.api('resume', {'id': self.job()['id']})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        self.assertEqual((self.root / 'files/no-range').read_bytes(), PAYLOAD)

    def test_existing_file_is_preserved(self):
        files = self.root / 'files'
        files.mkdir(exist_ok=True)
        (files / 'file').write_bytes(b'keep my file')
        self.api('add', {'url': self.url + '/file'})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        self.assertEqual((files / 'file').read_bytes(), b'keep my file')
        self.assertEqual((files / self.job()['name']).read_bytes(), PAYLOAD)

    def test_failure_and_invalid_url(self):
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.api('add', {'url': 'file:///etc/passwd'})
        self.assertEqual(error.exception.code, 400)
        with self.assertRaises(urllib.error.HTTPError) as error:
            self.api('add', {'url': self.url + '/file'}, token=False)
        self.assertEqual(error.exception.code, 403)
        self.api('add', {'url': self.url + '/missing'})
        self.wait_for(lambda: self.job()['status'] == 'failed')
        self.assertIn('404', self.job()['error'])


if __name__ == '__main__':
    unittest.main()
