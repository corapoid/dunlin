// Production app + actual AHP. Stdio only mutates the controlled source journal.
import assert from 'node:assert/strict';
import { createInterface } from 'node:readline';
import { chromium } from 'playwright-core';
const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
const iterator = lines[Symbol.asyncIterator]();
async function receive() { const line = await iterator.next(); assert.equal(line.done, false); return JSON.parse(line.value); }
const fixture = await receive();
async function source(command, extra = {}) { process.stdout.write(`${JSON.stringify({ command, ...extra })}\n`); assert.deepEqual(await receive(), { ok: command }); }
const browser = await chromium.launch({ executablePath: process.env.DUNLIN_CHROMIUM || chromium.executablePath(), headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
try {
  assert.ok(fixture.fullBytes > 900 * 1024);
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: 'reduce' });
  const page = await context.newPage(); page.setDefaultTimeout(15000);
  const errors = [], requests = [], snapshots = [];
  let maxFrame = 0;
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => assert.equal(new URL(request.url()).origin, fixture.origin));
  page.on('websocket', socket => {
    if (new URL(socket.url()).pathname !== '/v1/web/ahp') return;
    socket.on('framesent', ({ payload }) => requests.push(JSON.parse(String(payload))));
    socket.on('framereceived', ({ payload }) => {
      maxFrame = Math.max(maxFrame, Buffer.byteLength(String(payload)));
      const message = JSON.parse(String(payload));
      const values = message.result?.snapshot ? [message.result.snapshot] : message.result?.snapshots ?? [];
      for (const snapshot of values) if (snapshot.resource === fixture.chatUri) snapshots.push({ cursor: snapshot.state.turnsNextCursor, count: snapshot.state.turns.length, seq: snapshot.fromSeq });
    });
  });
  await page.addInitScript(() => {
    const NativeWebSocket = window.WebSocket, sockets = new Set();
    const fault = { held: 0, dropped: 0, dropNextPage: false, firstFetch: null, reply: null, page: null };
    window.historyFaultForAcceptance = fault;
    window.WebSocket = class extends NativeWebSocket {
      constructor(...args) {
        super(...args);
        if (new URL(this.url).pathname !== '/v1/web/ahp') return;
        sockets.add(this); this.addEventListener('close', () => sockets.delete(this));
        this.addEventListener('message', event => {
          const message = JSON.parse(event.data);
          if (fault.dropNextPage && message.method === 'action' && message.params.action.type === 'chat/turnsLoaded') {
            fault.dropNextPage = false; fault.dropped++; fault.page = { socket: this, data: event.data };
            event.stopImmediatePropagation(); return;
          }
          if (message.id === fault.firstFetch && fault.held === 0 && message.result) {
            // Hold one ORIGINAL RPC acknowledgement while actual live actions
            // continue. No response/action payload is fabricated or modified.
            fault.held++; fault.reply = { socket: this, data: event.data };
            event.stopImmediatePropagation();
          }
        }, { capture: true });
      }
      send(data) {
        if (new URL(this.url).pathname === '/v1/web/ahp') {
          const message = JSON.parse(data);
          if (message.method === 'fetchTurns' && fault.firstFetch === null) fault.firstFetch = message.id;
        }
        return super.send(data);
      }
    };
    window.releaseHistoryReplyForAcceptance = () => { const held = fault.reply; fault.reply = null; held.socket.dispatchEvent(new MessageEvent('message', { data: held.data })); };
    window.releaseOldHistoryPageForAcceptance = () => { const held = fault.page; fault.page = null; held.socket.dispatchEvent(new MessageEvent('message', { data: held.data })); };
    window.disconnectAhpForAcceptance = () => { const count = sockets.size; for (const socket of sockets) socket.close(4000, 'acceptance reconnect'); return count; };
  });
  await page.goto(fixture.origin);
  await page.getByLabel('Jednorazowy kod parowania').fill(fixture.ownerCode);
  await page.getByRole('button', { name: 'Połącz bezpiecznie', exact: true }).click();
  await page.getByRole('button', { name: 'Sesje', exact: true }).click();
  const status = page.getByTestId('ahp-status'), transcript = page.getByTestId('ahp-transcript');
  await status.getByText('Na żywo', { exact: true }).waitFor();
  await page.locator(`[data-testid="ahp-session"][data-session-uri="${fixture.sessionUri}"]`).click();
  const more = page.getByTestId('ahp-history-more');
  await more.waitFor();
  await page.waitForFunction(marker => document.querySelector('#ahp-transcript')?.textContent.includes(marker), fixture.markers.at(-1));
  const initialIds = await transcript.locator('[data-turn-id]').evaluateAll(nodes => nodes.map(node => node.dataset.turnId));
  assert.ok(initialIds.length > 0 && initialIds.length < fixture.turnIds.length, 'Adaptive tail must bound a valid history larger than one snapshot');
  assert.equal((await transcript.innerText()).includes(fixture.markers[0]), false);
  assert.ok(snapshots[0].cursor); assert.ok(snapshots[0].count < 20);
  assert.deepEqual(requests.find(r => r.method === 'subscribe').params.view, { turns: 32 });
  assert.equal(requests.filter(r => r.method === 'fetchTurns').length, 0);
  // Native button keyboard activation, with one real page acknowledgement held.
  await more.focus(); await more.press('Enter');
  await page.waitForFunction(() => window.historyFaultForAcceptance.held === 1);
  await transcript.getByText('Wczytywanie historii…', { exact: true }).waitFor();
  await source('live');
  await page.waitForFunction(marker => document.querySelector('#ahp-transcript')?.textContent.includes(marker), fixture.live);
  await source('complete');
  await page.waitForFunction(() => ![...document.querySelectorAll('#ahp-transcript h3')].some(node => node.textContent.includes('w toku')));
  await transcript.getByText('Wczytywanie historii…', { exact: true }).waitFor();
  await page.evaluate(() => window.releaseHistoryReplyForAcceptance());
  await page.waitForFunction(() => document.querySelector('[data-testid="ahp-history-more"]')?.disabled === false);
  assert.equal(requests.filter(r => r.method === 'fetchTurns').length, 1);
  // Lose one real page notification. RPC{} cannot invent success or replace the
  // transcript; the explicit refresh UI must remint its one-use cursor.
  await page.evaluate(() => { window.historyFaultForAcceptance.dropNextPage = true; });
  await more.click();
  const refresh = page.getByTestId('ahp-history-refresh'); await refresh.waitFor();
  assert.equal(await more.isDisabled(), true);
  assert.equal(requests.filter(r => r.method === 'fetchTurns').length, 2);
  await refresh.click();
  await page.waitForFunction(() => !document.querySelector('[data-testid="ahp-history-refresh"]') && document.querySelector('[data-testid="ahp-history-more"]')?.disabled === false);
  assert.ok(snapshots.length >= 2);
  const refreshed = snapshots.at(-1); assert.notEqual(refreshed.cursor, snapshots[0].cursor);
  const beforeLate = await transcript.locator('[data-turn-id]').evaluateAll(nodes => nodes.map(node => node.dataset.turnId));
  await page.evaluate(() => window.releaseOldHistoryPageForAcceptance());
  await page.evaluate(() => new Promise(requestAnimationFrame));
  assert.deepEqual(await transcript.locator('[data-turn-id]').evaluateAll(nodes => nodes.map(node => node.dataset.turnId)), beforeLate, 'Pre-refresh page must not overwrite a newer snapshot');
  async function loadAll(expectedCount) {
    for (let index = 0; index < 10 && await more.count(); index++) {
      const before = await transcript.locator('[data-turn-id]').count(); await more.click();
      await page.waitForFunction(() => !document.querySelector('[data-testid="ahp-history-more"]') || document.querySelector('[data-testid="ahp-history-more"]')?.disabled === false);
      assert.ok(await transcript.locator('[data-turn-id]').count() > before, 'Each explicit page must prepend retained older turns');
    }
    assert.equal(await more.count(), 0);
    const ids = await transcript.locator('[data-turn-id]').evaluateAll(nodes => nodes.map(node => node.dataset.turnId));
    assert.equal(ids.length, expectedCount); assert.equal(new Set(ids).size, ids.length);
    assert.deepEqual(ids.slice(0, fixture.turnIds.length), fixture.turnIds);
  }
  await loadAll(21);
  for (const marker of fixture.markers) assert.ok((await transcript.innerText()).includes(marker));
  const fetchedBeforeReconnect = requests.filter(r => r.method === 'fetchTurns').length;
  await context.setOffline(true); assert.ok(await page.evaluate(() => window.disconnectAhpForAcceptance()) > 0);
  await source('offline'); assert.equal((await transcript.innerText()).includes(fixture.offline), false);
  await context.setOffline(false);
  await status.getByText('Na żywo', { exact: true }).waitFor();
  await page.waitForFunction(marker => document.querySelector('#ahp-transcript')?.textContent.includes(marker), fixture.offline);
  assert.ok(requests.some(r => r.method === 'reconnect'));
  assert.equal(requests.filter(r => r.method === 'fetchTurns').length, fetchedBeforeReconnect, 'Reconnect must not auto-fetch older pages');
  assert.ok(snapshots.at(-1).cursor); assert.notEqual(snapshots.at(-1).cursor, refreshed.cursor);
  assert.ok(await transcript.locator('[data-turn-id]').count() < 22);
  assert.equal((await transcript.innerText()).includes(fixture.markers[0]), false);
  await loadAll(22); await source('verify');
  assert.ok(maxFrame <= 1024 * 1024, `Unbounded AHP frame ${maxFrame}`);
  const discovery = await page.evaluate(async () => (await fetch('/v1/web/session')).json());
  await source('revoke', { device: discovery.device_id });
  await page.waitForFunction(marker => !document.body.textContent.includes(marker), fixture.markers[0]);
  const body = await page.locator('body').innerText();
  for (const marker of [...fixture.markers, fixture.live, fixture.offline]) assert.equal(body.includes(marker), false);
  assert.equal(await page.getByTestId('ahp-history-more').count(), 0);
  assert.ok(requests.every(r => ['initialize', 'reconnect', 'listSessions', 'subscribe', 'unsubscribe', 'fetchTurns'].includes(r.method)));
  assert.deepEqual(errors, []);
  console.log('PASS Chromium AHP history: >900KiB real journal, adaptive bounded tail, keyboard explicit pages, original delayed/lost frame recovery, live interleaving, snapshot cursor renewal, duplicate-free full history and revocation.');
} finally { await browser.close(); lines.close(); process.stdin.destroy(); }
