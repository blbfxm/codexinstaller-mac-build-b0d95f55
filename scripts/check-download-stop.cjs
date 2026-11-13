const { chromium } = require(process.env.PLAYWRIGHT_MODULE_PATH || "playwright");
const assert = require("node:assert/strict");

(async () => {
  const browser = await chromium.launch({ headless: true, channel: "msedge" });
  try {
    const page = await browser.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.addInitScript(() => {
      const callbacks = {};
      const listeners = {};
      window.installs = [];
      window.paused = false;
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: "main" } },
        transformCallback: callback => {
          const id = Object.keys(callbacks).length + 1;
          callbacks[id] = callback;
          return id;
        },
        invoke: async (command, args) => {
          if (command === "plugin:event|listen") { listeners[args.event] = callbacks[args.handler]; return 1; }
          if (command === "load_auto_login_kami") return "test-card";
          if (command === "account_login") return { ok: true, account: { id: 1 }, installerConfig: {} };
          if (command === "fetch_site_config") return {};
          if (command === "list_codexpp_providers" || command === "list_codex_skills") return [];
          if (command === "device_info") return { platform: "windows", arch: "x64" };
          if (command === "check_installations") return {
            codex: { installed: false, path: "", version: "" },
            codexpp: { installed: false, path: "", version: "" },
          };
          if (command === "set_download_paused") { window.paused = args.paused; return null; }
          if (command === "set_download_cancelled") { window.rejectDownload("DOWNLOAD_CANCELLED"); return null; }
          if (command === "install_product" || command === "install_codex_windows_fallback") {
            window.installs.push(args.product || "codex");
            return new Promise((resolve, reject) => {
              window.rejectDownload = reject;
              listeners["installer-download-progress"]({ payload: {
                product: args.product || "codex", downloaded: 1024, total: 10240,
                bytes_per_second: 100, resumable: true,
              } });
            });
          }
          return null;
        },
      };
    });
    await page.goto("http://127.0.0.1:1420/");
    await page.locator("#install-all").waitFor();
    await page.locator('.install-one[data-key="codexpp"]').click();
    await page.locator("#download-pause-toggle").click();
    assert.equal(await page.evaluate(() => window.paused), true);
    if (await page.locator("#library-submenu").isHidden()) await page.locator('[data-page="plugins"]').click();
    await page.locator('[data-library-tab="skills"]').click();
    assert.match(await page.locator("#progress-detail").innerText(), /已暂停/);
    await page.locator("#download-stop").click();
    await page.waitForFunction(() => document.querySelector("#progress-panel").hidden);
    await page.locator('[data-page="install"]').click();
    assert.equal(await page.locator('.install-one[data-key="codexpp"]').isEnabled(), true);
    await page.locator(".setup-options-disclosure > summary").click();
    await page.locator("#setup-enable-proxy").uncheck();
    await page.locator("#setup-configure-nut").uncheck();
    await page.locator("#install-all").click();
    await page.locator("#download-stop").click();
    await page.waitForFunction(() => document.querySelector("#progress-panel").hidden);
    assert.deepEqual(await page.evaluate(() => window.installs), ["codexpp", "codex"]);
    assert.equal(await page.locator(".setup-result.error").count(), 0);
    assert.equal(await page.locator("#install-all").isEnabled(), true);
    assert.equal(await page.evaluate(() => window.paused), false);
    assert.deepEqual(errors, []);
    console.log("PASS: paused download cancellation across navigation, retry enabled, one-click batch stops without launching next product.");
  } finally { await browser.close(); }
})().catch(error => { console.error(error); process.exit(1); });
