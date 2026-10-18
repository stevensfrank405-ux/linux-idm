// Requires Node 22+ and Chromium. Runs the actual unpacked extension against the Rust app.
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:http';
import assert from 'node:assert/strict';

const temporary = await mkdtemp(join(tmpdir(), 'opendownload-browser-'));
const payload = Buffer.from('OpenDownload browser integration fixture\n');
const capturedPayload = Buffer.alloc(256 * 1024, 42);
const fixture = createServer((request, response) => {
  if (request.url === '/captured.txt') {
    response.setHeader('Content-Type', 'application/octet-stream');
    response.setHeader('Content-Length', capturedPayload.length);
    let offset=0;
    const timer=setInterval(()=>{response.write(capturedPayload.subarray(offset,offset+8192));offset+=8192;if(offset>=capturedPayload.length){clearInterval(timer);response.end();}},20);
    response.on('close',()=>clearInterval(timer));
  } else if (request.url === '/page') {
    response.setHeader('Content-Type', 'text/html');
    response.end('<!doctype html><a download="example.txt" href="/example.txt">Example file</a>');
  } else { response.setHeader('Content-Length', payload.length); response.end(payload); }
});
await new Promise(resolve => fixture.listen(0, '127.0.0.1', resolve));
const source = `http://127.0.0.1:${fixture.address().port}`;
const app = spawn(resolve('target/release/opendownload'), ['--downloads', join(temporary, 'files'), '--state', join(temporary, 'queue.json')], { stdio: 'ignore' });
const browser = spawn(process.env.CHROMIUM || 'chromium', ['--headless=new', '--no-sandbox', '--disable-gpu', '--no-first-run', '--enable-unsafe-extension-debugging', '--disable-dev-shm-usage', `--user-data-dir=${join(temporary, 'profile')}`, '--remote-debugging-port=9224', `--disable-extensions-except=${resolve('extension')}`, `--load-extension=${resolve('extension')}`, 'about:blank'], { stdio: 'ignore' });
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
  const version = await until(async () => fetch('http://127.0.0.1:9224/json/version').then(r=>r.json()));
  const browserCdp = await attach(version);
  await browserCdp('Extensions.loadUnpacked', {path:resolve('extension')});
  const worker = await until(async () => (await targets()).find(t => t.type === 'service_worker' && t.url.includes('background.js')));
  const cdp = await attach(worker);
  await evaluate(cdp, `api.storage.local.set({token:${JSON.stringify(token)}})`);
  await evaluate(cdp, `sendDownload(${JSON.stringify(source+'/example.txt')})`);
  const queue = () => fetch('http://127.0.0.1:8765/api/downloads', {headers:{'X-Token':token}}).then(r=>r.json());
  await until(async () => (await queue()).some(j=>j.status==='completed'));
  assert.deepEqual(await readFile(join(temporary,'files/example.txt')), payload);
  assert.equal(await evaluate(cdp, `sendDownload('blob:https://example.com/private').then(()=>false,()=>true)`), true);
  // Exercise opt-in capture in Chromium's real downloads API.
  await evaluate(cdp, `api.storage.local.set({capture:true})`);
  await evaluate(cdp, `api.downloads.download({url:${JSON.stringify(source+'/captured.txt')},saveAs:false})`);
  await until(async () => (await queue()).some(j=>j.name==='captured.txt' && j.status==='completed'));
  assert.deepEqual(await readFile(join(temporary,'files/captured.txt')), capturedPayload);
  const browserCopies = await evaluate(cdp, `api.downloads.search({url:${JSON.stringify(source+'/captured.txt')}})`);
  assert.equal(browserCopies[0].state,'interrupted');
  // Open the actual popup, verify pairing, and run the DOM scanner on a normal tab.
  const pageTarget = (await targets()).find(t=>t.type==='page');
  const pageCdp = await attach(pageTarget);
  await pageCdp('Page.navigate', {url:source+'/page'});
  await until(async ()=>await evaluate(pageCdp, 'document.readyState==="complete" && !!document.querySelector("a[download]")'));
  const extensionId = new URL(worker.url).host;
  const created = await pageCdp('Target.createTarget', {url:`chrome-extension://${extensionId}/popup.html`});
  const popupTarget = await until(async () => (await targets()).find(t=>t.id===created.targetId));
  const popupCdp = await attach(popupTarget);
  await until(async ()=>await evaluate(popupCdp, '!!document.getElementById("pair")'));
  await evaluate(popupCdp, `document.getElementById('pair').click()`);
  await until(async ()=>await evaluate(popupCdp, `document.getElementById('status').textContent.includes('Paired')`));
  // Popup DOM tested; automated popups do not grant an activeTab user gesture.
  console.log('PASS: Chromium extension pairing, HTTP handoff, byte integrity, opt-in capture, blob rejection, and popup pairing.');
} finally {
  for (const socket of sockets) socket.close();
  app.kill(); browser.kill(); fixture.closeAllConnections(); fixture.close();
  await pause(500);
  await rm(temporary, {recursive:true,force:true,maxRetries:3,retryDelay:200});
}
