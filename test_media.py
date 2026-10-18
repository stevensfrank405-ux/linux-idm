"""Real yt-dlp + FFmpeg integration: HTML extraction, DASH merge and lifecycle."""
import json
import os
from pathlib import Path
import subprocess
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import test_app

FIXTURES = Path(__file__).parent/'tests/fixtures/video'
PAGE = b'<!doctype html><html><head><title>OpenDownload video fixture</title></head><body><video controls><source src="/media/manifest.mpd" type="application/dash+xml"></video></body></html>'

class Source(BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_GET(self):
        if self.path in ('/page','/not-video'):
            body = PAGE if self.path=='/page' else b'<html><title>No video</title><body>Ordinary website</body></html>'
            self.send_response(200);self.send_header('Content-Type','text/html; charset=utf-8');self.send_header('Content-Length',str(len(body)));self.end_headers()
            self.wfile.write(body);return
        name = self.path.removeprefix('/media/').split('?')[0]
        if Path(name).name!=name or not (FIXTURES/name).is_file(): self.send_error(404);return
        body=(FIXTURES/name).read_bytes()
        offset=0
        if self.headers.get('Range'): offset=int(self.headers['Range'].split('=')[1].split('-')[0])
        self.send_response(206 if offset else 200)
        self.send_header('Content-Type','application/dash+xml' if name.endswith('.mpd') else 'video/mp4')
        self.send_header('Content-Length',str(len(body)-offset))
        if offset:self.send_header('Content-Range',f'bytes {offset}-{len(body)-1}/{len(body)}')
        self.end_headers()
        try:
            for index in range(offset,len(body),2048):
                self.wfile.write(body[index:index+2048]);self.wfile.flush()
                if '.m4s' in name:time.sleep(.025)
        except (BrokenPipeError,ConnectionResetError):pass

class Media(unittest.TestCase):
    start=test_app.Downloads.start
    stop=test_app.Downloads.stop
    api=test_app.Downloads.api
    job=test_app.Downloads.job
    setUp=test_app.Downloads.setUp
    @classmethod
    def setUpClass(cls):
        cls.server=ThreadingHTTPServer(('127.0.0.1',0),Source)
        cls.server.requests_lock=threading.Lock();cls.server.requests=[]
        cls.server.max_active=0;cls.server.payload=b''
        threading.Thread(target=cls.server.serve_forever,daemon=True).start()
        cls.url=f'http://127.0.0.1:{cls.server.server_port}'
    @classmethod
    def tearDownClass(cls):cls.server.shutdown();cls.server.server_close()
    def wait_for(self,condition):
        deadline=time.monotonic()+60
        while not condition():
            if time.monotonic()>deadline:self.fail('Video did not reach expected state: '+str(self.api('downloads')))
            time.sleep(.05)
    def complete(self):
        self.wait_for(lambda:self.job()['status'] in ('completed','failed'))
        self.assertEqual(self.job()['status'],'completed',self.job()['error'])
        self.assertTrue(self.job()['media'])
        file=self.root/'files'/self.job()['name']
        tools=Path(os.environ['OPENDOWNLOAD_TOOLS_DIR'])
        metadata=json.loads(subprocess.check_output([str(tools/('ffprobe.exe' if os.name=='nt' else 'ffprobe')),'-v','error','-show_entries','stream=codec_type','-of','json',str(file)],text=True))
        self.assertEqual({s['codec_type'] for s in metadata['streams']},{'video','audio'})
        self.assertEqual(file.stat().st_size,self.job()['downloaded'])
        self.assertFalse(list((self.root/'files').glob('.opendownload-*')))
        return file
    def test_html_page_extracts_and_merges_audio_video(self):
        result=self.api('add',{'url':self.url+'/page'})
        self.assertEqual(result['id'],self.job()['id'])
        self.complete()
        self.assertFalse((self.root/'files/page').exists())
    def test_dash_manifest_uses_media_engine(self):
        self.api('add',{'url':self.url+'/media/manifest.mpd'})
        self.complete()
    def test_pause_resume_video_fragments(self):
        self.api('add',{'url':self.url+'/page'})
        self.wait_for(lambda:self.job()['downloaded']>0)
        self.api('pause',{'id':self.job()['id']})
        self.wait_for(lambda:self.job()['status']=='paused')
        time.sleep(.6)
        count=self.job()['downloaded']
        time.sleep(.3)
        self.assertEqual(self.job()['downloaded'],count)
        self.api('resume',{'id':self.job()['id']})
        self.complete()
    def test_unsupported_page_fails_instead_of_saving_html(self):
        self.api('add',{'url':self.url+'/not-video'})
        self.wait_for(lambda:self.job()['status']=='failed')
        self.assertIn('Video download failed',self.job()['error'])
        self.assertFalse((self.root/'files/not-video').exists())
    def test_explicit_file_mode_preserves_html_downloads(self):
        self.api('add',{'url':self.url+'/page','kind':'file'})
        self.wait_for(lambda:self.job()['status']=='completed')
        self.assertEqual((self.root/'files/page').read_bytes(),PAGE)
        self.assertFalse(self.job()['media'])
    def test_bad_mode_rejected(self):
        import urllib.error
        with self.assertRaises(urllib.error.HTTPError) as error:self.api('add',{'url':self.url+'/page','kind':'unknown'})
        self.assertEqual(error.exception.code,400)
        self.assertEqual(self.api('downloads'),[])

if __name__=='__main__':unittest.main()
