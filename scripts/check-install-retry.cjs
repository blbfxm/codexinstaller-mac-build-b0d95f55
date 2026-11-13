const { chromium } = require(process.env.PLAYWRIGHT_MODULE_PATH || "playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const http = require("node:http");

(async () => {
  const root = path.resolve(__dirname, "../dist");
  const server = http.createServer((request, response) => {
    const pathname = new URL(request.url, "http://localhost").pathname;
    const file = path.resolve(root, pathname === "/" ? "index.html" : `.${pathname}`);
    if (!file.startsWith(`${root}${path.sep}`) || !fs.existsSync(file)) {
      response.writeHead(404).end();
      return;
    }
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
      let attempts = 0;
      let installed = false;
      let resume;
      const callbacks = {};
      const listeners = {};
      window.finishRetry = () => resume();
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
          if (command === "list_codexpp_providers") return [{ id: "nut-test", name: "坚果", baseUrl: "https://api.jianguo.ltd/v1", model: "test-model", models: ["test-model"], apiKey: "test-key", hasApiKey: true, isActive: true }];
          if (command === "device_info") return { platform: "windows", arch: "x64" };
          if (command === "system_proxy_state") return { supported: true, enabled: false, assistantManaged: false };
          if (command === "check_installations") return {
            codex: { installed: true, path: "Codex.exe", version: "" },
            codexpp: { installed, path: installed ? "CodexPlusPlus.exe" : "", version: "" },
          };
          if (command === "install_product") {
            attempts++;
            if (attempts === 1) {
              listeners["installer-download-progress"]?.({ payload: {
                product: "Codex++", downloaded: 0, total: 1024,
                bytes_per_second: 0, resumable: true, status: "连接中断，正在重新连接（3/3）",
              } });
              throw "Codex++ 下载连接失败，已下载内容会保留";
            }
            await new Promise(resolve => { resume = resolve; window.retryReady = true; });
            installed = true;
            return "installed";
          }
          return null;
        },
      };
    });
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.locator(".setup-options-disclosure > summary").click();
    await page.locator("#setup-enable-proxy").uncheck();
    await page.locator("#setup-configure-nut").uncheck();
    await page.locator("#include-codexpp").check();
    await page.locator("#install-all").click();
    await page.locator(".setup-result.error").waitFor();
    await page.locator('.install-one[data-key="codexpp"]').click();
    assert.equal(await page.locator(".setup-result.error").count(), 0);
    await page.waitForFunction(() => window.retryReady && document.querySelector("#progress-panel") && !document.querySelector("#progress-panel").hidden);
    await page.evaluate(() => window.finishRetry());
    await page.waitForFunction(() => document.querySelector("#progress-panel").hidden);
    assert.equal(await page.locator(".setup-result.error").count(), 0);
    assert.doesNotMatch(await page.locator(".setup-result").innerText(), /失败|中断|重新连接/);
    await page.locator('[data-page="config"]').click();
    await page.locator('[data-page="install"]').click();
    assert.equal(await page.locator(".setup-result.error").count(), 0);
    assert.doesNotMatch(await page.locator(".setup-result").innerText(), /失败|中断|重新连接/);
    assert.deepEqual(errors, []);
    console.log("PASS: failed batch message clears on individual retry and stays cleared after successful install and navigation.");
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
