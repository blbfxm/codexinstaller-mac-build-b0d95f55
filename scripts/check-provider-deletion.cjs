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
    if (!file.startsWith(`${root}${path.sep}`) || !fs.existsSync(file)) return response.writeHead(404).end();
    response.setHeader("Content-Type", { ".html": "text/html", ".js": "text/javascript", ".css": "text/css" }[path.extname(file)] || "application/octet-stream");
    fs.createReadStream(file).pipe(response);
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  let browser;
  try {
    browser = await chromium.launch({ headless: true, channel: "msedge" });
    for (const platform of ["windows", "macos"]) {
      const page = await browser.newPage();
      const errors = [];
      page.on("pageerror", error => errors.push(error.message));
      await page.addInitScript(platform => {
        window.confirm = () => { throw new Error("Native confirm is unavailable in this WebView"); };
        let providers = [{ id: "active", name: "Active Provider", isActive: true, isCurrent: true, baseUrl: "https://example.test/v1", model: "test-model", models: [], apiKey: "test-key", hasApiKey: true }];
        window.deleted = [];
        window.__TAURI_INTERNALS__ = {
          metadata: { currentWindow: { label: "main" } },
          transformCallback: () => 1,
          invoke: async (command, args) => {
            if (command === "load_auto_login_kami") return "test-card";
            if (command === "account_login") return { ok: true, account: { id: 1 }, installerConfig: {} };
            if (command === "device_info") return { platform, arch: platform === "macos" ? "arm64" : "x64" };
            if (command === "fetch_site_config") return {};
            if (command === "list_codexpp_providers") return providers;
            if (command === "check_installations") return { codex: { installed: false, path: "" }, codexpp: { installed: false, path: "" } };
            if (command === "delete_codexpp_provider") { window.deleted.push(args.providerId); providers = []; }
            return null;
          },
        };
      }, platform);
      await page.goto(`http://127.0.0.1:${server.address().port}/`);
      await page.locator('[data-page="config"]').click();
      const remove = page.locator('[data-delete-provider="active"]');
      await remove.waitFor();
      await remove.click();
      const confirmation = page.locator("#app-confirm-dialog");
      await confirmation.waitFor();
      assert.match(await confirmation.innerText(), /正在使用/);
      await confirmation.locator("[data-confirm-cancel]").click();
      assert.deepEqual(await page.evaluate(() => window.deleted), []);
      assert.equal(await remove.count(), 1);
      await remove.click();
      await page.keyboard.press("Escape");
      await confirmation.waitFor({ state: "detached" });
      assert.deepEqual(await page.evaluate(() => window.deleted), []);
      await remove.click();
      await confirmation.locator("[data-confirm-accept]").click();
      await page.locator(".empty-provider-list").waitFor();
      assert.deepEqual(await page.evaluate(() => window.deleted), ["active"]);
      assert.equal(await remove.count(), 0);
      await page.locator('[data-page="install"]').click();
      await page.locator('[data-page="config"]').click();
      assert.equal(await remove.count(), 0);
      assert.deepEqual(errors, []);
      console.log(`PASS (${platform}): cancel/Escape keep active provider; confirm deletes it without native confirm, including after refresh.`);
      await page.close();
    }
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
