// Real production UI/HTTP/cookies/native AHP; stdio only changes the source journal.
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
async function source(command) {
  process.stdout.write(`${JSON.stringify({ command })}\n`);
  assert.deepEqual(await receive(), { ok: command });
}
const browser = await chromium.launch({ executablePath: process.env.DUNLIN_CHROMIUM || chromium.executablePath(), headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
try {
  const errors = [];
  const readerEvents = [];
  const writes = [];
  let readerCloses = 0;
  async function paired(code, reader = false) {
    const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: 'reduce' });
    const page = await context.newPage();
    page.setDefaultTimeout(15000);
    page.on('pageerror', (error) => errors.push(error.message));
    page.on('request', (request) => {
      assert.equal(new URL(request.url()).origin, fixture.origin);
      if (request.method() === 'PUT' && new URL(request.url()).pathname.endsWith('/sharing')) writes.push(JSON.parse(request.postData()));
    });
    page.on('websocket', (socket) => {
      assert.equal(new URL(socket.url()).host, new URL(fixture.origin).host);
      if (reader && new URL(socket.url()).pathname === '/v1/web/ahp') {
        socket.on('framereceived', ({ payload }) => readerEvents.push(JSON.parse(String(payload))));
        socket.on('close', () => readerCloses++);
      }
    });
    await page.goto(fixture.origin);
    await page.getByLabel('Jednorazowy kod parowania').fill(code);
    await page.getByRole('button', { name: 'Połącz bezpiecznie', exact: true }).click();
    await page.getByRole('button', { name: 'Sesje', exact: true }).click();
    await page.getByTestId('ahp-status').getByText('Na żywo', { exact: true }).waitFor();
    return page;
  }
  const owner = await paired(fixture.ownerCode);
  const reader = await paired(fixture.readerCode, true);
  const stranger = await paired(fixture.strangerCode);
  const sessionRow = (page) => page.locator(`[data-testid="ahp-session"][data-session-uri="${fixture.sessionUri}"]`);
  assert.equal(await sessionRow(reader).count(), 0);
  assert.equal(await sessionRow(stranger).count(), 0, 'A global admin has no implicit private-session access');
  await sessionRow(owner).click();
  await owner.getByTestId('ahp-transcript').getByText(fixture.initial, { exact: false }).waitFor();
  assert.equal((await reader.locator('body').innerText()).includes(fixture.initial), false);
  const recipient = owner.locator(`[data-testid="ahp-sharing-reader"][data-user-id="${fixture.readerId}"]`);
  let revision = 0;
  async function share(enabled) {
    await recipient.waitFor();
    await recipient.setChecked(enabled);
    const responsePromise = owner.waitForResponse((response) => response.request().method() === 'PUT' && new URL(response.url()).pathname === `/v1/web/interactive-sessions/${fixture.session}/sharing`);
    await owner.getByTestId('ahp-sharing-save').click();
    const response = await responsePromise;
    assert.equal(response.status(), 200);
    const view = await response.json();
    assert.ok(view.revision > revision);
    revision = view.revision;
    assert.deepEqual(view.readers.map((reader) => reader.user_id), enabled ? [fixture.readerId] : []);
    await owner.getByTestId('ahp-sharing-revision').getByText(`Wersja ${revision}`, { exact: true }).waitFor();
  }
  await share(true);
  await sessionRow(reader).waitFor();
  assert.ok(readerEvents.some((message) => message.method === 'root/sessionAdded' && message.params.summary?.resource === fixture.sessionUri));
  await sessionRow(reader).click();
  await reader.getByTestId('ahp-transcript').getByText(fixture.initial, { exact: false }).waitFor();
  async function assertReadOnly() {
    const panel = reader.getByTestId('ahp-observer');
    await panel.getByText('Udostępniona · Tylko odczyt', { exact: true }).first().waitFor();
    for (const selector of ['ahp-composer', 'ahp-cancel-turn', 'ahp-queue-edit', 'ahp-queue-remove', 'ahp-queue-up', 'ahp-queue-down', 'ahp-tool-approve', 'ahp-tool-deny', 'ahp-sharing']) assert.equal(await reader.getByTestId(selector).count(), 0, `${selector} must remain owner-only`);
  }
  await assertReadOnly();
  await source('append');
  for (const page of [owner, reader]) await page.getByTestId('ahp-transcript').getByText(fixture.streamed, { exact: false }).waitFor();
  assert.equal(await sessionRow(stranger).count(), 0);
  await share(false);
  await reader.waitForFunction((marker) => !document.body.textContent.includes(marker), fixture.initial);
  assert.ok(readerCloses > 0, 'Revoking a selected shared channel must close its actual native WS');
  assert.equal((await reader.locator('body').innerText()).includes(fixture.streamed), false);
  await source('unshared');
  await owner.getByTestId('ahp-transcript').getByText(fixture.unshared, { exact: false }).waitFor();
  assert.equal((await reader.locator('body').innerText()).includes(fixture.unshared), false);
  assert.equal(JSON.stringify(readerEvents).includes(fixture.unshared), false, 'Unshared output must not leak in transport frames');
  await reader.reload();
  await reader.getByRole('button', { name: 'Sesje', exact: true }).click();
  await reader.getByTestId('ahp-status').getByText('Na żywo', { exact: true }).waitFor();
  assert.equal(await sessionRow(reader).count(), 0);
  assert.equal((await reader.locator('body').innerText()).includes(fixture.unshared), false);
  await share(true);
  await sessionRow(reader).click();
  for (const marker of [fixture.initial, fixture.streamed, fixture.unshared]) await reader.getByTestId('ahp-transcript').getByText(marker, { exact: false }).waitFor();
  await assertReadOnly();
  assert.equal(await sessionRow(stranger).count(), 0);
  assert.equal(writes.length, 3, 'Grant, revoke and regrant must persist through the actual sharing API');
  assert.deepEqual(writes.map((write) => write.reader_ids), [[fixture.readerId], [], [fixture.readerId]]);
  assert.ok(writes[0].expected_revision < writes[1].expected_revision && writes[1].expected_revision < writes[2].expected_revision);
  const audit = await owner.evaluate(async (session) => {
    const response = await fetch(`/v1/web/interactive-sessions/${session}/sharing`);
    if (!response.ok) throw new Error('Owner sharing audit unavailable');
    return response.json();
  }, fixture.session);
  assert.equal(audit.revision, revision);
  assert.equal(audit.audit.length, 3);
  assert.equal(audit.audit.filter((entry) => entry.added.includes(fixture.readerId)).length, 2);
  assert.equal(audit.audit.filter((entry) => entry.removed.includes(fixture.readerId)).length, 1);
  for (const marker of [fixture.initial, fixture.streamed, fixture.unshared, 'Private owner prompt']) assert.equal(JSON.stringify(audit).includes(marker), false, 'Sharing audit must contain metadata only');
  await owner.getByTestId('ahp-sharing-audit').locator('summary').click();
  await owner.getByTestId('ahp-sharing-audit').getByText(/Dodano:/).first().waitFor();
  assert.deepEqual(errors, []);
  console.log('PASS Chromium AHP sharing: isolated cookies, private default, explicit UI grant, live read-only sharing, actual journal delta, revoke clears selected transcript, no post-revoke leak, regrant fresh history, no implicit admin access.');
} finally {
  await browser.close();
  lines.close();
  process.stdin.destroy();
}
