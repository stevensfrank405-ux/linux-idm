/* All transfers are explicitly sent to the loopback app; no remote service. */
const api = globalThis.browser || chrome;
const endpoint = 'http://127.0.0.1:8765';
function validUrl(value) {
  try { const url = new URL(value); return ['http:', 'https:'].includes(url.protocol) && !url.username && !url.password; }
  catch { return false; }
}
async function sendDownload(url) {
  if (!validUrl(url)) throw Error('Only direct HTTP(S) URLs are supported. Blob and data URLs cannot be transferred.');
  const { token } = await api.storage.local.get('token');
  if (!token) throw Error('Pair the extension with OpenDownload first.');
  const response = await fetch(endpoint + '/api/add', {
    method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Token': token },
    body: JSON.stringify({ url }), signal: AbortSignal.timeout(5000)
  });
  const result = await response.json();
  if (!response.ok) throw Error(result.error || 'OpenDownload rejected the download.');
  return result;
}
async function badge(text) { await api.action.setBadgeText({ text }); }
api.runtime.onInstalled.addListener(() => {
  api.contextMenus.removeAll().then(() => api.contextMenus.create({
    id: 'opendownload', title: 'Download with OpenDownload', contexts: ['link', 'image', 'video', 'audio']
  }));
});
api.contextMenus.onClicked.addListener(async info => {
  if (info.menuItemId !== 'opendownload') return;
  try { await sendDownload(info.srcUrl || info.linkUrl); await badge(''); }
  catch (error) { await api.storage.local.set({ lastError: error.message }); await badge('!'); }
});
api.runtime.onMessage.addListener((message, _sender, reply) => {
  if (message.type !== 'download') return false;
  sendDownload(message.url).then(() => reply({ ok: true })).catch(error => reply({ error: error.message }));
  return true;
});
api.downloads.onCreated.addListener(async item => {
  const { capture } = await api.storage.local.get('capture');
  if (!capture || !validUrl(item.url)) return;
  try { await api.downloads.pause(item.id); }
  catch (error) {
    await api.storage.local.set({ lastError: 'Browser transfer already finished or cannot be paused; kept in the browser.' });
    return;
  }
  try { await sendDownload(item.url); }
  catch (error) {
    await api.downloads.resume(item.id).catch(() => {});
    await api.storage.local.set({ lastError: 'Browser download resumed: ' + error.message });
    await badge('!');
    return;
  }
  try { await api.downloads.cancel(item.id); await badge(''); }
  catch (error) {
    await api.storage.local.set({ lastError: 'Added to OpenDownload, but the paused browser copy could not be cancelled. Cancel it manually.' });
    await badge('!');
  }
});
