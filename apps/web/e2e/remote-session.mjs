// Production UI -> REST admission -> AHP -> real worker WSS/ACP sandbox.
// Only the local ACP peer and private network daemons are controlled fixtures.
import assert from 'node:assert/strict';
import { createInterface } from 'node:readline';
import { chromium } from 'playwright-core';
const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
const first = await lines[Symbol.asyncIterator]().next();
assert.equal(first.done, false);
const fixture = JSON.parse(first.value);
lines.close();
const browser = await chromium.launch({
  executablePath: process.env.DUNLIN_CHROMIUM,
  headless: true,
  args: ['--no-sandbox', '--disable-dev-shm-usage'],
});
try {
  const context = await browser.newContext();
  const page = await context.newPage();
  page.setDefaultTimeout(15000);
  const errors = [], attempts = [], healthReads = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => {
    const url = new URL(request.url());
    assert.equal(url.origin, fixture.origin);
    if (url.pathname.endsWith('/health')) healthReads.push(url.href);
  });
  await page.goto(fixture.origin);
  await page.getByLabel('Jednorazowy kod parowania').fill(fixture.ownerCode);
  await page.getByRole('button', { name: 'Połącz bezpiecznie', exact: true }).click();
  await page.getByRole('button', { name: 'Sesje', exact: true }).click();
  let acceptance;
  await page.route('**/interactive-sessions', async route => {
    const request = route.request();
    if (request.method() !== 'POST') return route.continue();
    attempts.push({ body: request.postData(), key: request.headers()['idempotency-key'] });
    if (attempts.length === 1) {
      const response = await route.fetch();
      assert.equal(response.status(), 202);
      assert.equal(response.headers()['idempotency-replayed'], 'false');
      acceptance = await response.json();
      await route.abort('failed'); // Lose only the genuine server response.
    } else {
      const response = await route.fetch();
      assert.equal(response.status(), 202);
      assert.equal(response.headers()['idempotency-replayed'], 'true');
      await route.fulfill({ response });
    }
  });
  await page.getByTestId('session-create').locator('summary').click();
  await page.getByTestId('agent-workspace').selectOption(fixture.workspace);
  await page.getByTestId('agent-project').selectOption(fixture.project);
  await page.getByTestId('session-create-target').selectOption(fixture.binding.id);
  await page.getByTestId('session-create-profile').selectOption(fixture.profile.agent_id);
  await page.getByTestId('session-create-submit').click();
  await page.getByTestId('session-create-retry').waitFor();
  assert.equal(attempts.length, 1);
  assert.equal(await page.getByTestId('session-create-submit').isDisabled(), true);
  assert.deepEqual(JSON.parse(attempts[0].body), {
    profile: fixture.profile,
    target: { kind: 'remote', binding: fixture.binding },
  });
  await page.getByTestId('session-create-retry').click();
  const id = acceptance.session.id;
  await page.getByTestId('session-created-id').filter({ hasText: id }).waitFor();
  assert.equal(attempts.length, 2);
  assert.deepEqual(attempts[0], attempts[1]);
  const select = async () => {
    await page.locator(`[data-testid="ahp-session"][data-session-uri="ahp-session:/${id}"]`).click();
    await page.getByTestId('ahp-composer').waitFor();
  };
  await select();
  await page.getByTestId('ahp-prompt').fill('Remember nonce:browser-context');
  await page.getByTestId('ahp-send').click();
  const replies = () => page.getByTestId('ahp-transcript').getByText('remembered:browser-context', { exact: true });
  await replies().first().waitFor();
  assert.equal(await replies().count(), 1);
  // A new browser/SDK connection observes the same server-owned context.
  await page.reload();
  await page.getByRole('button', { name: 'Sesje', exact: true }).click();
  await select();
  await replies().first().waitFor();
  await page.getByTestId('ahp-prompt').fill('Recall prior nonce');
  await page.getByTestId('ahp-send').click();
  await replies().nth(1).waitFor();
  assert.equal(await replies().count(), 2);
  assert.equal(await page.getByTestId('ahp-tool-approve').count(), 0);
  await page.getByTestId('ahp-prompt').fill('Start cancellation fixture');
  await page.getByTestId('ahp-send').click();
  await page.getByTestId('ahp-transcript').getByText('active-cancel', { exact: true }).waitFor();
  for (const text of ['queued-first','queued-second']) {
    await page.getByTestId('ahp-prompt').fill(text);
    await page.getByTestId('ahp-send').click();
    await page.getByTestId('ahp-queued').getByText(`W kolejce: ${text}`,{exact:true}).waitFor();
  }
  const firstQueued=page.getByTestId('ahp-queued').filter({hasText:'queued-first'});
  await firstQueued.getByTestId('ahp-queue-edit').click();
  await firstQueued.getByTestId('ahp-queue-editor').fill('QUEUED_AFTER_CANCEL_EDITED');
  await firstQueued.getByTestId('ahp-queue-save').click();
  await page.getByTestId('ahp-queued').getByText('W kolejce: QUEUED_AFTER_CANCEL_EDITED',{exact:true}).waitFor();
  await page.getByTestId('ahp-queued').filter({hasText:'queued-second'}).getByTestId('ahp-queue-up').click();
  await page.getByTestId('ahp-queued').first().getByText('W kolejce: queued-second',{exact:true}).waitFor();
  const cancel=page.getByTestId('ahp-cancel-turn');
  assert.match(await cancel.textContent(),/Anuluj i zakończ sesję/);
  await cancel.click();
  await page.getByTestId('ahp-composer').waitFor({ state: 'detached' });
  await page.getByTestId('ahp-cancel-stopped').waitFor();
  assert.equal(await page.getByTestId('ahp-cancel-unknown').count(),0);
  assert.match(await page.getByTestId('ahp-cancel-stopped').textContent(),/Wynik przerwanej tury pozostaje nieznany/);
  assert.equal(await page.getByTestId('ahp-queued').count(),2);
  assert.equal(await page.getByTestId('ahp-queue-edit').count(),0);
  // Observe the actual still-connected v4 worker through the production host
  // panel. Success requires a later source heartbeat, not a one-off SQL record.
  await page.getByRole('button', { name: 'Hosty', exact: true }).click();
  await page.getByTestId('host-workspace').selectOption(fixture.workspace);
  const row = page.locator(`[data-testid="host-row"][data-host-id="${fixture.host}"]`);
  await row.getByTestId('host-health-start').click();
  await row.getByTestId('host-health-observation').waitFor();
  assert.match(await row.getByTestId('host-health-protocol').textContent(), /v4.*Sesje trwałe/);
  assert.match(await row.getByTestId('host-health-connection').textContent(), /Kanał WSS połączony/);
  const heartbeat = Number(await row.getByTestId('host-health-heartbeat').textContent());
  assert.ok(heartbeat >= 1);
  await page.waitForFunction(({host, heartbeat}) => {
    const counter = document.querySelector(`[data-testid="host-row"][data-host-id="${host}"] [data-testid="host-health-heartbeat"]`);
    return Number(counter?.textContent) > heartbeat;
  }, {host: fixture.host, heartbeat});
  assert.deepEqual(healthReads, [], 'live host observations must not use REST polling');
  assert.deepEqual(errors, []);
  await context.close();
  process.stdout.write(`${JSON.stringify({ status: 'passed', session_id: id })}\n`);
} finally {
  await browser.close();
}
