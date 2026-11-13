// Real production registry UI and actual HTTP worker enrollment/report via Rust.
// No daemon, tailnet, Serve verification or execution readiness is simulated.
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
const browser = await chromium.launch({
  executablePath: process.env.DUNLIN_CHROMIUM || chromium.executablePath(),
  headless: true,
  args: ['--no-sandbox', '--disable-dev-shm-usage'],
});
try {
  const owner = await browser.newContext();
  const viewer = await browser.newContext();
  const errors = [];
  const urls = [];
  async function pair(context, code) {
    const page = await context.newPage();
    page.setDefaultTimeout(15000);
    page.on('pageerror', (error) => errors.push(error.message));
    page.on('request', (request) => {
      const url = new URL(request.url());
      assert.equal(url.origin, fixture.origin);
      urls.push(url.href);
    });
    await page.goto(fixture.origin);
    await page.getByLabel('Jednorazowy kod parowania').fill(code);
    await page.getByRole('button', { name: 'Połącz bezpiecznie', exact: true }).click();
    await page.getByRole('button', { name: 'Hosty', exact: true }).click();
    await page.getByTestId('host-workspace').selectOption(fixture.workspace);
    return page;
  }
  const page = await pair(owner, fixture.ownerCode);
  await page.getByTestId('hosts-empty').waitFor();
  await page.getByTestId('host-name').fill('HOST_PRIVATE_MARKER');
  await page.getByTestId('host-node').fill('n-fixture-host');
  await page.getByTestId('host-origin').fill('https://worker.tail123.ts.net');
  await page.getByRole('checkbox', { name: 'opencode', exact: true }).check();
  await page.getByTestId('host-create-submit').click();
  const row = page.getByTestId('host-row');
  await row.getByTestId('host-status').getByText('Oczekuje na zgłoszenie', { exact: true }).waitFor();
  assert.equal(await row.count(), 1);
  const hostId = await row.getAttribute('data-host-id');
  assert.ok(hostId);
  const ticket = await page.getByTestId('host-ticket').innerText();
  assert.match(ticket, /^dhe_[A-Za-z0-9_-]+$/);
  async function noPersistence(page) {
    const stored = await page.evaluate(() =>
      JSON.stringify({
        local: Object.entries(localStorage),
        session: Object.entries(sessionStorage),
      }),
    );
    assert.equal(stored.includes(ticket), false, 'Enrollment ticket must remain memory-only');
    assert.equal(
      urls.some((url) => url.includes(ticket)),
      false,
      'Ticket must never occur in a URL',
    );
  }
  async function unverified() {
    const text = await row.innerText();
    assert.ok(text.includes('Sieć niezweryfikowana'));
    assert.ok(text.includes('Powiązanie projektu i profil sesji są weryfikowane przy uruchomieniu.'));
    assert.equal(text.includes('Gotowy do wykonania'), false);
  }
  await unverified();
  await noPersistence(page);
  assert.equal(await row.getByTestId('host-approve').isDisabled(), true);
  await source('claim', { ticket, host_id: hostId });
  await page.getByTestId('hosts-refresh').click();
  await row.getByTestId('host-status').getByText('Zgłoszony przez worker', { exact: true }).waitFor();
  assert.equal(
    await page.getByTestId('host-ticket').count(),
    0,
    'Refresh cannot recover the one-time ticket',
  );
  await row.getByTestId('host-approve').click();
  await row.getByTestId('host-status').getByText('Zatwierdzony', { exact: true }).waitFor();
  await unverified();
  const revision = (await row.innerText()).match(/rewizja (\d+)/)?.[1];
  assert.ok(revision);
  // The dedicated worker credential stays in Rust. Reporting must not change
  // the administrative CAS revision used by the following production UI action.
  await source('report', { host_id: hostId });
  await row.getByTestId('host-drain').click();
  await row.getByText('Drenaż włączony', { exact: false }).waitFor();
  const drainedRevision = Number((await row.innerText()).match(/rewizja (\d+)/)?.[1]);
  assert.equal(drainedRevision, Number(revision) + 1);
  await unverified();
  await row.getByTestId('host-revoke').click();
  await row.getByTestId('host-status').getByText('Odwołany', { exact: true }).waitFor();
  await unverified();
  await source('verify-revoked', { host_id: hostId, ticket });
  await noPersistence(page);
  const diagnosticReads = () =>
    urls.filter((url) => new URL(url).pathname.endsWith(`/hosts/${hostId}/channel`)).length;
  assert.equal(diagnosticReads(), 0, 'Channel metadata is never fetched implicitly');
  await row.getByTestId('host-channel-read').click();
  await row
    .getByTestId('host-channel-record')
    .getByText('Brak zapisanego kanału.', { exact: false })
    .waitFor();
  assert.equal(diagnosticReads(), 1);
  assert.ok(
    (await row.getByTestId('host-channel-record').innerText()).includes(
      'Zapis diagnostyczny kanału. Bieżącą obserwację pokazuje podgląd na żywo.',
    ),
  );

  // A fresh enrollment is intentionally still visible when management is
  // revoked. Catalog invalidation must remove protected DOM without a refresh.
  await row.getByTestId('host-enroll').click();
  await page.getByTestId('host-ticket').waitFor();
  await row.getByTestId('host-status').getByText('Oczekuje na zgłoszenie', { exact: true }).waitFor();
  const pendingTicket = await page.getByTestId('host-ticket').innerText();
  assert.notEqual(pendingTicket, ticket);
  assert.equal(
    await row.getByTestId('host-channel-record').count(),
    0,
    'Registry mutation invalidates the previous diagnostic read',
  );
  assert.equal(diagnosticReads(), 1, 'Invalidation does not poll or reload channel metadata');
  await row.getByTestId('host-channel-read').click();
  await row
    .getByTestId('host-channel-record')
    .getByText('Brak zapisanego kanału.', { exact: false })
    .waitFor();
  assert.equal(diagnosticReads(), 2);
  await source('revoke-manager');
  await page.getByTestId('hosts-forbidden').waitFor();
  assert.equal(await page.getByTestId('host-row').count(), 0);
  assert.equal(await page.getByTestId('host-channel-record').count(), 0);
  assert.equal(diagnosticReads(), 2, 'Revocation clears diagnostics without channel polling');
  assert.equal(await page.getByTestId('host-ticket').count(), 0);
  assert.equal((await page.locator('body').innerText()).includes('HOST_PRIVATE_MARKER'), false);
  assert.equal((await page.locator('body').innerText()).includes(pendingTicket), false);
  const ownerStorage = await page.evaluate(() =>
    JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]),
  );
  assert.equal(ownerStorage.includes(pendingTicket), false);
  assert.equal(
    urls.some((url) => url.includes(pendingTicket)),
    false,
  );
  const other = await pair(viewer, fixture.viewerCode);
  const forbidden = other.getByTestId('hosts-forbidden');
  await forbidden.waitFor();
  assert.equal(await other.getByTestId('host-row').count(), 0);
  assert.equal(await other.getByTestId('host-create').count(), 0);
  assert.equal(await other.getByTestId('host-ticket').count(), 0);
  const viewerBody = await other.locator('body').innerText();
  assert.equal(viewerBody.includes('HOST_PRIVATE_MARKER'), false);
  assert.equal(viewerBody.includes(ticket), false);
  const protectedResponse = await other.evaluate(async (workspace) => {
    const response = await fetch(`/v1/web/workspaces/${workspace}/hosts`);
    return { status: response.status, body: await response.text() };
  }, fixture.workspace);
  assert.equal(protectedResponse.status, 403);
  assert.equal(protectedResponse.body.includes('HOST_PRIVATE_MARKER'), false);
  assert.equal(protectedResponse.body.includes(ticket), false);
  await noPersistence(other);
  assert.deepEqual(errors, []);
  console.log(
    'PASS Chromium host registry: production create/one-time enrollment, actual dedicated worker HTTP claim/report, administrative CAS approval/drain/revocation, registry remains unverified without a global readiness claim, old worker credential/ticket rejected, explicit raw channel diagnostic reads and permission invalidation, workspace viewer forbidden and no secret persistence. No Serve verification or daemon execution claimed.',
  );
} finally {
  await browser.close();
  lines.close();
  process.stdin.destroy();
}
