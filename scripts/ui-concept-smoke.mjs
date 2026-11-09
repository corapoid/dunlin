// Browser composition/parity smoke only. Does not exercise native operations.
import assert from "node:assert/strict";
import { mkdir } from "node:fs/promises";
import { resolve } from "node:path";

const playwright = await import(process.env.PLAYWRIGHT_MODULE || "playwright");
const engine = process.argv.includes("--webkit") ? "webkit" : "chromium";
const browser = await playwright[engine].launch({
  headless: true,
  executablePath: process.env.PLAYWRIGHT_EXECUTABLE_PATH || undefined,
});
const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
const errors = [];
const network = [];
page.on("pageerror", (error) => errors.push(error.message));
page.on("console", (message) => {
  if (message.type() === "error") errors.push(message.text());
});
page.on("request", (request) => {
  if (/^https?:/.test(request.url())) network.push(request.url());
});
const evidence = process.env.DUNLIN_UI_CONCEPT_EVIDENCE;
const url = new URL(
  "../docs/development/ui-concepts/quiet-workspace.html",
  import.meta.url,
).href;
const visible = async (selector) =>
  assert.equal(
    await page.locator(selector).first().isVisible(),
    true,
    selector,
  );
const openCoverage = async () => {
  await page.locator("#show-coverage").click();
  await visible("#coverage-dialog[open]");
};
const closeDialog = async () => {
  if (await page.locator("dialog[open]").count())
    await page.keyboard.press("Escape");
};

try {
  await page.goto(url);
  await visible(".primary-pane");
  await visible(".agent-pane");
  await visible("#dock");
  assert.equal(await page.locator("#document-tabs [role=tab]").count(), 2);
  assert.equal(await page.locator(".send-button").isDisabled(), true);
  const inventory = await page.evaluate(() => window.DUNLIN_CAPABILITIES);
  const settings = await page.evaluate(() => window.DUNLIN_SETTINGS);
  assert.equal(settings.length, 15);
  assert.equal(
    new Set(inventory.map((item) => item.id)).size,
    inventory.length,
  );
  assert.ok(inventory.length >= 48, "Do not silently drop capability families");

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
        caret: "initial",
        animations: "allow",
      });
    }
    await page.locator("#theme").selectOption("light");
  }

  // Workbench state: canonical demo buffers outlive navigation and project changes.
  await page.locator("#edit-buffer").click();
  const buffer =
    "pub struct RetainedBuffer;\n// <script>window.injected=true</script>";
  await page.locator("#code-buffer").fill(buffer);
  await page.locator('.project-tools [data-route="git"]').click();
  await visible("#stage-file");
  await page.locator("#stage-file").click();
  await page.locator('[data-git-tab="history"]').click();
  await page.locator('[data-git-tab="worktrees"]').click();
  await page.locator('[data-git-tab="state"]').click();
  await page.locator('[data-document="editor"]').click();
  assert.equal(await page.locator("#code-buffer").inputValue(), buffer);
  await page.locator("#edit-buffer").click();
  assert.equal(await page.locator("#code-preview script").count(), 0);
  assert.equal(await page.evaluate(() => window.injected), undefined);

  await page.locator("#prompt").fill("Dunlin draft");
  await page.locator("#mode-select").selectOption("Plan");
  await page.locator("#choose-project").click();
  await page.locator('#project-options [data-project="relay"]').click();
  assert.equal(await page.locator("#prompt").inputValue(), "");
  assert.equal(await page.locator("#mode-select").inputValue(), "Build");
  await page.locator("#prompt").fill("Relay draft");
  await page.locator("#choose-project").click();
  await page.locator('#project-options [data-project="dunlin"]').click();
  assert.equal(await page.locator("#prompt").inputValue(), "Dunlin draft");
  assert.equal(await page.locator("#mode-select").inputValue(), "Plan");
  await page.locator("#edit-buffer").click();
  assert.equal(await page.locator("#code-buffer").inputValue(), buffer);
  await page.locator('[data-nav="sessions"]').click();
  await page.locator('[data-session="1"]').click();
  assert.equal(await page.locator("#prompt").inputValue(), "");
  await page.locator('[data-session="0"]').click();
  assert.equal(await page.locator("#prompt").inputValue(), "Dunlin draft");

  const untrusted =
    '<img src="https://example.invalid/no-request" onerror="window.injected=true">';
  await page.locator("#prompt").fill(untrusted);
  await page.locator(".send-button").click();
  assert.equal(
    await page.locator(".message.user").last().textContent(),
    untrusted,
  );
  assert.equal(await page.locator("#agent-content img").count(), 0);
  await page.locator('[data-lens="chat"]').focus();
  await page.keyboard.press("ArrowRight");
  await visible(".graph-flow");
  await page.keyboard.press("ArrowRight");
  await visible(".trace-row");
  await page.locator('[data-lens="chat"]').click();
  await visible("#composer");

  // All mapped routes must open real composition surfaces, not dead menu rows.
  await openCoverage();
  assert.equal(await page.locator(".coverage-row").count(), inventory.length);
  await closeDialog();
  for (const route of new Set(inventory.map((item) => item.route))) {
    await openCoverage();
    const item = inventory.find((item) => item.route === route);
    await page.locator(`[data-coverage-id="${item.id}"] button`).click();
    if (route.startsWith("settings:")) {
      await visible("#settings-dialog[open]");
      assert.equal(
        await page
          .locator(`[data-settings="${route.split(":")[1]}"]`)
          .getAttribute("aria-current"),
        "page",
      );
      await closeDialog();
    } else if (["projects", "files", "sessions"].includes(route)) {
      assert.equal(
        await page
          .locator(`[data-nav="${route}"]`)
          .getAttribute("aria-selected"),
        "true",
      );
    } else if (["chat", "graph", "trace"].includes(route)) {
      assert.equal(
        await page
          .locator(`[data-lens="${route}"]`)
          .getAttribute("aria-selected"),
        "true",
      );
    } else if (["terminal", "run", "agents", "checks"].includes(route)) {
      assert.equal(
        await page
          .locator(`[data-dock="${route}"]`)
          .getAttribute("aria-selected"),
        "true",
      );
    } else if (["context", "budget"].includes(route)) {
      await visible("#inspector");
      await page.locator("#close-inspector").click();
    } else {
      assert.equal(
        await page
          .locator(`[data-document="${route}"]`)
          .getAttribute("aria-selected"),
        "true",
      );
    }
  }

  await page.locator("#open-settings").click();
  for (const item of settings) {
    await page.locator(`[data-settings="${item.id}"]`).click();
    assert.equal(
      await page.locator("#settings-content h3").textContent(),
      item.label,
    );
  }
  await page.locator("#settings-search").fill("keychain");
  assert.equal(await page.locator("#settings-navigation button").count(), 1);
  await closeDialog();

  await page.locator('.project-tools [data-route="knowledge"]').click();
  await page.locator('[data-knowledge-tab="relations"]').click();
  await page.locator('[data-knowledge-tab="proposals"]').click();
  await page.locator("#accept-proposal").click();
  assert.match(
    await page.locator("#proposal-state").textContent(),
    /no file written/,
  );
  await page.locator('.project-tools [data-route="spec"]').click();
  await page.locator('[data-spec-tab="trace"]').click();
  await page.locator('[data-spec-tab="contract"]').click();
  await page.locator('[data-dock="terminal"]').click();
  await page.locator("#split-terminal").click();
  assert.equal(await page.locator(".terminal-panes > div").count(), 2);
  await page.locator('[data-dock="agents"]').click();
  await page.locator(".dock-card-row button").last().click();
  await visible("#workflow-dialog[open]");
  await closeDialog();
  assert.equal(
    await page
      .locator(".dock-card-row button")
      .last()
      .evaluate((el) => el === document.activeElement),
    true,
  );

  // Host selection selects a separate project; local settings do not retarget.
  await page.locator("#choose-host").click();
  await page.locator('#workflow-content [data-project="server"]').click();
  assert.equal(await page.locator("#host-name").textContent(), "Studio relay");
  assert.match(
    await page.locator("#scope-status").textContent(),
    /\/srv\/api-service/,
  );
  await page.locator("#open-settings").click();
  await page.locator('[data-settings="credentials"]').click();
  await page.locator("#settings-content [data-preview]").first().click();
  assert.match(
    await page.locator(".workflow-scope").textContent(),
    /This Mac — application settings/,
  );
  await closeDialog();
  await closeDialog();
  await page.locator("#choose-project").click();
  await page.locator('#project-options [data-project="dunlin"]').click();

  await page.locator("#open-commands").click();
  await page.locator("#command-query").fill("Task dispatch budgets");
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("Enter");
  await visible("#inspector");
  await page.keyboard.press("Escape");
  await page.locator("#open-commands").click();
  await page.locator("#command-query").fill("not-a-real-command-123");
  assert.equal(await page.locator("#command-results button").count(), 0);
  await closeDialog();
  assert.equal(
    await page
      .locator("#open-commands")
      .evaluate((el) => el === document.activeElement),
    true,
  );

  await page.locator("#reduce-motion").check();
  assert.equal(
    await page
      .locator(".workspace-body")
      .evaluate((el) => getComputedStyle(el).transitionDuration),
    "0s",
  );
  await page.locator("#reduce-motion").uncheck();
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.waitForFunction(
    () =>
      getComputedStyle(document.querySelector(".workspace-body"))
        .transitionDuration === "0s",
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
  await page.locator("#glass").uncheck();
  assert.equal(
    await page
      .locator(".app-window")
      .evaluate((el) => getComputedStyle(el).backdropFilter),
    "none",
  );

  for (const [width, height] of [
    [1600, 1000],
    [1440, 900],
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
    if (width <= 1350)
      assert.equal(
        await page.locator(".workbench").evaluate((el) => el.inert),
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
  assert.deepEqual(errors, [], "Browser console errors");
  assert.deepEqual(network, [], "External requests");
  console.log(
    `PASS (${engine}): ${inventory.length} capability families, all mapped routes and 15 settings pages; independent drafts/buffers, Chat/Graph/Trace, dock/splits, rich workspaces, host boundaries, safe messages, focus/keyboard, preferences, six viewport sizes. No network or native operations.`,
  );
} finally {
  await browser.close();
}
