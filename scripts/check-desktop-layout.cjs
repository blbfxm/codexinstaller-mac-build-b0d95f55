const { chromium } = require(process.env.PLAYWRIGHT_MODULE_PATH || "playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const http = require("node:http");

(async () => {
  const root = path.resolve(__dirname, "../dist");
  const output = path.resolve(__dirname, "../dist-portable/ui-check/redesign");
  fs.mkdirSync(output, { recursive: true });
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
    const page = await browser.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.addInitScript(() => {
      window.calls = [];
      window.testInstalled = { codex: true, codexpp: true };
      window.testProxyEnabled = false;
      window.testCoreRunning = false;
      window.testSelectedProxyProfile = "node-1";
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: "main" } },
        transformCallback: () => 1,
        invoke: async (command, args) => {
          window.calls.push({ command, args });
          if (command === "load_auto_login_kami") return "test-card";
          if (command === "account_login") return { ok: true, account: { id: 1, mainKamiMasked: "TEST****" }, installerConfig: {} };
          if (command === "fetch_site_config") return {};
          if (command === "device_info") return { platform: "windows", arch: "x64" };
          if (command === "check_installations") return Object.fromEntries(Object.entries(window.testInstalled).map(([key, installed]) => [key, { installed, path: installed ? `C:\\Apps\\${key}.exe` : "" }]));
          if (command === "product_process_running") return false;
          if (command === "list_codexpp_providers") return [
            { id: "nut", name: "坚果中转", baseUrl: "https://example.test/v1", model: "test-model", models: ["test-model", "test-model-2"], hasApiKey: true, apiKey: "test-key", isActive: true, isCurrent: true },
            { id: "other", name: "测试备用供应商", baseUrl: "https://example.test/v1", model: "another-model", models: ["another-model"], hasApiKey: true, apiKey: "test-key" },
          ];
          if (command === "check_codexpp_provider") return ["test-model"];
          if (command === "account_balance") return 125;
          if (command === "system_proxy_state") return { supported: true, enabled: window.testProxyEnabled };
          if (command === "set_system_proxy") { window.testProxyEnabled = args.enabled; return null; }
          if (command === "select_proxy_profile") { window.testSelectedProxyProfile = args.profileId; return null; }
          if (command === "installer_proxy_billing") return { allowed: true, balance: 100, bytesUsed: 0, billedUnits: 0, dailyPrice: 5, usageDate: "2026-11-08" };
          if (command === "launch_installed_product") return "launched";
          if (command === "list_codex_skills") return ["开发工具", "文档处理", "设计创作"].flatMap((category, group) =>
            [1, 2, 3].map(i => ({ name: `skill-${group}-${i}`, name_zh: `${category}技能 ${i}`, display_name: "Example skill", category, description_zh: "帮助完成日常工作中的相关任务，支持在 Codex 中安装使用。", author: "测试作者", tags: [], skills: i, bytes: 1024, installed: false })));
          if (command === "v2rayn_api") {
            let data = [];
            if (args.path === "/api/status") data = { coreRunning: window.testCoreRunning, listeners: window.testCoreRunning ? [{ protocols: ["http"], listenAddress: "127.0.0.1", port: 10809 }] : [] };
            if (args.path === "/api/subscriptions") data = [{ id: "test", remarks: "自定义订阅", url: "https://example.test/sub", enabled: true }];
            if (args.path.startsWith("/api/profiles")) data = [1, 2, 3, 4].map(i => ({ indexId: `node-${i}`, remarks: i === 2 ? "US Los Angeles" : `测试节点 ${i}`, address: "example.test", port: 443, protocol: "vless", delay: i * 30, isCurrent: `node-${i}` === window.testSelectedProxyProfile, subscriptionId: "test" }));
            if (args.path === "/api/settings/routing-profiles") data = [{ id: "global", remarks: "global", isActive: true }];
            if (args.path === "/api/core/start") window.testCoreRunning = true;
            if (args.path === "/api/core/stop") window.testCoreRunning = false;
            return { data };
          }
          return null;
        },
      };
    });
    await page.setViewportSize({ width: 1120, height: 820 });
    await page.goto(`http://127.0.0.1:${server.address().port}/`);
    await page.locator('[data-launch-product="codex"]').waitFor();
    for (const theme of ["dark", "light"]) {
      await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
      for (const size of [{ width: 1120, height: 820 }, { width: 760, height: 620 }, { width: 1440, height: 900 }, { width: 390, height: 820 }]) {
        await page.setViewportSize(size);
        for (const view of ["install", "config", "proxy", "plugins", "account"]) {
          if (view === "plugins") {
            if (await page.locator("#library-submenu").isHidden()) await page.locator('[data-page="plugins"]').click();
            await page.locator('[data-library-tab="skills"]').click();
          } else await page.locator(`[data-page="${view}"]`).click();
          if (await page.locator("#proxy-agreement-accept").isVisible()) await page.locator("#proxy-agreement-accept").click();
          await page.locator(`#page-content[data-view="${view}"]`).waitFor();
          if (view === "plugins") { await page.locator('[data-library-tab="skills"]').click(); await page.locator(".skill-item").first().waitFor(); }
          if (view === "config") await page.locator("#provider-new").waitFor();
          const overflow = await page.evaluate(() => {
            const main = document.querySelector(".content");
            return document.documentElement.scrollWidth > innerWidth || main.scrollWidth > main.clientWidth + 1;
          });
          assert.equal(overflow, false, `${theme} ${size.width} ${view} overflows horizontally`);
          assert.equal(await page.locator(".sidebar").evaluate(el => el.getBoundingClientRect().bottom <= innerHeight + 1), true);
          await page.screenshot({ path: path.join(output, `${theme}-${size.width}-${view}.png`) });
          if (view === "proxy") {
            await page.locator('[data-proxy-view="subscriptions"]').click();
            assert.equal(await page.locator("#subscription-url").isVisible(), true);
            assert.equal(await page.locator(".node-panel").isVisible(), false);
            await page.locator('[data-proxy-view="subscriptions"]').press("ArrowRight");
            assert.equal(await page.locator("#system-proxy-mode").isVisible(), true);
            await page.locator('[data-proxy-view="nodes"]').click();
          }
          if (view === "config") {
            await page.locator('[data-edit-provider="nut"]').click();
            assert.equal(await page.locator("#provider-model").inputValue(), "test-model");
            assert.equal(await page.locator(".provider-options").count(), 0);
            await page.screenshot({ path: path.join(output, `${theme}-${size.width}-editor.png`) });
            assert.equal(await page.locator(".content").evaluate(el => el.scrollWidth > el.clientWidth + 1), false);
            await page.locator("#provider-key").scrollIntoViewIfNeeded();
            assert.equal(await page.locator("#provider-key").isVisible(), true);
            await page.locator("#provider-cancel").click();
          }
        }
      }
    }
    await page.setViewportSize({ width: 1120, height: 820 });
    await page.locator('[data-page="install"]').click();
    await page.locator(".setup-options-disclosure > summary").click();
    await page.evaluate(() => { window.testInstalled.codex = false; });
    await page.locator("#refresh").click();
    assert.equal(await page.locator('[data-launch-product="codex"]').isVisible(), true);
    assert.equal(await page.locator('[data-launch-product="codex"]').isDisabled(), true);
    await page.evaluate(() => { window.testInstalled.codex = true; });
    await page.locator("#refresh").click();
    assert.equal(await page.locator("#option-codex-access").isVisible(), true);
    assert.equal(await page.locator("#option-codex-chinese").isVisible(), true);
    assert.equal(await page.locator("#option-codexpp-chinese").isVisible(), true);
    assert.equal(await page.locator("#option-codexpp-start-codex").isVisible(), true);
    assert.equal(await page.locator("#setup-full-launch").isChecked(), false);
    await page.locator('[data-launch-product="codex"]').click();
    assert.equal(await page.evaluate(() => window.calls.some(call => call.command === "launch_installed_product" && call.args.product === "codex")), true);
    await page.locator("#setup-full-launch").check();
    const fullLaunchStart = await page.evaluate(() => window.calls.length);
    await page.locator('[data-launch-product="codex"]').click();
    await page.waitForFunction(() => window.testProxyEnabled && window.calls.some(call => call.command === "launch_installed_product"));
    const fullLaunchCalls = await page.evaluate(start => window.calls.slice(start), fullLaunchStart);
    const selectIndex = fullLaunchCalls.findIndex(call => call.command === "select_proxy_profile");
    const proxyIndex = fullLaunchCalls.findIndex(call => call.command === "set_system_proxy" && call.args.enabled);
    const launchIndex = fullLaunchCalls.findIndex(call => call.command === "launch_installed_product" && call.args.product === "codex");
    assert.ok(selectIndex >= 0, "Full launch selects a US node");
    assert.ok(proxyIndex > selectIndex, "System proxy turns on after node selection");
    assert.ok(launchIndex > proxyIndex, "Codex starts after the proxy is enabled");
    assert.deepEqual(errors, []);
    console.log("PASS: all five pages, proxy tabs, provider editor, launch action, both themes at 390/760/1120/1440px");
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
