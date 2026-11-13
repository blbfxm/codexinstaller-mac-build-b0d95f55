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
    for (const scenario of ["defaults", "agreement-cancel", "proxy-failure", "empty-nodes", "install-only", "pp-only", "config-only", "config-missing-codex", "proxy-only", "proxy-setup", "none"]) {
      const page = await browser.newPage({ viewport: { width: 1120, height: 820 } });
      const errors = [];
      page.on("pageerror", error => errors.push(error.message));
      await page.addInitScript(scenario => {
        window.calls = [];
        const ready = { codex: scenario === "config-only", codexpp: false };
        let coreRunning = false, enabled = false, selected = false;
        window.__TAURI_INTERNALS__ = {
          metadata: { currentWindow: { label: "main" } },
          transformCallback: () => 1,
          invoke: async (command, args) => {
            window.calls.push({ command, args });
            if (command === "load_auto_login_kami") return "test-card";
            if (command === "account_login") return { ok: true, account: { id: 1 }, installerConfig: {} };
            if (command === "device_info") return { platform: "windows", arch: "x64" };
            if (command === "fetch_site_config") return {};
            if (command === "list_codexpp_providers") return [];
            if (command === "check_installations") return Object.fromEntries(Object.entries(ready).map(([key, installed]) => [key, { installed, path: installed ? `${key}.exe` : "" }]));
            if (command === "product_process_running") return false;
            if (command === "fetch_codexpp_models") return { models: ["test-model"], defaultModel: "test-model" };
            if (command === "install_codex_windows_fallback") { ready.codex = true; return "installed"; }
            if (command === "install_product") { ready.codexpp = true; return "installed"; }
            if (command === "configure_codexpp_provider") return "坚果供应商配置完成";
            if (command === "system_proxy_state") return { supported: true, enabled };
            if (command === "set_system_proxy") { enabled = args.enabled; return null; }
            if (command === "select_proxy_profile") { selected = true; return null; }
            if (command === "check_nut_subscription_access") return true;
            if (command === "v2rayn_api") {
              if (scenario === "proxy-failure") throw "测试代理服务无法启动";
              let data = [];
              if (args.path === "/api/status") data = { coreRunning, listeners: coreRunning ? [{ protocols: ["http"], listenAddress: "127.0.0.1", port: 10808 }] : [] };
              if (args.path.startsWith("/api/profiles")) data = scenario === "empty-nodes" ? [] : [{ indexId: "node-test", remarks: "Test Node", address: "example.test", port: 443, protocol: "vless", delay: 30, isCurrent: selected }];
              if (args.path === "/api/subscriptions") data = [{ id: "subscription", remarks: "Test", url: "https://example.test/sub", enabled: true }];
              if (args.path === "/api/core/start") coreRunning = true;
              if (args.path === "/api/core/stop") coreRunning = false;
              return { data };
            }
            return null;
          },
        };
      }, scenario);
      await page.goto(`http://127.0.0.1:${server.address().port}/`);
      await page.waitForFunction(() => !document.querySelector("#install-all")?.disabled);
      await page.locator(".setup-options-disclosure > summary").click();
      for (const [id, checked] of [["include-codex", true], ["setup-enable-proxy", false], ["setup-configure-nut", true], ["include-codexpp", false]]) {
        assert.equal(await page.locator(`#${id}`).isChecked(), checked);
      }
      assert.equal(await page.locator("#setup-full-launch").isChecked(), false);
      assert.match(await page.locator("#full-launch-tip").innerText(), /每天 5 点.*每日流量上限为 30G/);
      const proxy = ["agreement-cancel", "proxy-failure", "empty-nodes", "proxy-only", "proxy-setup"].includes(scenario);
      const codex = ["defaults", "agreement-cancel", "proxy-failure", "empty-nodes", "install-only", "proxy-setup"].includes(scenario);
      const config = ["defaults", "agreement-cancel", "proxy-failure", "empty-nodes", "config-only", "config-missing-codex", "proxy-setup"].includes(scenario);
      await page.locator("#include-codex").setChecked(codex);
      await page.locator("#setup-enable-proxy").setChecked(proxy);
      await page.locator("#setup-configure-nut").setChecked(config);
      await page.locator("#include-codexpp").setChecked(scenario === "pp-only");
      await page.locator('[data-page="account"]').click();
      await page.locator('[data-page="install"]').click();
      await page.locator(".setup-options-disclosure > summary").click();
      assert.equal(await page.locator("#setup-enable-proxy").isChecked(), proxy);
      assert.equal(await page.locator("#include-codex").isChecked(), codex);
      if (scenario === "none") assert.equal(await page.locator("#install-all").isDisabled(), true);
      else {
        await page.locator("#install-all").click();
        if (proxy) {
          await page.locator("#proxy-agreement-accept").waitFor();
          const before = await page.evaluate(() => window.calls);
          assert.equal(before.some(call => ["install_product", "install_codex_windows_fallback", "v2rayn_api", "configure_codexpp_provider"].includes(call.command)), false);
          if (scenario === "agreement-cancel") {
            await page.locator("#proxy-agreement-cancel").click();
            assert.equal(await page.locator("#install-all").isEnabled(), true);
          } else await page.locator("#proxy-agreement-accept").click();
        }
        if (["proxy-failure", "empty-nodes", "config-missing-codex"].includes(scenario)) await page.locator(".setup-result.error").waitFor();
        else if (scenario !== "agreement-cancel") await page.waitForFunction(() => document.querySelector(".setup-result")?.textContent.includes("已完成") || document.querySelector(".setup-result")?.textContent.includes("配置完成"));
      }
      const calls = await page.evaluate(() => window.calls);
      const blocked = ["none", "agreement-cancel", "proxy-failure", "empty-nodes", "config-missing-codex"].includes(scenario);
      assert.equal(calls.filter(call => call.command === "install_codex_windows_fallback").length, codex && !blocked ? 1 : 0);
      assert.equal(calls.filter(call => call.command === "install_product").length, scenario === "pp-only" ? 1 : 0);
      assert.equal(calls.filter(call => call.command === "configure_codexpp_provider").length, config && !blocked ? 1 : 0);
      assert.equal(calls.filter(call => call.command === "set_system_proxy" && call.args.enabled).length, ["proxy-only", "proxy-setup"].includes(scenario) ? 1 : 0);
      assert.equal(calls.filter(call => call.command === "installer_proxy_billing").length, 0);
      if (["proxy-only", "proxy-setup"].includes(scenario)) assert.equal(calls.filter(call => call.command === "set_system_proxy" && !call.args.enabled).length, 1);
      for (const config of calls.filter(call => call.command === "configure_codexpp_provider")) {
        assert.equal(config.args.configureCodexChinese, true);
      }
      if (scenario === "defaults") {
        await page.screenshot({ path: path.join(root, "setup-options-dark.png"), fullPage: true });
        await page.setViewportSize({ width: 760, height: 620 });
        await page.screenshot({ path: path.join(root, "setup-options-compact.png"), fullPage: true });
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
        await page.locator("#theme-toggle").click();
        await page.screenshot({ path: path.join(root, "setup-options-light.png"), fullPage: true });
      }
      assert.deepEqual(errors, []);
      console.log(`PASS: ${scenario}`);
      await page.close();
    }
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
