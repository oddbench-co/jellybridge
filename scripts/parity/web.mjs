// Drives a headless Chrome at an exact viewport and scale, signed in to
// jellyfin-web with the app's session, and runs a step script:
//   go <hash>        navigate to #<hash>
//   wait <ms>
//   js <code>        evaluate in the page
//   click <x> <y>    CSS px
//   move <x> <y>
//   shot <file>      screenshot the viewport
import { spawn } from 'node:child_process';
import { readFileSync, writeFileSync, mkdtempSync } from 'node:fs';
const [,, sessionPath, stepsPath, width, height] = process.argv;
const session = JSON.parse(readFileSync(sessionPath, 'utf8'));
const server = (session.serverUrl || session.server_url).replace(/\/$/, '');
const token = session.token, userId = session.userId || session.user_id;
const info = await (await fetch(server + '/System/Info/Public')).json();
const profile = mkdtempSync((process.env.TMPDIR || '/tmp') + '/jellybridge-parity-');
const chrome = spawn('/usr/bin/chromium', ['--headless=new', '--remote-debugging-port=9333', `--user-data-dir=${profile}`, '--force-color-profile=srgb', '--disable-gpu', '--mute-audio', 'about:blank'], { stdio: 'ignore' });
const sleep = ms => new Promise(r => setTimeout(r, ms));
let target;
for (let i = 0; i < 50 && !target; i++) { await sleep(100); try { target = (await (await fetch('http://127.0.0.1:9333/json')).json()).find(t => t.type === 'page'); } catch {} }
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise(r => ws.onopen = r);
let id = 0; const pending = new Map();
const requests = [];
ws.onmessage = e => { const m = JSON.parse(e.data); if (m.method === 'Network.requestWillBeSent') requests.push(m.params.request.method + ' ' + m.params.request.url); if (m.id && pending.has(m.id)) { pending.get(m.id)(m); pending.delete(m.id); } };
const send = (method, params = {}) => new Promise(r => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
await send('Emulation.setDeviceMetricsOverride', { width: +width, height: +height, deviceScaleFactor: 1.5, mobile: false, screenWidth: +(process.env.SCREEN_W || 2560), screenHeight: +(process.env.SCREEN_H || 1440) });
await send('Page.enable');
await send('Page.navigate', { url: server + '/web/' });
await sleep(1500);
const creds = { Servers: [{ ManualAddress: server, LocalAddress: server, Id: info.Id, Name: info.ServerName, UserId: userId, AccessToken: token, DateLastAccessed: Date.now(), LastConnectionMode: 2, manualAddressOnly: true }] };
await send('Runtime.evaluate', { expression: `localStorage.setItem('jellyfin_credentials', ${JSON.stringify(JSON.stringify(creds))}); localStorage.setItem('enableAutoLogin','true'); localStorage.setItem('_deviceId2','jellybridge-rust-desktop');` });
await send('Page.navigate', { url: server + '/web/index.html#/home' });
await sleep(500);
await send('Page.reload', { ignoreCache: false });
await sleep(3000);
for (const line of readFileSync(stepsPath, 'utf8').split('\n')) {
  const [cmd, ...rest] = line.trim().split(' ');
  if (!cmd || cmd.startsWith('#')) continue;
  if (cmd === 'go') await send('Runtime.evaluate', { expression: `location.hash=${JSON.stringify(rest.join(' '))}` });
  else if (cmd === 'wait') await sleep(+rest[0]);
  else if (cmd === 'js') { const r = await send('Runtime.evaluate', { expression: rest.join(' '), awaitPromise: true, returnByValue: true }); if (r.result?.result?.value !== undefined) console.log(JSON.stringify(r.result.result.value)); }
  else if (cmd === 'move') await send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: +rest[0], y: +rest[1] });
  else if (cmd === 'click') { for (const type of ['mousePressed', 'mouseReleased']) await send('Input.dispatchMouseEvent', { type, x: +rest[0], y: +rest[1], button: 'left', clickCount: 1 }); }
  else if (cmd === 'net') { await send('Network.enable'); requests.length = 0; }
  else if (cmd === 'requests') { for (const r of requests) if (!rest[0] || r.includes(rest[0])) console.log(r.replace(/api_key=[^&]+/, 'api_key=…')); requests.length = 0; }
  else if (cmd === 'size') { await send('Emulation.setDeviceMetricsOverride', { width: +rest[0], height: +rest[1], deviceScaleFactor: 1.5, mobile: false, screenWidth: +(process.env.SCREEN_W || 2560), screenHeight: +(process.env.SCREEN_H || 1440) }); }
  else if (cmd === 'shot') { const r = await send('Page.captureScreenshot', { format: 'png' }); writeFileSync(rest[0], Buffer.from(r.result.data, 'base64')); }
}
ws.close(); chrome.kill();
