// Real production UI -> relay -> isolated signing provider -> browser callback.
import assert from 'node:assert/strict';
import { chromium } from 'playwright-core';
let input = '';
for await (const part of process.stdin) input += part;
const fixture = JSON.parse(input);
const browser = await chromium.launch({ executablePath: process.env.DUNLIN_CHROMIUM || chromium.executablePath(), headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
try {
  const context = await browser.newContext();
  const page = await context.newPage();
  page.setDefaultTimeout(10000);
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('request', (request) => assert.ok([fixture.origin, fixture.issuer].includes(new URL(request.url()).origin)));
  await page.goto(fixture.origin);
  await page.getByRole('button', { name: 'Zaloguj kontem organizacji', exact: true }).click();
  await page.getByText('Na żywo', { exact: true }).waitFor();
  assert.equal(page.url(), `${fixture.origin}/`);
  const cookies = await context.cookies();
  const session = cookies.find((cookie) => cookie.name === 'dunlin_web_session');
  assert.equal(session?.httpOnly, true);
  assert.equal(session?.sameSite, 'Strict');
  assert.ok(session.expires <= Date.now() / 1000 + 600);
  assert.equal(cookies.find((cookie) => cookie.name === 'dunlin_oidc_flow'), undefined);
  assert.equal(await page.evaluate(() => localStorage.length + sessionStorage.length), 0);
  await page.reload();
  await page.getByText('Na żywo', { exact: true }).waitFor();
  await page.getByRole('button', { name: 'Wyloguj', exact: true }).click();
  await page.getByRole('button', { name: 'Zaloguj kontem organizacji', exact: true }).waitFor();
  assert.deepEqual(errors, []);
  console.log('PASS Chromium OIDC: production UI, real discovery/JWKS/code exchange/PKCE/RS256, callback cookie, live catalog, reload, logout, no browser storage.');
} finally { await browser.close(); }
