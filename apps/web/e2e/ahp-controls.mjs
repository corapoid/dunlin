// Production UI and native AHP WS against a real server-owned, cancellable ACP peer.
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
  const actions = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('request', (request) => assert.equal(new URL(request.url()).origin, fixture.origin));
  page.on('websocket', (socket) => {
    assert.equal(new URL(socket.url()).host, new URL(fixture.origin).host);
    if (new URL(socket.url()).pathname === '/v1/web/ahp') socket.on('framesent', ({ payload }) => {
      const message = JSON.parse(String(payload));
      if (message.method === 'dispatchAction') actions.push(message.params.action);
    });
  });
  await page.goto(fixture.origin);
  await page.getByLabel('Jednorazowy kod parowania').fill(fixture.ownerCode);
  await page.getByRole('button', { name: 'Połącz bezpiecznie', exact: true }).click();
  await page.getByRole('button', { name: 'Sesje', exact: true }).waitFor();
  const created = await page.evaluate(async ({ workspace, project }) => {
    const response = await fetch(`/v1/web/workspaces/${workspace}/projects/${project}/interactive-sessions`, {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ model: 'fixture-model', mode: 'plan', max_seconds: 120, max_output_bytes: 1048576 }),
    });
    return { status: response.status, body: await response.json() };
  }, fixture);
  assert.equal(created.status, 202, JSON.stringify(created.body));
  const session = created.body.session.id;
  await page.getByRole('button', { name: 'Sesje', exact: true }).click();
  await page.getByTestId('ahp-status').getByText('Na żywo', { exact: true }).waitFor();
  await page.locator(`[data-testid="ahp-session"][data-session-uri="ahp-session:/${session}"]`).click();
  const transcript = page.getByTestId('ahp-transcript');
  const queued = page.getByTestId('ahp-queued');
  const idleOutbox = () => page.getByTestId('ahp-pending').waitFor({ state: 'detached' });
  async function send(text) {
    await page.getByTestId('ahp-prompt').fill(text);
    await page.getByTestId('ahp-send').click();
  }
  await send(fixture.active);
  await transcript.getByText(`started: ${fixture.active}`, { exact: false }).waitFor();
  await idleOutbox();
  await send(fixture.removed);
  await queued.filter({ hasText: fixture.removed }).waitFor();
  await idleOutbox();
  await send(fixture.original);
  const editable = queued.filter({ hasText: fixture.original });
  await editable.waitFor();
  await idleOutbox();
  assert.equal(await queued.count(), 2);
  await editable.getByTestId('ahp-queue-edit').click();
  await editable.getByTestId('ahp-queue-editor').fill(fixture.edited);
  // The row's original text may disappear once its controlled editor changes.
  await page.getByTestId('ahp-queue-save').click();
  const remaining = queued.filter({ hasText: fixture.edited });
  await remaining.waitFor();
  await idleOutbox();
  await remaining.getByTestId('ahp-queue-up').click();
  await idleOutbox();
  await page.waitForFunction((marker) => document.querySelector('[data-testid="ahp-queued"]')?.textContent.includes(marker), fixture.edited);
  const order = await queued.allTextContents();
  assert.ok(order[0].includes(fixture.edited) && order[1].includes(fixture.removed));
  await queued.filter({ hasText: fixture.removed }).getByTestId('ahp-queue-remove').click();
  await queued.filter({ hasText: fixture.removed }).waitFor({ state: 'detached' });
  await idleOutbox();
  assert.equal(await queued.count(), 1);
  await page.getByTestId('ahp-cancel-turn').click();
  await page.getByTestId('ahp-cancel-pending').waitFor();
  await transcript.getByText('Agent · anulowano', { exact: true }).waitFor();
  assert.equal((await transcript.innerText()).includes(`reply: ${fixture.edited}`), false);
  // Rust checks a real session/cancel was received and the next ACP request is
  // still blocked, then releases the peer's actual cancelled prompt result.
  await source('cancel-barrier', { session });
  await transcript.getByText(`reply: ${fixture.edited}`, { exact: false }).waitFor();
  await page.getByTestId('ahp-cancel-pending').waitFor({ state: 'detached' });
  await transcript.getByText('Agent', { exact: true }).waitFor();
  assert.equal(await queued.count(), 0);
  assert.equal((await transcript.innerText()).includes(fixture.removed), false);
  assert.equal((await transcript.innerText()).includes(fixture.original), false);
  await source('verify', { session });
  for (const type of ['chat/pendingMessageSet', 'chat/queuedMessagesReordered', 'chat/pendingMessageRemoved', 'chat/turnCancelled']) assert.ok(actions.some((action) => action.type === type), `Missing actual native WS action ${type}`);
  assert.deepEqual(errors, []);
  console.log('PASS Chromium AHP controls: real ACP process, queued edit/reorder/remove, native WS cancel, cancellation acknowledgement barrier, only edited remaining prompt executes after cancelled result.');
} finally {
  await browser.close();
  lines.close();
  process.stdin.destroy();
}
