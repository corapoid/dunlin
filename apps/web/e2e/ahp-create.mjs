// Production Agent Builder -> published profile -> production creation UI -> AHP.
import assert from 'node:assert/strict';
import { createInterface } from 'node:readline';
import { chromium } from 'playwright-core';
const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
const input = lines[Symbol.asyncIterator]();
async function receive() {
  const line = await input.next();
  assert.equal(line.done, false);
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
  const context = await browser.newContext();
  const page = await context.newPage();
  page.setDefaultTimeout(15000);
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('request', (request) => assert.equal(new URL(request.url()).origin, fixture.origin));
  await page.goto(fixture.origin);
  await page.getByLabel('Jednorazowy kod parowania').fill(fixture.ownerCode);
  await page.getByRole('button', { name: 'Połącz bezpiecznie', exact: true }).click();
  await page.getByRole('button', { name: 'Sesje', exact: true }).waitFor();
  const discovery = await page.evaluate(async () => (await fetch('/v1/web/session')).json());
  const attempts = [];
  let lostAcceptance;
  await page.route('**/interactive-sessions', async (route) => {
    const request = route.request();
    if (request.method() !== 'POST') return route.continue();
    attempts.push({
      body: request.postData(),
      key: request.headers()['idempotency-key'],
      user: request.headers()['x-dunlin-expected-user'],
      device: request.headers()['x-dunlin-expected-device'],
      url: request.url(),
    });
    assert.equal(attempts.at(-1).user, discovery.user.id);
    assert.equal(attempts.at(-1).device, discovery.device_id);
    if (attempts.length === 1) {
      // Real POST reaches the server and returns acceptance; only its HTTP response
      // is dropped. No fake success payload, mocked actor or second create call.
      const response = await route.fetch();
      assert.equal(response.status(), 202);
      assert.equal(response.headers()['idempotency-replayed'], 'false');
      lostAcceptance = await response.json();
      await route.abort('failed');
    } else await route.continue();
  });
  await page.getByRole('button', { name: 'Agenci', exact: true }).click();
  await page.getByTestId('agent-workspace').selectOption(fixture.workspace);
  await page.getByTestId('agent-project').selectOption(fixture.project);
  await page.getByTestId('agent-name').fill('Browser pinned agent');
  await page.getByTestId('agent-model').fill('fixture-model');
  await page.getByTestId('agent-mode').fill('plan');
  await page.getByTestId('agent-instructions').fill('BROWSER_TOP_INSTRUCTIONS');
  await page.getByTestId('agent-skill-add').click();
  const skill = page.getByTestId('agent-skill');
  await skill.getByLabel('Identyfikator umiejętności').fill('browser-skill');
  await skill.getByLabel('Wersja umiejętności').fill('1.0.0');
  await skill.getByLabel('Instrukcje umiejętności').fill('BROWSER_SKILL_INSTRUCTIONS');
  await page.getByTestId('agent-save').click();
  await page.getByText('Szkic zapisany. Publikacja wymaga osobnej akcji.', { exact: true }).waitFor();
  await page.getByTestId('agent-publish').click();
  await page.getByText('Opublikowano wersję 1.', { exact: false }).waitFor();
  await page.getByRole('button', { name: 'Sesje', exact: true }).click();
  await page.getByTestId('session-create').locator('summary').click();
  await page.getByTestId('agent-workspace').selectOption(fixture.workspace);
  await page.getByTestId('agent-project').selectOption(fixture.project);
  await page.getByTestId('session-create-profile').selectOption({ label: 'Browser pinned agent · wersja 1' });
  await page.getByTestId('session-create-submit').click();
  await page.getByTestId('session-create-retry').waitFor();
  assert.equal(attempts.length, 1);
  assert.equal(await page.getByTestId('session-create-submit').isDisabled(), true);
  const pinned = JSON.parse(attempts[0].body);
  assert.equal(pinned.profile.version, 1);
  assert.match(pinned.profile.digest, /^[a-f0-9]{64}$/);
  assert.deepEqual(Object.keys(pinned), ['profile']);
  await source('verify-accepted');
  // The unresolved intent survives navigation; it cannot mint a second identity.
  await page.getByRole('button', { name: 'Agenci', exact: true }).click();
  await page.getByRole('button', { name: 'Sesje', exact: true }).click();
  await page.getByTestId('session-create').locator('summary').click();
  await page.getByTestId('session-create-retry').click();
  const session = lostAcceptance.session.id;
  await page.getByTestId('session-created-id').filter({ hasText: session }).waitFor();
  assert.equal(attempts.length, 2);
  assert.deepEqual(attempts[1], attempts[0]);
  await source('verify-accepted');
  assert.equal(await page.getByTestId('session-create-retry').count(), 0);
  // Historical HTTP phase never gates prompting. Obtain a fresh standard AHP
  // session snapshot and/or session/ready from the current actor before UI input.
  const current = await page.evaluate(
    (session) =>
      new Promise((resolve, reject) => {
        const socket = new WebSocket(`${location.origin.replace(/^http/, 'ws')}/v1/web/ahp`);
        const timer = setTimeout(() => {
          socket.close();
          reject(new Error('Current AHP lifecycle deadline'));
        }, 12000);
        const finish = (state) => {
          clearTimeout(timer);
          socket.close();
          resolve(state);
        };
        socket.onopen = () =>
          socket.send(
            JSON.stringify({
              jsonrpc: '2.0',
              id: 1,
              method: 'initialize',
              params: {
                channel: 'ahp-root://',
                clientId: crypto.randomUUID(),
                protocolVersions: ['1.0.0'],
                initialSubscriptions: ['ahp-root://'],
              },
            }),
          );
        socket.onmessage = (event) => {
          const m = JSON.parse(event.data);
          if (m.error) {
            clearTimeout(timer);
            socket.close();
            reject(new Error(JSON.stringify(m.error)));
            return;
          }
          if (m.id === 1)
            socket.send(
              JSON.stringify({
                jsonrpc: '2.0',
                id: 2,
                method: 'subscribe',
                params: { channel: `ahp-session:/${session}` },
              }),
            );
          if (m.id === 2 && m.result.snapshot.state.lifecycle === 'ready')
            finish({ resource: m.result.snapshot.resource, lifecycle: 'ready' });
          if (
            m.method === 'action' &&
            m.params.channel === `ahp-session:/${session}` &&
            m.params.action.type === 'session/ready'
          )
            finish({ resource: m.params.channel, lifecycle: 'ready' });
        };
      }),
    session,
  );
  assert.deepEqual(current, { resource: `ahp-session:/${session}`, lifecycle: 'ready' });
  await page.getByRole('button', { name: 'Sesje', exact: true }).click();
  await page.getByTestId('ahp-status').getByText('Na żywo', { exact: true }).waitFor();
  assert.equal(await page.getByTestId('ahp-session').count(), 1);
  await page.locator(`[data-testid="ahp-session"][data-session-uri="ahp-session:/${session}"]`).click();
  await page.getByTestId('ahp-prompt').fill(fixture.prompt);
  await page.getByTestId('ahp-send').click();
  const transcript = page.getByTestId('ahp-transcript');
  await transcript.getByText('reply: BROWSER_TOP_INSTRUCTIONS', { exact: false }).waitFor();
  await transcript.getByText('BROWSER_SKILL_INSTRUCTIONS', { exact: false }).waitFor();
  await transcript.getByText('Agent', { exact: true }).waitFor();
  await source('verify-completed', { session });
  await page.getByRole('button', { name: 'Agenci', exact: true }).click();
  await page.getByTestId('agent-workspace').selectOption(fixture.workspace);
  await page.getByTestId('agent-project').selectOption(fixture.project);
  await page.getByTestId('agent-name').fill('MCP pinned profile');
  await page.getByTestId('agent-model').fill('fixture-model');
  await page.getByTestId('agent-mode').fill('plan');
  await page.getByTestId('agent-mcp-add').click();
  const mcp = page.getByTestId('agent-mcp-tool');
  const providerInput=mcp.getByLabel('Dostawca MCP (ID)');
  await providerInput.fill('work-id');assert.equal(await providerInput.evaluate(input=>input.checkValidity()),true);
  await providerInput.fill('bad?');assert.equal(await providerInput.evaluate(input=>input.checkValidity()),false);
  await providerInput.fill('work');
  await mcp.getByLabel('Nazwa narzędzia').fill('lookup');
  await mcp.getByLabel('Odcisk definicji narzędzia').fill('a'.repeat(64));
  await page.getByTestId('agent-save').click();
  await page.getByText('Szkic zapisany. Publikacja wymaga osobnej akcji.', { exact: true }).waitFor();
  await page.getByTestId('agent-publish').click();
  await page.getByText('Opublikowano wersję 1.', { exact: false }).waitFor();
  const profiles = await page.evaluate(async ({workspace,project}) => (await fetch(`/v1/web/workspaces/${workspace}/projects/${project}/agent-profiles`)).json(), fixture);
  const published = profiles.profiles.find(profile=>profile.definition.name==='MCP pinned profile');
  assert.equal(published.definition.schema_version,2);
  assert.deepEqual(published.definition.mcp,[{provider_id:'work',name:'lookup',definition_digest:'a'.repeat(64)}]);
  for (const [name,width] of [['DESKTOP',1440],['MOBILE',390]]) {
    await page.setViewportSize({width,height:1100});await mcp.scrollIntoViewIfNeeded();
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
    console.log(`AGENT_MCP_IMAGE_${name} ${(await page.screenshot()).toString('base64')}`);
  }
  await page.getByRole('button', { name: 'Sesje', exact: true }).click();
  await page.getByTestId('session-create').locator('summary').click();
  await page.getByTestId('agent-workspace').selectOption(fixture.workspace);
  await page.getByTestId('agent-project').selectOption(fixture.project);
  const unavailable=page.getByTestId('session-create-profile').locator('option').filter({hasText:'MCP pinned profile'});
  assert.equal(await unavailable.isDisabled(),true);
  assert.match(await unavailable.innerText(),/MCP niedostępne/);
  assert.equal(attempts.length, 2);
  assert.deepEqual(errors, []);
  console.log(
    'PASS Chromium production Agent Builder/creation UI: saved and published actual instructions/skill, pinned profile, lost real HTTP acceptance, navigation retains unknown intent, explicit identical retry, one durable session/ACP process, current AHP ready, one context-prefixed completed prompt.',
  );
} finally {
  await browser.close();
  lines.close();
  process.stdin.destroy();
}
