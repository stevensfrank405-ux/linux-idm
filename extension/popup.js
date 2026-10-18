const api = globalThis.browser || chrome;
const el = id => document.getElementById(id);
const status = message => { el('status').textContent = message; };
async function send(url) {
  const result = await api.runtime.sendMessage({ type: 'download', url });
  if (result.error) throw Error(result.error);
  status('Added to OpenDownload. The desktop progress window opens automatically.');
}
async function init() {
  const settings = await api.storage.local.get(['token', 'capture', 'lastError']);
  el('token').value = settings.token || '';
  el('capture').checked = settings.capture || false;
  if (settings.lastError) { status(settings.lastError); await api.storage.local.remove('lastError'); }
}
el('pair').onclick = async () => {
  try {
    const token = el('token').value.trim();
    const response = await fetch('http://127.0.0.1:8765/api/downloads', { headers: { 'X-Token': token }, signal: AbortSignal.timeout(5000) });
    if (!response.ok) throw Error('Pairing rejected. Copy the current token from the dashboard.');
    await api.storage.local.set({ token }); status('Paired with OpenDownload.');
  } catch (error) { status(error.message); }
};
el('capture').onchange = async () => { await api.storage.local.set({ capture: el('capture').checked }); };
el('form').onsubmit = async event => { event.preventDefault(); try { await send(el('url').value.trim()); } catch (error) { status(error.message); } };
el('page-video').onclick = async () => {
  try {
    const [tab] = await api.tabs.query({ active: true, currentWindow: true });
    const result = await api.runtime.sendMessage({ type: 'download', url: tab.url, kind: 'video' });
    if (result.error) throw Error(result.error);
    status('Video added. Watch extraction and download progress in the desktop window.');
  } catch (error) { status(error.message); }
};
el('scan').onclick = async () => {
  try {
    const [tab] = await api.tabs.query({ active: true, currentWindow: true });
    const results = await api.scripting.executeScript({ target: { tabId: tab.id }, func: scanPage });
    el('items').replaceChildren();
    const items = results.flatMap(result => result.result || []);
    for (const item of items) {
      const row = document.createElement('div'); row.className = 'item';
      const label = document.createElement('span'); label.textContent = item.label; label.title = item.url;
      const button = document.createElement('button'); button.textContent = '↓'; button.title = 'Download ' + item.label;
      button.onclick = async () => { button.disabled = true; try { await send(item.url); } catch (error) { status(error.message); } finally { button.disabled = false; } };
      row.append(label, button); el('items').append(row);
    }
    status(items.length ? `${items.length} URLs found. Choose a file or video page URL. Unsupported pages show an error in the app.` : 'No transferable URLs found. Blob streams, DRM, and private browser sessions are not supported.');
  } catch (error) { status('Cannot scan this page: ' + error.message); }
};
init().catch(error => status(error.message));
