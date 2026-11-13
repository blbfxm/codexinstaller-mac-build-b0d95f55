const { chromium } = require(process.env.PLAYWRIGHT_MODULE_PATH || "playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const http = require("node:http");

(async () => {
  const root = path.resolve(__dirname, "../dist");
  const output = path.resolve(__dirname, "../src-tauri/target/wechat-ui-check");
  fs.mkdirSync(output, { recursive: true });
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
    const page = await browser.newPage({ viewport: { width: 1200, height: 900 } });
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.addInitScript(() => {
      window.calls = [];
      window.runtimeReady = false;
      window.qrMode = "wait";
      window.startFails = false;
      window.wx = { bound: false, running: false, workspace: "", writable: false, phase: "尚未连接", error: "", received: 0, replied: 0, activity: "", hasSession: false };
      const callbacks = {};
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: "main" } },
        transformCallback: callback => { const id = Object.keys(callbacks).length + 1; callbacks[id] = callback; return id; },
        invoke: async (command, args) => {
          window.calls.push({ command, args });
          if (command === "load_auto_login_kami") return "fixture-card";
          if (command === "account_login") return { ok: true, account: { id: 1 } };
          if (command === "fetch_site_config") return {};
          if (command === "device_info") return { platform: "macos", arch: "arm64" };
          if (command === "list_codexpp_providers") return [];
          if (command === "check_installations") return { codex: { installed: true, path: "/Applications/ChatGPT.app/Contents/MacOS/ChatGPT", version: "26.11" }, codexpp: { installed: false, path: "", version: "" } };
          if (command === "wechat_action") {
            const name = args.action;
            if (name === "preflight") return { ready: window.runtimeReady, message: window.runtimeReady ? "Codex 会话组件可用" : "已检测到 Codex，但安装中缺少可用的会话组件。请更新 Codex 后重新检测。" };
            if (name === "chooseFolder") return { path: "/Users/test/Projects/example" };
            if (name === "qr") {
              if (!window.runtimeReady) throw "Codex 会话组件检查失败，请更新 Codex 后重试";
              if (!args.payload.workspace) throw "请先选择工作文件夹";
              if (window.qrMode === "hold") return new Promise(resolve => { window.releaseQr = resolve; });
              if (window.qrMode === "error") throw "获取微信二维码超时，请检查网络后重试";
              return { content: "https://example.test/wechat-fixture" };
            }
            if (name === "pollQr") {
              if (window.qrMode === "wait") return { status: "wait" };
              window.wx.bound = true;
              return { status: "confirmed" };
            }
            if (name === "start") {
              if (window.startFails) { window.runtimeReady = false; throw "Codex 会话组件无法运行，请更新后重新检测"; }
              window.wx = { ...window.wx, ...args.payload, running: true, phase: "等待微信消息", error: "" };
            }
            if (name === "stop") window.wx.running = false;
            return { ...window.wx };
          }
          return null;
        },
      };
    });
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.locator('[data-page="wechat"]').click();
    await page.waitForFunction(() => document.querySelector("#wechat-environment-status")?.dataset.ready === "false");
    assert.equal(await page.locator("#wechat-start").isDisabled(), true);
    assert.equal(await page.evaluate(() => window.calls.some(call => call.args?.action === "qr")), false, "Dependency failure must be visible before generating a QR");
    await page.locator("#wechat-folder-select").click();
    assert.equal(await page.locator("#wechat-workspace").inputValue(), "/Users/test/Projects/example");
    await page.screenshot({ path: path.join(output, "mac-component-missing.png"), fullPage: true });
    await page.evaluate(() => { window.runtimeReady = true; });
    await page.locator("#wechat-recheck").click();
    await page.waitForFunction(() => !document.querySelector("#wechat-start").disabled);
    assert.equal(await page.locator("#wechat-start").innerText(), "扫码并开启连接");

    // A cancelled slow QR must not appear later or start a connection.
    await page.evaluate(() => { window.qrMode = "hold"; });
    await page.locator("#wechat-start").click();
    await page.waitForFunction(() => !!window.releaseQr);
    assert.equal(await page.locator("#wechat-folder-select").isEnabled(), true);
    await page.locator("#wechat-cancel-scan").click();
    await page.evaluate(() => window.releaseQr({ content: "https://example.test/stale" }));
    await page.waitForTimeout(100);
    assert.equal(await page.locator("#wechat-qr svg").count(), 0);
    assert.equal(await page.evaluate(() => window.calls.some(call => call.args?.action === "start")), false);
    await page.evaluate(() => { window.qrMode = "error"; });
    await page.locator("#wechat-start").click();
    await page.waitForFunction(() => document.querySelector("#wechat-error").textContent.includes("超时"));
    assert.equal(await page.locator("#wechat-start").isEnabled(), true);
    assert.equal(await page.locator("#wechat-cancel-scan").isHidden(), true);

    // One action scans and starts. Failure after binding must preserve that binding.
    await page.evaluate(() => { window.qrMode = "confirmed"; window.startFails = true; });
    await page.locator("#wechat-start").click();
    await page.waitForFunction(() => document.querySelector("#wechat-environment-status").dataset.ready === "false" && window.wx.bound);
    assert.match(await page.locator("#wechat-owner").innerText(), /无需重复扫码/);
    assert.equal(await page.locator("#wechat-start").innerText(), "开启微信连接");
    const qrCalls = await page.evaluate(() => window.calls.filter(call => call.args?.action === "qr").length);
    await page.evaluate(() => { window.runtimeReady = true; window.startFails = false; });
    await page.locator("#wechat-recheck").click();
    await page.waitForFunction(() => !document.querySelector("#wechat-start").disabled);
    await page.locator("#wechat-start").click();
    await page.waitForFunction(() => document.querySelector("#wechat-badge").textContent === "连接已开启");
    assert.equal(await page.locator("#wechat-error").isHidden(), true);
    assert.equal(await page.evaluate(() => window.calls.filter(call => call.args?.action === "qr").length), qrCalls);
    const starts = await page.evaluate(() => window.calls.filter(call => call.args?.action === "start"));
    assert.equal(starts.at(-1).args.payload.writable, false);
    assert.equal(starts.at(-1).args.payload.workspace, "/Users/test/Projects/example");
    assert.equal(await page.evaluate(() => window.calls.find(call => call.args?.action === "qr").args.payload.workspace), "/Users/test/Projects/example");
    const stopCount = await page.evaluate(() => window.calls.filter(call => call.args?.action === "stop").length);
    await page.locator('[data-page="config"]').click();
    await page.locator('[data-page="wechat"]').click();
    await page.waitForFunction(() => !document.querySelector("#wechat-stop").disabled);
    assert.equal(await page.evaluate(() => window.calls.filter(call => call.args?.action === "stop").length), stopCount);
    for (const width of [1200, 760]) {
      await page.setViewportSize({ width, height: 900 });
      for (const theme of ["dark", "light"]) {
        await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
        await page.screenshot({ path: path.join(output, `mac-connected-${width}-${theme}.png`), fullPage: true });
      }
    }
    await page.locator("#wechat-stop").click();
    await page.waitForFunction(() => !document.querySelector("#wechat-start").disabled);
    assert.equal(await page.locator("#wechat-start").innerText(), "开启微信连接");
    // Background error messages are cleared when the backend clears them.
    await page.evaluate(() => { window.wx.error = "连接临时中断"; });
    await page.waitForFunction(() => document.querySelector("#wechat-error").textContent === "连接临时中断");
    await page.evaluate(() => { window.wx.error = ""; });
    await page.waitForFunction(() => document.querySelector("#wechat-error").hidden);
    assert.deepEqual(errors, []);
    console.log("PASS: macOS dependency preflight; folder selection; QR cancellation/error recovery; auto-connect; bound retry without re-scanning; default read-only; navigation and stop; both themes and widths.");
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
