// Optional browser-only verification. No native app, agent or repository writes.
// Requires an externally installed Playwright and its Chromium/WebKit browser.
import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const playwright = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const engine = process.argv.includes("--webkit") ? "webkit" : "chromium";
const browser = await playwright[engine].launch({
  headless: true,
  executablePath: process.env.PLAYWRIGHT_EXECUTABLE_PATH || undefined,
});
const context = await browser.newContext({
  viewport: { width: 1440, height: 1000 },
});
const page = await context.newPage();
const errors = [];
const externalRequests = [];
page.on("pageerror", (error) => errors.push(error.message));
page.on("console", (message) => {
  if (message.type() === "error") errors.push(message.text());
});
page.on("request", (request) => {
  if (/^https?:/.test(request.url())) externalRequests.push(request.url());
});
const url = new URL(
  "../docs/development/ui-concepts/quiet-workspace.html",
  import.meta.url,
).href;
const evidence = process.env.DUNLIN_UI_CONCEPT_EVIDENCE;
const visible = async (selector) =>
  assert.equal(await page.locator(selector).isVisible(), true, selector);
const panel = async (name) => {
  for (const candidate of ["work", "changes", "activity"]) {
    assert.equal(
      await page.locator(`#view-${candidate}`).isVisible(),
      candidate === name,
    );
  }
};

try {
  await page.goto(url);
  await panel("work");
  assert.equal(
    await page.locator("#conversation-title").textContent(),
    "Refine the workspace",
  );
  assert.equal(await page.locator(".send-button").isDisabled(), true);

  // Capture the unmodified design, not the messages used for security testing.
  if (evidence) {
    await mkdir(resolve(evidence), { recursive: true });
    for (const theme of ["light", "dark"]) {
      await page.locator("#theme").selectOption(theme);
      await page.locator("#theme").evaluate((el) => el.blur());
      await page
        .locator(".app-window")
        .evaluate((el) =>
          Promise.all(
            el
              .getAnimations({ subtree: true })
              .map((animation) => animation.finished),
          ),
        );
      await page.screenshot({
        path: resolve(evidence, `quiet-workspace-${engine}-${theme}.png`),
        fullPage: true,
        // Keep captures faithful to the settled page, including native controls.
        animations: "allow",
        caret: "initial",
      });
    }
    await page.locator("#theme").selectOption("light");
  }

  // Drafts belong to individual conversations and projects.
  await page.locator("#prompt").fill("Dunlin draft");
  await page.locator("#choose-project").click();
  await page.locator('[data-project="relay"]').click();
  assert.equal(await page.locator("#prompt").inputValue(), "");
  await page.locator("#prompt").fill("Relay draft");
  await page.locator("#choose-project").click();
  await page.locator('[data-project="dunlin"]').click();
  assert.equal(await page.locator("#prompt").inputValue(), "Dunlin draft");
  await page.locator(".conversation-link").nth(1).click();
  assert.equal(await page.locator("#prompt").inputValue(), "");
  await page.locator(".conversation-link").first().click();
  assert.equal(await page.locator("#prompt").inputValue(), "Dunlin draft");

  // Untrusted demo messages are text, not markup, and no agent is called.
  const text =
    '<img src="https://example.invalid/never" onerror="window.injected=true">';
  await page.locator("#prompt").fill(text);
  await page.locator(".send-button").click();
  assert.equal(await page.locator(".user-message").last().textContent(), text);
  assert.equal(await page.locator("#transcript img").count(), 0);
  assert.equal(await page.evaluate(() => window.injected), undefined);
  assert.equal(await page.locator("#prompt").inputValue(), "");

  await page.locator("#toggle-inspector").click();
  await visible("#inspector");
  await page.locator("#detail-context").focus();
  await page.keyboard.press("ArrowRight");
  await visible("#inspector-files");
  await page.locator('[data-open-file="1"]').click();
  await panel("changes");
  assert.equal(
    await page.locator("#diff-filename").textContent(),
    "src/palette.rs",
  );
  await page.locator("#mark-reviewed").click();
  assert.equal(
    await page.locator("#review-progress").textContent(),
    "1 of 3 files reviewed",
  );
  await page.locator("#mark-reviewed").click();
  assert.equal(
    await page.locator("#review-progress").textContent(),
    "0 of 3 files reviewed",
  );
  await page.locator("#draft-commit").click();
  await visible("#commit-dialog[open]");
  await page.keyboard.press("Escape");
  assert.equal(
    await page
      .locator("#draft-commit")
      .evaluate((el) => el === document.activeElement),
    true,
  );
  await page.locator("#close-inspector").click();
  assert.equal(await page.locator("#inspector").isVisible(), false);

  // Roving keyboard focus follows the selected destination.
  await page.locator("#tab-changes").focus();
  await page.keyboard.press("ArrowRight");
  await panel("activity");
  assert.equal(
    await page.locator("#tab-activity").getAttribute("tabindex"),
    "0",
  );
  await page.locator(".timeline summary").first().click();
  assert.equal(
    await page.locator(".timeline details").first().getAttribute("open"),
    "",
  );

  await page.locator("#open-commands").click();
  await page.locator("#command-query").fill("palette.rs");
  assert.equal(await page.locator("#command-results button").count(), 1);
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await panel("changes");
  await page.locator("#open-commands").click();
  await page.locator("#command-query").fill("no-such-command-123");
  assert.equal(await page.locator("#command-results button").count(), 0);
  await page.keyboard.press("Escape");
  assert.equal(
    await page
      .locator("#open-commands")
      .evaluate((el) => el === document.activeElement),
    true,
  );

  await page.locator("#new-conversation").click();
  await panel("work");
  assert.equal(await page.locator("#transcript").textContent(), "");
  await page.locator("#open-settings").click();
  assert.equal(await page.locator(".settings-groups > div").count(), 6);
  await page.keyboard.press("Escape");

  // System preference and app preference are additive, not competing switches.
  await page.locator("#reduce-motion").check();
  assert.equal(
    await page
      .locator(".workspace-body")
      .evaluate((el) => getComputedStyle(el).transitionDuration),
    "0s",
  );
  await page.locator("#reduce-motion").uncheck();
  await page.emulateMedia({ reducedMotion: "reduce" });
  assert.equal(
    await page
      .locator(".workspace-body")
      .evaluate((el) => getComputedStyle(el).transitionDuration),
    "0s",
  );
  await page.emulateMedia({
    reducedMotion: "no-preference",
    colorScheme: "dark",
  });
  await page.waitForFunction(
    () => matchMedia("(prefers-color-scheme: dark)").matches,
  );
  await page.locator("#theme").selectOption("system");
  await page.waitForFunction(
    () => document.documentElement.dataset.theme === "dark",
  );
  assert.equal(await page.locator("html").getAttribute("data-theme"), "dark");
  await page.locator("#glass").uncheck();
  assert.equal(
    await page
      .locator(".app-window")
      .evaluate((el) => getComputedStyle(el).backdropFilter),
    "none",
  );
  assert.match(
    await page
      .locator(".app-window")
      .evaluate((el) => getComputedStyle(el).backgroundColor),
    /^rgb\(/,
  );
  await page.locator("#glass").check();

  // No horizontal page overflow; drawers remain keyboard-dismissible.
  for (const [width, height] of [
    [1440, 1000],
    [1100, 760],
    [800, 600],
    [390, 844],
    [320, 568],
  ]) {
    await page.setViewportSize({ width, height });
    assert.equal(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
      true,
      `${width}: page overflow`,
    );
    await page.locator("#toggle-inspector").click();
    await visible("#inspector");
    if (width <= 860)
      assert.equal(
        await page.locator(".work-area").evaluate((el) => el.inert),
        true,
      );
    await page.keyboard.press("Escape");
    assert.equal(await page.locator("#inspector").isVisible(), false);
    if (width <= 860) {
      await page.locator("#toggle-sidebar").click();
      assert.equal(
        await page.locator("#navigator").evaluate((el) => el.inert),
        false,
      );
      await page.keyboard.press("Escape");
      assert.equal(
        await page.locator("#navigator").evaluate((el) => el.inert),
        true,
      );
    }
  }
  assert.deepEqual(
    externalRequests,
    [],
    "The prototype must not make network requests",
  );
  assert.deepEqual(errors, [], "Browser console errors");
  console.log(
    `PASS (${engine}): navigation, project/session drafts, safe demo messages, file review, dialogs/focus, keyboard tabs, appearance/accessibility preferences, five viewport sizes, no network or console errors.`,
  );
} finally {
  await browser.close();
}
