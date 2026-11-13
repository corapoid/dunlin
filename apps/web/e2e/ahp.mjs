// Production React + real Axum/AHP WebSocket. Stdio controls only the source journal.
import assert from 'node:assert/strict';
import { createInterface } from 'node:readline';
import { chromium } from 'playwright-core';

const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
const input = lines[Symbol.asyncIterator]();
async function receive() {
  const line = await input.next();
  assert.equal(line.done, false, 'Rust fixture closed its control pipe');
  return JSON.parse(line.value);
}
const fixture = await receive();
async function source(command, extra = {}) {
  process.stdout.write(`${JSON.stringify({ command, ...extra })}\n`);
  assert.deepEqual(await receive(), { ok: command });
}
const browser = await chromium.launch({ executablePath: process.env.DUNLIN_CHROMIUM || chromium.executablePath(), headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
try {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: 'reduce' });
  const page = await context.newPage();
  page.setDefaultTimeout(10000);
  const errors = [];
  const methods = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('request', (request) => assert.equal(new URL(request.url()).origin, fixture.origin));
  page.on('websocket', (socket) => {
    if (new URL(socket.url()).pathname !== '/v1/web/ahp') return;
    socket.on('framesent', ({ payload }) => {
      const message = JSON.parse(String(payload));
      if (message.method) methods.push(message.method);
    });
  });
  // Keep native browser sockets and real traffic; expose only a deterministic disconnect.
  await page.addInitScript(() => {
    const NativeWebSocket = window.WebSocket;
    const sockets = new Set();
    window.WebSocket = class extends NativeWebSocket {
      constructor(...args) {
        super(...args);
        if (new URL(this.url).pathname === '/v1/web/ahp') {
          sockets.add(this);
          this.addEventListener('close', () => sockets.delete(this));
        }
      }
    };
    window.disconnectAhpForAcceptance = () => {
      const count = sockets.size;
      for (const socket of sockets) socket.close(4000, 'acceptance reconnect');
      return count;
    };
  });
  await page.goto(fixture.origin);
  await page.getByLabel('Jednorazowy kod parowania').fill(fixture.ownerCode);
  await page.getByRole('button', { name: 'Połącz bezpiecznie', exact: true }).click();
  await page.getByRole('button', { name: 'Sesje', exact: true }).waitFor();
  const discovery = await page.evaluate(async () => {
    const response = await fetch('/v1/web/session');
    if (!response.ok) throw new Error('Authenticated session discovery failed');
    return response.json();
  });
  assert.equal(discovery.capabilities.ahp_observer, true);
  assert.equal(discovery.ahp_endpoint, '/v1/web/ahp');
  assert.equal(typeof discovery.device_id, 'string');
  const cookie = (await context.cookies()).find((c) => c.name === 'dunlin_web_session');
  assert.equal(cookie?.httpOnly, true);
  assert.equal(cookie?.sameSite, 'Strict');
  await page.getByRole('button', { name: 'Sesje', exact: true }).click();
  const panel = page.getByTestId('ahp-observer');
  const status = page.getByTestId('ahp-status');
  const transcript = page.getByTestId('ahp-transcript');
  await status.getByText('Na żywo', { exact: true }).waitFor();
  await panel.getByText('Tylko odczyt', { exact: true }).waitFor();
  await page.locator(`[data-testid="ahp-session"][data-session-uri="${fixture.sessionUri}"]`).click();
  await transcript.getByText(fixture.initial, { exact: false }).waitFor();
  assert.equal(await page.getByTestId('ahp-session').count(), 1, 'Other users’ private sessions must stay hidden');
  assert.equal((await panel.innerText()).includes(fixture.hidden), false);
  assert.equal(await panel.locator('textarea,input,[contenteditable="true"]').count(), 0);
  assert.equal(await panel.getByRole('button', { name: /wyślij|send|prompt/i }).count(), 0);
  await source('append');
  await transcript.getByText(fixture.streamed, { exact: false }).waitFor();
  assert.ok(methods.includes('initialize') && methods.includes('listSessions') && methods.includes('subscribe'));

  await context.setOffline(true);
  assert.ok(await page.evaluate(() => window.disconnectAhpForAcceptance()) > 0);
  await source('offline');
  assert.equal((await transcript.innerText()).includes(fixture.offline), false);
  await context.setOffline(false);
  await status.getByText('Na żywo', { exact: true }).waitFor();
  await transcript.getByText(fixture.offline, { exact: false }).waitFor();
  assert.ok(methods.includes('reconnect'), 'Recovery must use the actual AHP reconnect command');
  await source('complete');
  await transcript.getByText('Agent', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
  await source('revoke', { device: discovery.device_id });
  await page.waitForFunction((marker) => !document.body.textContent.includes(marker), fixture.initial);
  assert.equal((await page.locator('body').innerText()).includes(fixture.streamed), false);
  const denied = await page.evaluate(async () => (await fetch('/v1/web/session')).status);
  assert.equal(denied, 401);
  await page.reload();
  await page.getByLabel('Jednorazowy kod parowania').waitFor();
  assert.equal((await page.locator('body').innerText()).includes(fixture.offline), false);
  assert.ok(methods.every((method) => ['initialize', 'reconnect', 'listSessions', 'subscribe', 'unsubscribe'].includes(method)), 'Observer must only send read methods');
  assert.deepEqual(errors, []);
  console.log('PASS Chromium AHP observer: production app, real journal snapshot/stream/reconnect/completion, user isolation, read-only controls, mobile layout, device revocation.');
} finally {
  await browser.close();
  lines.close();
  process.stdin.destroy();
}
