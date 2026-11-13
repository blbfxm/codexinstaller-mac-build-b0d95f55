const { chromium } = require(process.env.PLAYWRIGHT_MODULE_PATH || "playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const http = require("node:http");

(async () => {
  const root = path.resolve(__dirname, "../dist");
  const server = http.createServer((request, response) => {
    const file = path.resolve(root, request.url === "/" ? "index.html" : `.${request.url}`);
    if (!file.startsWith(`${root}${path.sep}`) || !fs.existsSync(file)) return response.writeHead(404).end();
    response.setHeader("Content-Type", { ".html": "text/html", ".js": "text/javascript", ".css": "text/css" }[path.extname(file)] || "application/octet-stream");
    fs.createReadStream(file).pipe(response);
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  let browser;
  try {
    browser = await chromium.launch({ headless: true, channel: "msedge" });
    const page = await browser.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.addInitScript(() => {
      const callbacks = {};
      const listeners = {};
      window.testInstalled = false;
      window.installCalls = 0;
      window.localizeCalls = 0;
      window.localizeOutcome = "confirmed";
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: "main" } },
        transformCallback: callback => { const id = Object.keys(callbacks).length + 1; callbacks[id] = callback; return id; },
        invoke: async (command, args) => {
          if (command === "plugin:event|listen") { listeners[args.event] = callbacks[args.handler]; return 1; }
          if (command === "load_auto_login_kami") return "fixture-card";
          if (command === "account_login") return { ok: true, account: { id: 1 }, installerConfig: { codex_macos_arm64_url: "https://example.org/Codex.dmg" } };
          if (command === "fetch_site_config") return { codex_macos_arm64_url: "https://example.org/Codex.dmg" };
          if (command === "device_info") return { platform: "macos", arch: "arm64" };
          if (command === "list_codexpp_providers") return [];
          if (command === "check_installations") return {
            codex: { installed: window.testInstalled, path: window.testInstalled ? "/Applications/Codex.app/Contents/MacOS/ChatGPT" : "", version: "" },
            codexpp: { installed: false, path: "", version: "" },
          };
          if (command === "install_product") {
            window.installCalls++;
            listeners["installer-download-progress"]?.({ payload: {
              product: "Codex", phase: "download", downloaded: 100, total: 100, bytes_per_second: 10, resumable: true, status: "",
            } });
            return "manual";
          }
          if (command === "localize_codex") {
            window.localizeCalls++;
            if (window.localizeOutcome === "write-failure") throw "保存中文设置失败";
            if (window.localizeOutcome === "unconfirmed") return "中文设置已保存，Codex 已启动。自动界面检查尚未完成；若界面已是中文，无需重复操作。若仍为英文，请等待主界面加载后再检查。";
            return "Codex 已切换为简体中文，无需重新安装。";
          }
          return null;
        },
      };
    });
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    const install = page.locator('.install-one[data-key="codex"]');
    await install.click();
    await page.waitForFunction(() => document.querySelector("#progress-title").textContent.includes("安装检测"));
    assert.equal(await page.locator("#download-pause-toggle").isHidden(), true);
    assert.equal(await page.locator("#download-stop").innerText(), "取消等待");
    assert.match(await page.locator("#progress-detail").innerText(), /拖入.*应用程序/);
    // Installed via Finder, not via the assistant: the polling must still finish.
    await page.evaluate(() => { window.testInstalled = true; });
    await page.waitForFunction(() => document.querySelector("#progress-panel").hidden);
    assert.equal(await install.innerText(), "已安装");
    const localize = page.locator(".product-menu #localize-codex");
    assert.equal(await localize.count(), 1, "localization belongs in the Codex menu");
    assert.equal(await localize.isHidden(), true, "localization stays hidden until the menu opens");
    await page.getByRole("button", { name: "Codex更多操作", exact: true }).click();
    assert.equal(await localize.isVisible(), true);
    await localize.click();
    await page.waitForFunction(() => window.localizeCalls === 1 && document.querySelector("#progress-panel").hidden);
    assert.equal(await localize.isHidden(), true, "localization closes the product menu");
    assert.equal(await page.evaluate(() => window.installCalls), 1, "localization must never download or reinstall");
    // A saved setting with an inconclusive renderer check must not leave a red failure panel.
    await page.evaluate(() => { window.localizeOutcome = "unconfirmed"; });
    await page.getByRole("button", { name: "Codex更多操作", exact: true }).click();
    await localize.click();
    await page.waitForFunction(() => window.localizeCalls === 2 && document.querySelector("#progress-panel").hidden);
    assert.match(await page.locator("#toast").innerText(), /若界面已是中文，无需重复操作/);
    assert.equal(await page.locator("#toast").evaluate(node => node.classList.contains("error")), false);
    // A real configuration write failure must still surface as a failed task.
    await page.evaluate(() => { window.localizeOutcome = "write-failure"; });
    await page.getByRole("button", { name: "Codex更多操作", exact: true }).click();
    await localize.click();
    await page.waitForFunction(() => window.localizeCalls === 3 && document.querySelector("#progress-percent").textContent === "失败");
    assert.equal(await page.locator("#progress-detail").innerText(), "保存中文设置失败");
    assert.equal(await page.locator("#progress-bar").evaluate(node => node.classList.contains("failed")), true);
    // Cancelling the installation wait must return controls to an actionable state.
    await page.evaluate(() => { window.testInstalled = false; });
    await page.locator("#refresh").click();
    await install.click();
    await page.waitForFunction(() => document.querySelector("#download-stop").textContent === "取消等待");
    await page.locator("#download-stop").click();
    await page.waitForFunction(() => document.querySelector("#progress-panel").hidden);
    assert.equal(await install.isEnabled(), true);
    assert.equal(await page.locator("#localize-codex").count(), 0, "uninstalled applications must not offer localization");
    assert.deepEqual(errors, []);
    console.log("PASS: macOS installation completion/cancellation; unconfirmed localization stays neutral, actual write failures remain errors.");
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
