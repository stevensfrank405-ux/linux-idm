// Browser API contract tests against the actual Rust API. Node 22+, no npm dependencies.
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:http';
import vm from 'node:vm';
import assert from 'node:assert/strict';
const temporary = await mkdtemp(join(tmpdir(), 'opendownload-protocol-'));
const fixture = createServer((_request, response) => response.end('extension file content'));
await new Promise(resolve => fixture.listen(0, '127.0.0.1', resolve));
const source = `http://127.0.0.1:${fixture.address().port}`;
const app = spawn(resolve(process.env.OPENDOWNLOAD_BIN || 'target/release/opendownload' + (process.platform === 'win32' ? '.exe' : '')), ['--headless', '--downloads',join(temporary,'files'),'--state',join(temporary,'queue.json')], {stdio:'ignore'});
const pause = ms => new Promise(resolve => setTimeout(resolve,ms));
async function until(fn) { for(let i=0;i<100;i++){try{const value=await fn();if(value)return value;}catch{}await pause(30);}throw Error('Readiness timeout'); }
const events = {}, settings = {}, calls = [];
const event = name => ({addListener: callback => {events[name]=callback;}});
const api = {
  storage:{local:{get:async()=>({...settings}),set:async values=>Object.assign(settings,values)}},
  action:{setBadgeText:async value=>calls.push(['badge',value.text])},
  contextMenus:{removeAll:async()=>{},create:async value=>calls.push(['menu',value.id,value.contexts]),onClicked:event('click')},
  runtime:{onInstalled:event('installed'),onMessage:event('message')},
  downloads:{onCreated:event('created'),pause:async id=>calls.push(['pause',id]),cancel:async id=>calls.push(['cancel',id]),resume:async id=>calls.push(['resume',id])}
};
const context = vm.createContext({chrome:api,fetch,URL,AbortSignal,console});
try {
  const html = await until(async()=>{const r=await fetch('http://127.0.0.1:8765');return r.ok&&r.text();});
  const token = html.match(/const token='([^']+)'/)[1];
  settings.token = token;
  vm.runInContext(await readFile('extension/background.js','utf8'),context);
  const run = code=>vm.runInContext(code,context);
  events.installed(); await pause(10);
  assert.ok(calls.find(c=>c[0]==='menu')[2].includes('page'));
  await run(`sendDownload(${JSON.stringify(source+'/direct.txt')})`);
  const queue=()=>fetch('http://127.0.0.1:8765/api/downloads',{headers:{'X-Token':token}}).then(r=>r.json());
  await until(async()=>(await queue()).some(j=>j.status==='completed'));
  assert.equal(await readFile(join(temporary,'files/direct.txt'),'utf8'),'extension file content');
  await assert.rejects(run("sendDownload('blob:https://example.org/file')"), /HTTP/);
  await assert.rejects(run("sendDownload('https://user:password@example.org/file')"), /HTTP/);
  settings.token='wrong';
  await assert.rejects(run(`sendDownload(${JSON.stringify(source+'/rejected.txt')})`), /Unauthorized/);
  settings.token=token;
  // Opt-in is off initially and must not cancel regular browser transfers.
  await events.created({id:1,url:source+'/ignored.txt'});
  assert.equal(calls.some(c=>c[0]==='pause'),false);
  settings.capture=true;
  await events.created({id:2,url:source+'/captured.txt'});
  assert.deepEqual(calls.filter(c=>['pause','cancel','resume'].includes(c[0])),[['pause',2],['cancel',2]]);
  settings.token='wrong';
  await events.created({id:3,url:source+'/browser-fallback.txt'});
  assert.deepEqual(calls.filter(c=>c[1]===3),[['pause',3],['resume',3]]);
  settings.token=token;
  await events.click({menuItemId:'opendownload',srcUrl:source+'/image.png'});
  const reply=await new Promise(resolve=>events.message({type:'download',url:source+'/message.txt'},null,resolve));
  assert.equal(reply.ok,true);
  await events.click({menuItemId:'opendownload',pageUrl:source+'/current-page.txt'});
  const explicit=await new Promise(resolve=>events.message({type:'download',url:source+'/explicit-file.txt',kind:'file'},null,resolve));
  assert.equal(explicit.ok,true);
  await until(async()=>(await queue()).filter(j=>j.status==='completed').length===6);
  assert.equal((await queue()).find(j=>j.url.endsWith('/explicit-file.txt')).allow_html,true);
  console.log('PASS: real Rust API handoff, file integrity, URL/token rejection, context menu, message bridge, capture opt-in, cancellation and failure recovery. Browser APIs were mocked.');
} finally {
  app.kill();fixture.closeAllConnections();fixture.close();await pause(200);
  await rm(temporary,{recursive:true,force:true});
}
