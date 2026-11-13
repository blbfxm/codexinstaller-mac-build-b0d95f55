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
    for (const error of ["SKILL_CATALOG_NETWORK", "HTTP status client error (404 Not Found) for url (https://example.test)"]) {
      const page = await browser.newPage({ viewport: { width: 1100, height: 800 } });
      const errors = [];
      page.on("pageerror", e => errors.push(e.message));
      await page.addInitScript(error => {
        window.skillCalls = 0;
        window.skillFailure = error;
        window.__TAURI_INTERNALS__ = {
          metadata: { currentWindow: { label: "main" } },
          transformCallback: () => 1,
          invoke: async command => {
            if (command === "load_auto_login_kami") return "test-card";
            if (command === "account_login") return { ok: true, account: { id: 1 }, installerConfig: {} };
            if (command === "device_info") return { platform: "windows", arch: "x64" };
            if (command === "fetch_site_config") return {};
            if (command === "list_codexpp_providers") return [];
            if (command === "check_installations") return {
              codex: { installed: true, path: "Codex.exe" },
              codexpp: { installed: true, path: "CodexPlusPlus.exe" },
            };
            if (command === "list_codex_skills") {
              window.skillCalls++;
              if (window.skillFailure) throw window.skillFailure;
              return [];
            }
            return null;
          },
        };
      }, error);
      await page.goto(`http://127.0.0.1:${server.address().port}/`);
      if (await page.locator("#library-submenu").isHidden()) await page.locator('[data-page="plugins"]').click();
    await page.locator('[data-library-tab="skills"]').click();
      const message = page.locator(".skill-load-error");
      await message.waitFor();
      assert.doesNotMatch(await message.innerText(), /HTTP|https:|SKILL_CATALOG/);
      assert.equal(await page.locator("#skills-refresh").isEnabled(), true);
      assert.equal(await page.locator(".skill-loading").count(), 0);
      await page.locator("#skill-sort").selectOption("name");
      assert.equal(await page.evaluate(() => window.skillCalls), 1);
      if (error === "SKILL_CATALOG_NETWORK") {
        assert.match(await message.innerText(), /选择节点并开启浏览器代理/);
        await page.locator("#skills-open-proxy").click();
        await page.locator("#proxy-agreement").waitFor({ state: "visible" });
        await page.locator("#proxy-agreement-cancel").click();
        await page.screenshot({ path: path.join(root, "skill-network-error-test.png") });
      } else {
        assert.match(await message.innerText(), /目录暂不可用/);
        assert.equal(await page.locator("#skills-open-proxy").count(), 0);
      }
      await page.evaluate(() => { window.skillFailure = ""; });
      await page.locator("#skills-retry").click();
      await message.waitFor({ state: "detached" });
      assert.equal(await page.evaluate(() => window.skillCalls), 2);
      assert.deepEqual(errors, []);
      await page.close();
    }
    console.log("PASS: skill network errors offer proxy setup; 404 has a distinct Chinese message; no retry loop; manual retry recovers.");
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
