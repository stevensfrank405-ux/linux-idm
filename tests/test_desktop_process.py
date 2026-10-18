"""Regressions for bridge readiness and startup-error reporting on both runners."""
import socket
import sys
import tempfile
import unittest
from pathlib import Path
from desktop_process import DesktopProcess

class ReadinessTests(unittest.TestCase):
    def port(self):
        with socket.socket() as sock:
            sock.bind(('127.0.0.1',0))
            return sock.getsockname()[1]

    def test_waits_through_connection_refusal_and_checks_authenticated_queue(self):
        port=self.port()
        # Simulate a slow GUI startup; a visible helper window is not bridge readiness.
        code=f'''
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
time.sleep(.4)
class Handler(BaseHTTPRequestHandler):
 def log_message(self,*args): pass
 def do_GET(self):
  if self.path=='/':
   body=b"const token='fixture-pairing-code';"
  elif self.path=='/api/downloads' and self.headers.get('X-Token')=='fixture-pairing-code':
   body=b'[]'
  else:
   self.send_error(403);return
  self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
HTTPServer(('127.0.0.1',{port}),Handler).serve_forever()
'''
        with tempfile.TemporaryDirectory() as directory:
            desktop=DesktopProcess([sys.executable,'-c',code],Path(directory),port,use_environment=False)
            try:
                desktop.start()
                self.assertEqual(desktop.ready(),'fixture-pairing-code')
                self.assertIsNone(desktop.process.poll())
            finally: desktop.stop()

    def test_reports_process_exit_and_original_startup_error(self):
        with tempfile.TemporaryDirectory() as directory:
            desktop=DesktopProcess([sys.executable,'-c',"import sys;print('renderer initialization failed',flush=True);sys.exit(7)"],Path(directory),self.port(),use_environment=False)
            try:
                desktop.start()
                with self.assertRaisesRegex(AssertionError,'app exited with code 7') as error:
                    desktop.ready()
                self.assertIn('renderer initialization failed',str(error.exception))
            finally: desktop.stop()

if __name__=='__main__': unittest.main()
