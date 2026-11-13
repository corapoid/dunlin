// Real two-tab UI -> native AHP -> local ACP permission request and safe marker write.
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
  const errors = [];
  const dispatches = [];
  let currentTurn;
  function observe(page, capture = false) {
    page.setDefaultTimeout(15000);
    page.on('pageerror', (error) => errors.push(error.message));
    page.on('request', (request) => assert.equal(new URL(request.url()).origin, fixture.origin));
    page.on('websocket', (socket) => {
      assert.equal(new URL(socket.url()).host, new URL(fixture.origin).host);
      if (!capture || new URL(socket.url()).pathname !== '/v1/web/ahp') return;
      socket.on('framesent', ({ payload }) => {
        const message = JSON.parse(String(payload));
        if (message.method === 'dispatchAction') dispatches.push(message.params);
      });
      socket.on('framereceived', ({ payload }) => {
        const message = JSON.parse(String(payload));
        if (message.method === 'action' && message.params.action.type === 'chat/turnStarted') currentTurn = message.params.action.turnId;
      });
    });
  }
  const page = await context.newPage();
  observe(page, true);
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
  async function openSession(target) {
    await target.getByRole('button', { name: 'Sesje', exact: true }).click();
    await target.getByTestId('ahp-status').getByText('Na żywo', { exact: true }).waitFor();
    await target.locator(`[data-testid="ahp-session"][data-session-uri="ahp-session:/${session}"]`).click();
    await target.getByTestId('ahp-prompt').waitFor();
  }
  await openSession(page);
  const other = await context.newPage();
  observe(other);
  await other.goto(fixture.origin);
  await openSession(other);
  async function send(text) {
    await page.getByTestId('ahp-prompt').fill(text);
    await page.getByTestId('ahp-send').click();
  }
  await send(fixture.approve);
  for (const target of [page, other]) {
    await target.getByTestId('ahp-permission').waitFor();
    assert.equal(await target.getByTestId('ahp-tool-approve').count(), 1);
    assert.equal(await target.getByTestId('ahp-tool-deny').count(), 1);
    assert.equal(await target.getByRole('button', { name: /always|zawsze/i }).count(), 0);
  }
  await source('before-approval');
  const firstOpaque = await page.getByTestId('ahp-tool-approve').getAttribute('data-option-id');
  assert.ok(firstOpaque && firstOpaque !== 'once' && firstOpaque !== 'always');
  await page.getByTestId('ahp-tool-approve').click();
  for (const target of [page, other]) {
    await target.getByTestId('ahp-transcript').getByText(`approved: ${fixture.approve}`, { exact: false }).waitFor();
    await target.getByTestId('ahp-permission').waitFor({ state: 'detached' });
    await target.getByTestId('ahp-transcript').getByText('Agent', { exact: true }).waitFor();
  }
  await source('after-approval');
  const firstVote = dispatches.find((item) => item.action.type === 'chat/toolCallConfirmed');
  assert.ok(firstVote);
  assert.equal(firstVote.action.selectedOptionId, firstOpaque);
  const firstTurn = firstVote.action.turnId;
  await send(fixture.deny);
  await page.getByTestId('ahp-permission').waitFor();
  await other.getByTestId('ahp-permission').waitFor();
  const secondOpaque = await page.getByTestId('ahp-tool-approve').getAttribute('data-option-id');
  assert.notEqual(secondOpaque, firstOpaque, 'Same remote tool/request IDs need a fresh host approval nonce');
  assert.ok(currentTurn && currentTurn !== firstTurn);
  // A separate authenticated native connection submits an old opaque option in
  // the CURRENT turn. This tests server nonce fencing rather than UI validation.
  const stale = await page.evaluate(({ origin, channel, action, turnId }) => new Promise((resolve, reject) => {
    const url = new URL('/v1/web/ahp', origin); url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
    const socket = new WebSocket(url);
    const clientId = `stale-approval-${crypto.randomUUID()}`;
    const timer = setTimeout(() => { socket.close(); reject(new Error('Stale approval response deadline')); }, 10000);
    const done = (result) => { clearTimeout(timer); socket.close(); resolve(result); };
    socket.onopen = () => socket.send(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { channel: 'ahp-root://', clientId, protocolVersions: ['1.0.0'], initialSubscriptions: ['ahp-root://', channel] } }));
    socket.onmessage = ({ data }) => {
      const message = JSON.parse(data);
      if (message.id === 1) {
        if (message.error) { done({ initializationError: message.error }); return; }
        socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'dispatchAction', params: { channel, clientSeq: 1, action: { ...action, turnId } } }));
      } else if (message.method === 'action' && message.params.origin?.clientId === clientId) done({ rejection: message.params.rejectionReason ?? null });
      else if (message.error) done({ error: message.error });
    };
  }), { origin: fixture.origin, channel: firstVote.channel, action: firstVote.action, turnId: currentTurn });
  assert.ok(stale.rejection || stale.error, JSON.stringify(stale));
  await source('after-stale');
  // The second observer can resolve the new request independently of the first.
  await other.getByTestId('ahp-tool-deny').click();
  for (const target of [page, other]) {
    await target.getByTestId('ahp-transcript').getByText(`denied: ${fixture.deny}`, { exact: false }).waitFor();
    await target.getByTestId('ahp-permission').waitFor({ state: 'detached' });
    await target.getByTestId('ahp-transcript').getByText('Agent', { exact: true }).last().waitFor();
  }
  await source('verify', { session });
  assert.deepEqual(errors, []);
  console.log('PASS Chromium AHP permissions: real ACP request, two observers, no persistent allow, single safe write after once, reused peer IDs get fresh nonce, stale nonce rejected, second observer denies without side effect.');
} finally {
  await browser.close();
  lines.close();
  process.stdin.destroy();
}
