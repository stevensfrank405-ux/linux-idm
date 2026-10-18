// Requires Node 22+ and Chromium. Tests the dashboard and media scanner in a real browser.
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:http';
import assert from 'node:assert/strict';

const temporary = await mkdtemp(join(tmpdir(), 'opendownload-browser-'));
const payload = Buffer.from('OpenDownload browser integration fixture\n');
const fixture = createServer((request, response) => {
  if (request.url === '/page') {
    response.setHeader('Content-Type', 'text/html');
    response.end('<!doctype html><a download="example.txt" href="/example.txt">Example file</a>');
  } else { response.setHeader('Content-Length', payload.length); response.end(payload); }
});
await new Promise(resolve => fixture.listen(0, '127.0.0.1', resolve));
const source = `http://127.0.0.1:${fixture.address().port}`;
const app = spawn(resolve(process.env.OPENDOWNLOAD_BIN || 'target/release/opendownload' + (process.platform === 'win32' ? '.exe' : '')), ['--headless', '--downloads', join(temporary, 'files'), '--state', join(temporary, 'queue.json')], { stdio: 'ignore' });
const browser = spawn(process.env.CHROMIUM || 'chromium', ['--headless=new', '--no-sandbox', '--disable-gpu', '--no-first-run',  '--disable-dev-shm-usage', `--user-data-dir=${join(temporary, 'profile')}`, '--remote-debugging-port=9224',  'about:blank'], { stdio: 'ignore' });
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(fn) { for (let i=0; i<200; i++) { try { const result = await fn(); if (result) return result; } catch {} await pause(50); } throw Error('Timed out waiting for browser/app readiness'); }
const sockets = [];
async function attach(target) {
  const socket = new WebSocket(target.webSocketDebuggerUrl); sockets.push(socket);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
  let next = 1; const callbacks = new Map();
  socket.onmessage = event => { const result = JSON.parse(event.data); if (result.id) { const callback = callbacks.get(result.id); callbacks.delete(result.id); if (result.error) callback.reject(Error(result.error.message)); else callback.resolve(result.result); } };
  return async (method, params = {}) => new Promise((resolve, reject) => { const id = next++; callbacks.set(id, {resolve, reject}); socket.send(JSON.stringify({id,method,params})); });
}
async function evaluate(cdp, expression) {
  const result = await cdp('Runtime.evaluate', {expression, awaitPromise:true, returnByValue:true});
  if (result.exceptionDetails) throw Error(JSON.stringify(result.exceptionDetails));
  return result.result.value;
}
try {
  const html = await until(async () => { const response = await fetch('http://127.0.0.1:8765'); return response.ok && response.text(); });
  const token = html.match(/const token='([^']+)'/)[1];
  const targets = () => fetch('http://127.0.0.1:9224/json/list').then(r=>r.json());
  const pageTarget = await until(async () => (await targets()).find(t=>t.type==='page' && t.url==='about:blank'));
  const cdp = await attach(pageTarget);
  await cdp('Page.navigate', {url:source+'/page'});
  await until(async ()=>await evaluate(cdp, 'document.readyState==="complete" && !!document.querySelector("a[download]")'));
  const scanner = await readFile('extension/scanner.js','utf8');
  const items = await evaluate(cdp, scanner + '; scanPage()');
  assert.equal(items.length,1);
  assert.equal(items[0].url,source+'/example.txt');
  assert.equal(items[0].label,'example.txt');
  await cdp('Page.navigate', {url:'http://127.0.0.1:8765'});
  await until(async()=>await evaluate(cdp,'!!document.getElementById("form")'));
  await evaluate(cdp, `document.getElementById('url').value=${JSON.stringify(source+'/example.txt')};document.getElementById('form').requestSubmit()`);
  await until(async()=>await evaluate(cdp, `document.querySelector('.row .meta')?.textContent.includes('completed')`));
  assert.deepEqual(await readFile(join(temporary,'files/example.txt')),payload);
  await evaluate(cdp, `document.getElementById('pairing').click()`);
  assert.equal(await evaluate(cdp, `!document.getElementById('pair-token').hidden && document.getElementById('pair-token').textContent.includes(token)`),true);
  await evaluate(cdp, `document.querySelector('[data-filter="paused"]').click()`);
  assert.equal(await evaluate(cdp, `!!document.querySelector('.empty')`),true);
  await cdp('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
  assert.equal(await evaluate(cdp, 'document.documentElement.scrollWidth <= window.innerWidth'),true);
  console.log('PASS: real Chromium dashboard download, file integrity, pairing control, status filtering, mobile layout, and media scanner.');

} finally {
  for (const socket of sockets) socket.close();
  app.kill(); browser.kill(); fixture.closeAllConnections(); fixture.close();
  await pause(500);
  await rm(temporary, {recursive:true,force:true,maxRetries:3,retryDelay:200});
}
