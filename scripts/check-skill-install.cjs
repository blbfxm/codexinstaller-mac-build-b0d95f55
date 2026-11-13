const { chromium } = require(process.env.PLAYWRIGHT_MODULE_PATH || "playwright");
const assert = require("node:assert/strict");

(async () => {
  const browser = await chromium.launch({ headless: true, channel: "msedge" });
  try {
    const page = await browser.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.addInitScript(() => {
      window.codexPresent = false;
      window.codexppPresent = false;
      window.skillCalls = 0;
      window.installPending = false;
      const callbacks = {};
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: "main" } },
        transformCallback: callback => {
          const id = Object.keys(callbacks).length + 1;
          callbacks[id] = callback;
          return id;
        },
        invoke: async (command, args) => {
          if (command === "plugin:event|listen") return 1;
          if (command === "load_auto_login_kami") return "test-card";
          if (command === "account_login") return { ok: true, account: { id: 1 }, installerConfig: {} };
          if (command === "fetch_site_config") return {};
          if (command === "list_codexpp_providers") return [];
          if (command === "device_info") return { platform: "windows", arch: "x64" };
          if (command === "check_installations") return {
            codex: { installed: window.codexPresent, path: window.codexPresent ? "C:\\Codex.exe" : "", version: "" },
            codexpp: { installed: window.codexppPresent, path: window.codexppPresent ? "C:\\CodexPlusPlus.exe" : "", version: "" },
          };
          if (command === "list_codex_skills") return [{
            id: "test", name: "test", name_zh: "测试技能", display_name: "Test", description: "",
            description_zh: "测试简介", version: "1", author: "Test", tags: [], license: "MIT",
            path: "plugins/test", tree: "main", bytes: 1024, skills: 1, requires_auth: false,
            category: "测试", installed: false,
          }];
          if (command === "install_product") {
            window.installPending = true;
            await new Promise(resolve => { window.finishDownload = resolve; });
            window.installPending = false;
            window.codexppPresent = true;
            return "launched";
          }
          if (command === "install_codex_skill") {
            window.skillCalls++;
            await new Promise(resolve => { window.finishSkill = resolve; });
            return null;
          }
          return null;
        },
      };
    });
    await page.goto("http://127.0.0.1:1420/");
    await page.locator("#install-all").waitFor();
    if (await page.locator("#library-submenu").isHidden()) await page.locator('[data-page="plugins"]').click();
    await page.locator('[data-library-tab="skills"]').click();
    const install = page.locator('[data-install-skill="test"]');
    await install.click();
    await page.waitForFunction(() => document.querySelector("#toast")?.textContent.includes("请先在安装页面"));
    assert.equal(await page.evaluate(() => window.skillCalls), 0);
    await page.evaluate(() => { window.codexPresent = true; });
    await page.locator('[data-page="install"]').click();
    await page.locator('.install-one[data-key="codexpp"]').click();
    await page.waitForFunction(() => window.installPending);
    if (await page.locator("#library-submenu").isHidden()) await page.locator('[data-page="plugins"]').click();
    await page.locator('[data-library-tab="skills"]').click();
    await install.click();
    await page.waitForFunction(() => window.skillCalls === 1);
    await page.locator('[data-page="install"]').click();
    if (await page.locator("#library-submenu").isHidden()) await page.locator('[data-page="plugins"]').click();
    await page.locator('[data-library-tab="skills"]').click();
    assert.equal(await install.innerText(), "安装中…");
    assert.equal(await install.isDisabled(), true);
    await page.evaluate(() => window.finishSkill());
    await page.waitForFunction(() => document.querySelector('[data-install-skill="test"]')?.textContent === "已安装");
    assert.equal(await page.evaluate(() => window.installPending), true, "Skill installation must not stop the Codex++ download");
    await page.evaluate(() => window.finishDownload());
    await page.waitForFunction(() => !document.querySelector("#progress-panel") || document.querySelector("#progress-panel").hidden);
    assert.deepEqual(errors, []);
    console.log("PASS: Codex required, skill installation works during Codex++ download, busy state survives navigation.");
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exit(1); });
