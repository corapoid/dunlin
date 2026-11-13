// Real rendered production app -> real Rust HTTP/WebSocket, no mocked fetch.
import assert from 'node:assert/strict';
import { chromium } from 'playwright-core';

let input = '';
for await (const part of process.stdin) input += part;
const fixture = JSON.parse(input);
const browser = await chromium.launch({ executablePath: process.env.DUNLIN_CHROMIUM || chromium.executablePath(),
  headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
const errors = [];
try {
  const ownerContext = await browser.newContext({ viewport: { width: 1280, height: 900 }, reducedMotion: 'reduce' });
  const memberContext = await browser.newContext({ viewport: { width: 390, height: 844 }, colorScheme: 'dark' });
  const owner = await ownerContext.newPage();
  const member = await memberContext.newPage();
  for (const page of [owner, member]) {
    page.setDefaultTimeout(8000);
    page.on('pageerror', (error) => errors.push(error.message));
    page.on('request', (request) => assert.equal(new URL(request.url()).origin, fixture.origin));
  }
  async function pair(page, code) {
    await page.goto(fixture.origin);
    await page.getByLabel('Jednorazowy kod parowania').fill(code);
    await page.getByRole('button', { name: 'Połącz bezpiecznie' }).click();
    await page.getByText('Na żywo', { exact: true }).waitFor();
  }
  await pair(owner, fixture.ownerCode);
  await pair(member, fixture.memberCode);
  await member.getByRole('button', { name: /Visible project/ }).click();
  await member.getByText('Only my session', { exact: true }).waitFor();
  assert.equal(await member.getByText('Private owner session', { exact: true }).count(), 0);
  assert.equal(await member.getByText('Hidden project', { exact: true }).count(), 0);
  await member.reload();
  await member.getByText('Na żywo', { exact: true }).waitFor();
  await member.getByRole('button', { name: 'Zespół', exact: true }).click();
  await member.getByText('Nie należysz jeszcze do workspace.', { exact: false }).waitFor();
  await owner.getByRole('button', { name: 'Zespół', exact: true }).click();
  await owner.getByLabel('Nazwa', { exact: true }).fill('Browser team');
  await owner.getByRole('button', { name: 'Utwórz', exact: true }).click();
  await owner.getByRole('button', { name: 'Browser team owner' }).click();
  async function invite() {
    await owner.getByLabel('Odbiorca zaproszenia', { exact: true }).fill(fixture.memberId);
    await owner.getByRole('button', { name: 'Utwórz zaproszenie', exact: true }).click();
    await owner.getByLabel('Kod dla odbiorcy', { exact: true }).waitFor();
    return owner.getByLabel('Kod dla odbiorcy', { exact: true }).inputValue();
  }
  const revokedToken = await invite();
  await owner.getByLabel('Odbiorca do odwołania', { exact: true }).fill(fixture.memberId);
  await owner.getByRole('button', { name: 'Odwołaj zaproszenia', exact: true }).click();
  await owner.getByText('Odwołano zaproszenia wskazanego użytkownika.', { exact: true }).waitFor();
  assert.equal(await owner.getByLabel('Kod dla odbiorcy', { exact: true }).count(), 0);
  await member.getByLabel('Kod zaproszenia', { exact: true }).fill(revokedToken);
  await member.getByRole('button', { name: 'Przyjmij zaproszenie', exact: true }).click();
  await member.getByRole('alert').filter({ hasText: 'Zaproszenie wygasło' }).waitFor();
  assert.equal(await member.getByLabel('Kod zaproszenia', { exact: true }).inputValue(), '');
  const token = await invite();
  await member.getByLabel('Kod zaproszenia', { exact: true }).fill(token);
  await member.getByRole('button', { name: 'Przyjmij zaproszenie', exact: true }).click();
  await member.getByText('Dołączono do workspace.', { exact: true }).waitFor();
  await owner.getByRole('button', { name: 'Ukryj kod', exact: true }).click();
  assert.equal(await owner.getByLabel('Kod dla odbiorcy', { exact: true }).count(), 0);
  await member.getByLabel('Kod zaproszenia', { exact: true }).fill(token);
  await member.getByRole('button', { name: 'Przyjmij zaproszenie', exact: true }).click();
  await member.getByRole('alert').filter({ hasText: 'Zaproszenie wygasło' }).waitFor();
  // Membership reaches the other browser through WS invalidation, no refresh/timer.
  await member.getByRole('button', { name: 'Browser team member' }).click();
  await member.getByText('Twoja rola nie pozwala zarządzać listą członków.', { exact: true }).waitFor();
  await owner.getByLabel('Nowy projekt logiczny').fill('Private logical project');
  await owner.getByRole('button', { name: 'Utwórz projekt', exact: true }).click();
  await owner.getByText('Private logical project', { exact: true }).waitFor();
  assert.equal(await member.getByText('Private logical project', { exact: true }).count(), 0);
  const cookies = await memberContext.cookies();
  const session = cookies.find((cookie) => cookie.name === 'dunlin_web_session');
  assert.equal(session?.httpOnly, true);
  assert.equal(session?.sameSite, 'Strict');
  assert.equal(await member.evaluate(() => localStorage.length), 0);
  assert.equal(await member.evaluate(() => sessionStorage.length), 0);
  for (const page of [owner, member]) {
    assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth), 'horizontal overflow');
    assert.ok(await page.locator('h1').isVisible());
  }
  await owner.getByRole('button', { name: 'Wyloguj', exact: true }).click();
  await owner.getByLabel('Jednorazowy kod parowania').waitFor();
  // One browser logout must not revoke a different user's active device.
  await member.reload();
  await member.getByText('Na żywo', { exact: true }).waitFor();
  assert.deepEqual(errors, []);
  console.log('PASS Chromium: production pairing, real WS, two users, private sessions/projects, live invitation acceptance/revocation/replay, responsive light/dark, reload and isolated logout.');
} finally { await browser.close(); }
