// Production browser -> controller -> real private TLS/WSS -> worker export.
// The controlled fixture proves metadata binding, not remote execution readiness.
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
const base = `/v1/web/workspaces/${fixture.workspace}/projects/${fixture.project}`;
function noPrivateFields(value) {
  if (!value || typeof value !== 'object') return;
  for (const [key, item] of Object.entries(value)) {
    assert.equal(
      ['root', 'path', 'git_dir', 'common_dir', 'credential', 'nonce', 'ticket', 'command'].includes(key),
      false,
      `Private field leaked: ${key}`,
    );
    noPrivateFields(item);
  }
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
  const writes = [];
  const resolves = [];
  async function pair(context, code) {
    const page = await context.newPage();
    page.setDefaultTimeout(15000);
    page.on('pageerror', (error) => errors.push(error.message));
    page.on('request', (request) => {
      const url = new URL(request.url());
      assert.equal(url.origin, fixture.origin);
      urls.push(url.href);
      if (url.pathname.startsWith(`${base}/remote-bindings`) && request.method() !== 'GET') {
        const body = JSON.parse(request.postData());
        noPrivateFields(body);
        if (url.pathname.endsWith('/remote-bindings/resolve')) resolves.push(body);
        else writes.push({ method: request.method(), path: url.pathname, body });
      }
    });
    await page.goto(fixture.origin);
    await page.getByLabel('Jednorazowy kod parowania').fill(code);
    await page.getByRole('button', { name: 'Połącz bezpiecznie', exact: true }).click();
    await page.getByRole('button', { name: 'Zespół', exact: true }).click();
    await page.locator(`[data-testid="team-workspace"][data-workspace-id="${fixture.workspace}"]`).click();
    await page
      .locator(`[data-testid="project-remote-bindings"][data-project-id="${fixture.project}"]`)
      .click();
    return page;
  }
  async function read(page, path) {
    return page.evaluate(async (path) => {
      const response = await fetch(path, { credentials: 'same-origin', cache: 'no-store' });
      return { status: response.status, value: await response.json() };
    }, path);
  }
  const page = await pair(owner, fixture.ownerCode);
  const panel = page.getByTestId('remote-bindings');
  await panel.getByText('Brak zapisanych zdalnych bindingów.', { exact: true }).waitFor();
  const localBefore = await read(page, `${base}/binding`);
  assert.equal(localBefore.status, 200);
  noPrivateFields(localBefore.value);
  await panel.getByTestId('remote-binding-host').selectOption(fixture.host);
  await panel.getByTestId('remote-binding-export').fill(fixture.exportId);
  await panel.getByTestId('remote-binding-export-revision').fill(String(fixture.exportRevision));
  const acceptedResponse = page.waitForResponse(
    (response) =>
      new URL(response.url()).pathname === `${base}/remote-bindings` &&
      response.request().method() === 'POST',
  );
  await panel.getByTestId('remote-binding-save').click();
  const accepted = await acceptedResponse;
  assert.equal(accepted.ok(), true, `Actual export binding HTTP status ${accepted.status()}`);
  const binding = await accepted.json();
  noPrivateFields(binding);
  assert.equal(binding.host_id, fixture.host);
  assert.equal(binding.export_id, fixture.exportId);
  assert.equal(binding.export_revision, fixture.exportRevision);
  assert.equal(binding.active, true);
  await panel.getByTestId('remote-binding-select').selectOption(binding.id);
  await panel.getByTestId('remote-binding-observed').getByText(fixture.exportId, { exact: false }).waitFor();
  assert.ok((await panel.innerText()).includes('Zdalne uruchamianie pozostaje niedostępne'));
  assert.deepEqual(writes, [
    {
      method: 'POST',
      path: `${base}/remote-bindings`,
      body: {
        host_id: fixture.host,
        export_id: fixture.exportId,
        expected_export_revision: fixture.exportRevision,
      },
    },
  ]);
  const observed = await read(page, `${base}/remote-bindings`);
  assert.equal(observed.status, 200);
  assert.equal(observed.value.observed_state_only, true);
  assert.equal(observed.value.execution_ready, false);
  noPrivateFields(observed.value);
  assert.deepEqual(observed.value.bindings, [binding]);
  assert.deepEqual(
    await read(page, `${base}/binding`),
    localBefore,
    'Remote binding cannot reinterpret the local target',
  );
  const publicText = await panel.innerText();
  for (const path of ['/scratch/', '/tmp/', '/Users/', 'exports.sqlite3', 'dhe_', 'dhw_'])
    assert.equal(publicText.includes(path), false);
  assert.equal(resolves.length, 0, 'Access resolution is never automatic');
  const identity = await read(page, '/v1/web/session');
  const checkedResponse = page.waitForResponse(response => new URL(response.url()).pathname === `${base}/remote-bindings/resolve` && response.request().method() === 'POST');
  await panel.getByTestId('remote-binding-resolve').click();
  const checkedHttp = await checkedResponse;
  assert.equal(checkedHttp.status(), 200);
  const checked = await checkedHttp.json(); noPrivateFields(checked);
  assert.deepEqual(checked.binding, { id: binding.id, revision: binding.revision });
  assert.deepEqual(checked.principal, { kind: 'human', id: identity.value.user.id });
  assert.equal(checked.required_worker_permission, 'read');
  assert.equal(checked.observed_state_only, true); assert.equal(checked.execution_ready, false);
  await panel.getByTestId('remote-binding-access-observed').waitFor();
  assert.deepEqual(await read(page, `${base}/remote-bindings`), observed, 'Resolve cannot rewrite saved administrator metadata');
  await source('revoke-worker-grant');
  const deniedCheckResponse = page.waitForResponse(response => new URL(response.url()).pathname === `${base}/remote-bindings/resolve` && response.request().method() === 'POST');
  await panel.getByTestId('remote-binding-resolve').click();
  assert.equal((await deniedCheckResponse).status(), 403, 'Current caller Read must be denied while the worker channel is still connected');
  await panel.getByTestId('remote-binding-access-error').waitFor();
  assert.equal(await panel.getByTestId('remote-binding-access-observed').count(), 0);
  assert.equal(await panel.getByTestId('remote-bindings-forbidden').count(), 0, 'Own worker Read denial does not revoke independent configuration authority');
  assert.deepEqual(resolves, [{ binding: { id: binding.id, revision: binding.revision } }, { binding: { id: binding.id, revision: binding.revision } }]);
  assert.deepEqual(await read(page, `${base}/remote-bindings`), observed, 'Own Read denial does not convert saved metadata into current authority');
  await source('disconnect-worker');
  // Worker authority changed without a controller-catalog event. The descriptor
  // is still an observation; a read never upgrades it to a current grant.
  await panel.getByTestId('remote-bindings-refresh').click();
  await panel.getByRole('button', { name: 'Wczytaj wybrany binding do formularza', exact: true }).click();
  await panel.getByTestId('remote-binding-observed').getByText('Zapis active', { exact: false }).waitFor();
  const afterDisconnect = await read(page, `${base}/remote-bindings`);
  assert.deepEqual(
    afterDisconnect,
    observed,
    'Disconnected worker does not silently retarget stored metadata',
  );
  assert.ok((await panel.innerText()).includes('nie potwierdza aktualnego grantu'));
  const revokedResponse = page.waitForResponse(
    (response) => new URL(response.url()).pathname === `${base}/remote-bindings/${binding.id}/revoke`,
  );
  await panel.getByTestId('remote-binding-revoke').click();
  const revoked = await (await revokedResponse).json();
  assert.equal(revoked.id, binding.id);
  assert.equal(revoked.active, false);
  assert.equal(revoked.revision, binding.revision + 1);
  noPrivateFields(revoked);
  await panel.getByTestId('remote-binding-observed').getByText('Zapis inactive', { exact: false }).waitFor();
  assert.deepEqual(writes[1], {
    method: 'POST',
    path: `${base}/remote-bindings/${binding.id}/revoke`,
    body: { expected_revision: binding.revision },
  });
  assert.deepEqual(
    await read(page, `${base}/binding`),
    localBefore,
    'Offline revoke cannot switch local execution targets',
  );
  await source('revoke-manager');
  await page.getByTestId('remote-bindings-forbidden').waitFor();
  assert.equal(await page.getByTestId('remote-binding-observed').count(), 0);
  assert.equal((await page.locator('body').innerText()).includes(binding.checkout_identity_digest), false);
  const other = await pair(viewer, fixture.viewerCode);
  await other.getByTestId('remote-bindings-forbidden').waitFor();
  assert.equal(await other.getByTestId('remote-binding-observed').count(), 0);
  const denied = await read(other, `${base}/remote-bindings`);
  assert.equal(denied.status, 403);
  assert.equal(JSON.stringify(denied.value).includes(binding.checkout_identity_digest), false);
  for (const current of [page, other]) {
    const stored = await current.evaluate(() =>
      JSON.stringify([Object.entries(localStorage), Object.entries(sessionStorage)]),
    );
    assert.equal(stored.includes(binding.checkout_identity_digest), false);
    assert.equal(stored.includes(binding.approved_settings_digest), false);
  }
  assert.equal(
    urls.some((url) => url.includes(binding.checkout_identity_digest)),
    false,
  );
  assert.equal(writes.length, 2, 'No automatic mutation retry, local fallback or execution dispatch');
  assert.equal(resolves.length, 2, 'No background resolver RPC');
  assert.equal(urls.some(url => new URL(url).pathname.includes('/interactive-sessions')), false, 'Read resolution cannot start a session');
  assert.deepEqual(errors, []);
  console.log(
    'PASS Chromium remote bindings: production project panel binds actual worker export over private TLS/WSS, observed metadata only, explicit current-caller Read resolution and connected grant revocation denial, no paths or secret persistence, worker grant revoke/disconnect leaves historical observation, offline CAS revoke succeeds, local binding unchanged, management revocation and viewer denial clear metadata. No remote execution claimed.',
  );
} finally {
  await browser.close();
  lines.close();
  process.stdin.destroy();
}
