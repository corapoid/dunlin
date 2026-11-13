// Real browser -> AHP dispatch -> server actor -> controlled local ACP process.
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
  page.setDefaultTimeout(15000);
  const errors = [];
  const methods = [];
  const dispatches = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('request', (request) => assert.equal(new URL(request.url()).origin, fixture.origin));
  page.on('websocket', (socket) => {
    if (new URL(socket.url()).pathname !== '/v1/web/ahp') return;
    socket.on('framesent', ({ payload }) => {
      const message = JSON.parse(String(payload));
      if (message.method) methods.push(message.method);
      if (message.method === 'dispatchAction') dispatches.push(message.params);
    });
  });
  await page.addInitScript(() => {
    const NativeWebSocket = window.WebSocket;
    const sockets = new Set();
    const fault = { dropped: 0, accepted: null, closedAfterSeq: null };
    window.ahpAckLossForAcceptance = fault;
    window.WebSocket = class extends NativeWebSocket {
      constructor(...args) {
        super(...args);
        if (new URL(this.url).pathname === '/v1/web/ahp') {
          sockets.add(this);
          this.addEventListener('close', () => sockets.delete(this));
          // Registered before the production onmessage handler. Discard one actual
          // accepted receipt, without replacing or modifying any server payload.
          this.addEventListener('message', (event) => {
            const message = JSON.parse(event.data);
            const action = message.method === 'action' ? message.params : null;
            if (!action) return;
            if (fault.dropped === 0 && action.origin && !action.rejectionReason) {
              fault.dropped++;
              fault.accepted = action;
              event.stopImmediatePropagation();
              return;
            }
            if (fault.accepted && fault.closedAfterSeq === null && action.serverSeq > fault.accepted.serverSeq) {
              // Let the real following update advance the production cursor first.
              // Its reconnect replay then cannot accidentally deliver the lost receipt.
              fault.closedAfterSeq = action.serverSeq;
              queueMicrotask(() => this.close(4000, 'acceptance accepted receipt loss'));
            }
          }, { capture: true });
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
    if (!response.ok) throw new Error('Session discovery failed');
    return response.json();
  });
  assert.equal(discovery.capabilities.ahp_observer, true);
  assert.equal(discovery.ahp_endpoint, '/v1/web/ahp');
  // Exercise the existing authorized bootstrap route, using this browser's cookie.
  const created = await page.evaluate(async ({ workspace, project }) => {
    const response = await fetch(`/v1/web/workspaces/${workspace}/projects/${project}/interactive-sessions`, {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ model: 'fixture-model', mode: 'plan', max_seconds: 120, max_output_bytes: 1048576 }),
    });
    return { status: response.status, body: await response.json() };
  }, fixture);
  assert.equal(created.status, 202, JSON.stringify(created.body));
  const session = created.body.session.id;
  assert.equal(typeof session, 'string');
  await page.getByRole('button', { name: 'Sesje', exact: true }).click();
  const status = page.getByTestId('ahp-status');
  const transcript = page.getByTestId('ahp-transcript');
  await status.getByText('Na żywo', { exact: true }).waitFor();
  await page.locator(`[data-testid="ahp-session"][data-session-uri="ahp-session:/${session}"]`).click();
  const prompt = page.getByTestId('ahp-prompt');
  const send = page.getByTestId('ahp-send');
  await prompt.waitFor();
  await prompt.fill(fixture.prompt);
  await send.click();
  // This text can only come from the actual ACP subprocess, not a local echo.
  await transcript.getByText(`reply: ${fixture.prompt}`, { exact: false }).waitFor();
  await page.waitForFunction(() => {
    const fault = window.ahpAckLossForAcceptance;
    return fault.dropped === 1 && fault.closedAfterSeq > fault.accepted.serverSeq
      && document.querySelector('[data-testid="ahp-status"]')?.textContent === 'Na żywo'
      && !document.querySelector('[data-testid="ahp-pending"]');
  });
  assert.ok(dispatches.length >= 2, 'A lost accepted receipt must cause a real outbox retry');
  for (const retry of dispatches.slice(1)) assert.deepEqual(retry, dispatches[0], 'Retry must preserve channel, clientSeq, message identity and payload');
  const ackLoss = await page.evaluate(() => window.ahpAckLossForAcceptance);
  assert.equal(ackLoss.dropped, 1);
  assert.equal(ackLoss.accepted.rejectionReason, undefined);
  assert.equal(ackLoss.accepted.origin.clientSeq, dispatches[0].clientSeq);
  assert.ok(methods.includes('reconnect'));
  const acknowledgedDispatchCount = dispatches.length;
  await context.setOffline(true);
  assert.ok(await page.evaluate(() => window.disconnectAhpForAcceptance()) > 0);
  await page.waitForFunction(() => document.querySelector('[data-testid="ahp-send"]')?.disabled === true);
  // Complete the process while every browser transport is offline.
  await source('release', { session });
  assert.equal((await transcript.innerText()).includes(fixture.completion), false);
  await context.setOffline(false);
  await status.getByText('Na żywo', { exact: true }).waitFor();
  await transcript.getByText(fixture.completion, { exact: false }).waitFor();
  await transcript.getByText('Agent', { exact: true }).waitFor();
  assert.ok(methods.includes('reconnect'));
  assert.equal(dispatches.length, acknowledgedDispatchCount, 'The later offline recovery must not resend an acknowledged prompt');
  for (const retry of dispatches.slice(1)) assert.deepEqual(retry, dispatches[0]);
  assert.equal(await page.evaluate(() => window.ahpAckLossForAcceptance.dropped), 1);
  await source('verify', { session });
  await source('revoke', { device: discovery.device_id });
  await page.waitForFunction((marker) => !document.body.textContent.includes(marker), fixture.prompt);
  assert.equal((await page.locator('body').innerText()).includes(fixture.completion), false);
  assert.equal(await page.evaluate(async () => (await fetch('/v1/web/session')).status), 401);
  await page.reload();
  await page.getByLabel('Jednorazowy kod parowania').waitFor();
  assert.equal(await page.getByTestId('ahp-composer').count(), 0);
  assert.deepEqual(errors, []);
  console.log('PASS Chromium AHP dispatch: browser cookie bootstrap, actual WS prompt, server-owned ACP process, dropped accepted receipt and identical outbox retry, exactly one executed prompt, stream, completion while disconnected, replay, revocation.');
} finally {
  await browser.close();
  lines.close();
  process.stdin.destroy();
}
