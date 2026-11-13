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
    const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css" };
    response.setHeader("Content-Type", types[path.extname(file)] || "application/octet-stream");
    fs.createReadStream(file).pipe(response);
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  let browser;
  try {
    browser = await chromium.launch({ headless: true, channel: "msedge" });
    const page = await browser.newPage({ viewport: { width: 1120, height: 900 } });
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.addInitScript(() => {
      window.savedOptions = [];
      window.savedProtocols = [];
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: "main" } },
        transformCallback: () => 1,
        invoke: async (command, args) => {
          if (command === "load_auto_login_kami") return "test-kami";
          if (command === "account_login") return { ok: true, account: { id: 1 }, installerConfig: {} };
          if (command === "device_info") return { platform: "windows", arch: "x64" };
          if (command === "fetch_site_config") return {};
          if (command === "check_installations") return {
            codex: { installed: true, path: "Codex.exe", version: "" },
            codexpp: { installed: true, path: "CodexPlusPlus.exe", version: "" },
          };
          if (command === "list_codexpp_providers") return [];
          if (command === "configure_codexpp_provider") {
            window.savedOptions.push(args.configureCodexppStartCodex);
            window.savedProtocols.push(args.protocol);
            return args.protocol === "responses"
              ? "供应商与 Codex 认证已同步配置，重启 Codex 后生效"
              : "供应商已保存；请在 Codex++ 中启用此供应商";
          }
          return null;
        },
      };
    });
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    for (const enabled of [true, false]) {
      await page.locator('[data-page="install"]').click();
      await page.locator(".setup-options-disclosure > summary").click();
      assert.equal(await page.locator("#option-codexpp-chinese").isChecked(), true);
      assert.equal(await page.locator("#option-codexpp-chinese").locator("..").locator("b").innerText(), "设置 Codex 强制中文界面");
      assert.equal(await page.locator("#option-codexpp-start-codex").isChecked(), true);
      if (!enabled) await page.locator("#option-codexpp-start-codex").uncheck();
      await page.locator('[data-page="config"]').click();
      await page.locator("#provider-new").click();
      await page.locator("#provider-custom").click();
      await page.locator("#provider-name").fill("Test");
      await page.locator("#provider-url").fill("https://example.test/v1");
      await page.locator("#provider-key").fill("test-key");
      await page.locator("#provider-model").fill("test-model");
      if (!enabled) await page.locator("#provider-protocol").selectOption("chatCompletions");
      await page.locator("#configure-provider").click();
      await page.locator("#provider-new").waitFor();
      await page.getByText(enabled
        ? "供应商与 Codex 认证已同步配置，重启 Codex 后生效"
        : "供应商已保存；请在 Codex++ 中启用此供应商", { exact: true }).waitFor();
    }
    assert.deepEqual(await page.evaluate(() => window.savedOptions), [true, false]);
    assert.deepEqual(await page.evaluate(() => window.savedProtocols), ["responses", "chatCompletions"]);
    assert.deepEqual(errors, []);
    console.log("PASS: auto-start and protocol options submit correctly; direct and conversion results are shown accurately.");
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
