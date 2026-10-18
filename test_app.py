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
import os

PAYLOAD = bytes(range(256)) * 8192


class Source(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        if self.path == '/missing':
            self.send_error(404)
            return
        range_header = self.headers.get('Range')
        with self.server.requests_lock:
            self.server.requests.append((self.path, range_header, self.headers.get('If-Range')))
        payload = PAYLOAD if not self.path.startswith('/versioned') else self.server.payload
        etag = '"fixture-v1"' if payload == PAYLOAD else '"fixture-v2"'
        bounded = range_header is not None and self.path != '/no-range'
        if self.path == '/lying-range' and range_header != 'bytes=0-0':
            bounded = False
        if self.headers.get('If-Range') and self.headers['If-Range'] != etag:
            bounded = False
        offset, end = 0, len(payload) - 1
        if bounded:
            first, last = range_header[6:].split('-')
            offset = int(first)
            if last:
                end = min(int(last), end)
        self.send_response(206 if bounded else 200)
        self.send_header('ETag', 'W/' + etag if self.path == '/weak' else etag)
        self.send_header('Content-Length', str(end - offset + 1))
        if bounded:
            claimed_end = end - 1 if self.path == '/bad-range' and range_header != 'bytes=0-0' else end
            self.send_header('Content-Range', f'bytes {offset}-{claimed_end}/{len(payload)}')
        self.end_headers()
        with self.server.requests_lock:
            self.server.active += 1
            self.server.max_active = max(self.server.max_active, self.server.active)
        try:
            for i in range(offset, end + 1, 8192):
                self.wfile.write(payload[i:min(i+8192, end+1)])
                self.wfile.flush()
                time.sleep(.005)
        except (BrokenPipeError, ConnectionResetError):
            pass
        finally:
            with self.server.requests_lock:
                self.server.active -= 1



class Downloads(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ThreadingHTTPServer(('127.0.0.1', 0), Source)
        cls.server.requests = []
        cls.server.active = 0
        cls.server.max_active = 0
        cls.server.requests_lock = threading.Lock()
        cls.server.payload = PAYLOAD
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.url = f'http://127.0.0.1:{cls.server.server_port}'

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def setUp(self):
        with self.server.requests_lock:
            self.server.requests.clear()
            self.server.max_active = 0
        self.server.payload = PAYLOAD
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        self.base = f'http://127.0.0.1:{port}'
        self.command = [str(Path(os.environ.get('OPENDOWNLOAD_BIN', 'target/release/opendownload' + ('.exe' if os.name == 'nt' else ''))).resolve()), '--headless', '--port', str(port), '--downloads', str(self.root / 'files'), '--state', str(self.root / 'queue.json')]
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
        self.wait_for(lambda: len(list((self.root / 'files').glob('*.segment-*'))) == 4)
        time.sleep(.05)
        self.stop()
        self.start()
        self.assertEqual(self.job()['status'], 'paused')
        self.api('resume', {'id': self.job()['id']})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        self.assertEqual((self.root / 'files/file').read_bytes(), PAYLOAD)
        with self.server.requests_lock:
            resumed = [r for path, r, validator in self.server.requests if path == '/file' and validator]
        initial = {f'bytes={i*len(PAYLOAD)//4}-{(i+1)*len(PAYLOAD)//4-1}' for i in range(4)}
        self.assertTrue(any(r not in initial for r in resumed), 'Expected resumed segment offsets')

    def test_server_ignores_range(self):
        self.api('add', {'url': self.url + '/no-range'})
        self.wait_for(lambda: self.job()['downloaded'] > 0)
        self.api('pause', {'id': self.job()['id']})
        time.sleep(.1)
        self.api('resume', {'id': self.job()['id']})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        self.assertEqual((self.root / 'files/no-range').read_bytes(), PAYLOAD)

    def test_parallel_ranges_and_integrity(self):
        self.api('add', {'url': self.url + '/parallel'})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        self.assertEqual(self.job()['segments'], 4)
        self.assertEqual((self.root / 'files/parallel').read_bytes(), PAYLOAD)
        with self.server.requests_lock:
            ranges = [r for path, r, validator in self.server.requests if path == '/parallel' and validator]
        expected = [f'bytes={i*len(PAYLOAD)//4}-{(i+1)*len(PAYLOAD)//4-1}' for i in range(4)]
        self.assertCountEqual(ranges, expected)
        self.assertGreaterEqual(self.server.max_active, 2)
        self.assertEqual(list((self.root / 'files').glob('*.segment-*')), [])

    def test_range_rejection_falls_back(self):
        self.api('add', {'url': self.url + '/lying-range'})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        self.assertEqual((self.root / 'files/lying-range').read_bytes(), PAYLOAD)
        self.assertEqual(self.job()['segments'], 0)

    def test_invalid_range_falls_back(self):
        self.api('add', {'url': self.url + '/bad-range'})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        self.assertEqual((self.root / 'files/bad-range').read_bytes(), PAYLOAD)
        self.assertEqual(self.job()['segments'], 0)

    def test_weak_validator_uses_single_stream(self):
        self.api('add', {'url': self.url + '/weak'})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        self.assertEqual(self.job()['segments'], 0)
        self.assertEqual((self.root / 'files/weak').read_bytes(), PAYLOAD)

    def test_changed_file_restarts_all_segments(self):
        self.api('add', {'url': self.url + '/versioned'})
        self.wait_for(lambda: self.job()['downloaded'] > 65536)
        self.api('pause', {'id': self.job()['id']})
        time.sleep(.1)
        self.stop()
        self.server.payload = bytes(reversed(PAYLOAD))
        self.start()
        self.api('resume', {'id': self.job()['id']})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        self.assertEqual((self.root / 'files/versioned').read_bytes(), self.server.payload)

    def test_single_connection_option(self):
        self.stop()
        self.command += ['--connections', '1']
        self.start()
        self.api('add', {'url': self.url + '/single'})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        self.assertEqual(self.job()['segments'], 0)
        self.assertEqual((self.root / 'files/single').read_bytes(), PAYLOAD)
        checkpoint = json.loads((self.root / 'queue.json').read_text())[self.job()['id']]
        self.assertEqual(checkpoint['status'], 'completed')
        self.assertEqual(checkpoint['speed'], 0)

    def test_resume_keeps_original_destination_after_settings_change(self):
        self.api('add', {'url': self.url + '/original.bin'})
        self.wait_for(lambda: self.job()['downloaded'] > 65536)
        self.api('pause', {'id': self.job()['id']})
        time.sleep(.1)
        self.stop()
        self.command[self.command.index('--downloads') + 1] = str(self.root / 'new-files')
        self.start()
        self.api('resume', {'id': self.job()['id']})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        self.assertEqual((self.root / 'files/original.bin').read_bytes(), PAYLOAD)
        self.assertFalse((self.root / 'new-files/original.bin').exists())

    def test_windows_safe_filename(self):
        self.api('add', {'url': self.url + '/CON'})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        self.assertEqual(self.job()['name'], 'download-CON')
        self.assertEqual((self.root / 'files/download-CON').read_bytes(), PAYLOAD)

    def test_remove_completed_keeps_file(self):
        self.api('add', {'url': self.url + '/keep.bin'})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        self.api('remove', {'id': self.job()['id']})
        self.assertEqual(self.api('downloads'), [])
        self.assertEqual((self.root / 'files/keep.bin').read_bytes(), PAYLOAD)

    def test_pairing_survives_restart(self):
        token = self.token
        self.stop()
        self.start()
        self.assertEqual(self.token, token)

    def test_second_instance_does_not_replace_queue(self):
        self.api('add', {'url': self.url + '/first.bin'})
        self.wait_for(lambda: self.job()['status'] == 'completed')
        before = (self.root / 'queue.json').read_bytes()
        checkpoint = json.loads(before)[self.job()['id']]
        self.assertEqual(checkpoint['status'], 'completed')
        self.assertEqual(checkpoint['speed'], 0)
        result = subprocess.run(self.command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.root / 'queue.json').read_bytes(), before)

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
