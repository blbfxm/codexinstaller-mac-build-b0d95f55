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
    for (const scenario of ["default", "both", "installed", "installed-other-disk", "new", "new-retry", "missing-model", "empty-models", "model-error", "install-failure", "config-failure", "offline", "cancel-download"]) {
      const page = await browser.newPage({ viewport: { width: 1120, height: 900 } });
      const errors = [];
      page.on("pageerror", error => errors.push(error.message));
      await page.addInitScript(scenario => {
        window.calls = [];
        window.fail = true;
        const ready = { codex: ["installed", "installed-other-disk"].includes(scenario), codexpp: false };
        const nut = { id: "nut-existing", name: "坚果中转", baseUrl: "https://api.jianguo.ltd/v1", model: "test-model", models: ["test-model"], apiKey: "test-key", hasApiKey: true, isActive: false };
        if (scenario === "missing-model") { nut.model = ""; nut.models = []; }
        const other = { ...nut, id: "other", name: "其他供应商", baseUrl: "https://example.test/v1", isActive: true };
        let providers = ["new", "new-retry", "empty-models", "model-error"].includes(scenario) ? [other] : [other, nut];
        window.__TAURI_INTERNALS__ = {
          metadata: { currentWindow: { label: "main" } },
          transformCallback: () => 1,
          invoke: async (command, args) => {
            window.calls.push({ command, args });
            if (command === "load_auto_login_kami") return "test-login-card";
            if (command === "account_login") return { ok: true, account: { id: 1 }, installerConfig: {} };
            if (command === "fetch_site_config") return {};
            if (command === "device_info") return { platform: "windows", arch: "x64" };
            if (command === "codex_install_options") return { path: scenario === "installed-other-disk" ? "F:\\WindowsApps" : "", volumes: [{ path: "F:\\WindowsApps", label: "F:" }] };
            if (command === "product_process_running") return false;
            if (command === "codex_setup_state") return { onboardingComplete: true, fullAccessEnabled: true };
            if (command === "system_proxy_state") return { supported: false, enabled: false };
            if (command === "list_codexpp_providers") return providers;
            if (command === "check_codexpp_provider") {
              if (scenario === "offline") throw "供应商暂时无法连接";
              return ["test-model"];
            }
            if (command === "fetch_codexpp_models") {
              if (scenario === "offline") throw "供应商暂时无法连接";
              if (scenario === "empty-models") return { models: [], defaultModel: null };
              if (scenario === "model-error") throw "获取模型失败，请检查网络后重试";
              return { models: [" zeta-model ", "test-model", "alpha-model", "zeta-model", " "], defaultModel: "test-model" };
            }
            if (command === "check_installations") return Object.fromEntries(Object.entries(ready).map(([key, installed]) => [key, { installed, path: installed ? `${key}.exe` : "", version: "" }]));
            if (command === "install_codex_windows_fallback") {
              if (scenario === "install-failure") throw "测试下载失败";
              if (scenario === "cancel-download") throw "DOWNLOAD_CANCELLED";
              if (ready.codex && scenario !== "installed-other-disk") return "already-installed";
              ready.codex = true;
              return "installed";
            }
            if (command === "install_product") { ready[args.product] = true; return "installed"; }
            if (command === "configure_codexpp_provider" || command === "set_codexpp_active_provider") {
              if (["config-failure", "new-retry"].includes(scenario) && window.fail) throw "测试配置失败";
              providers = [other, { ...nut, isActive: true }];
              return "坚果供应商配置完成";
            }
            return null;
          },
        };
      }, scenario);
      await page.goto(`http://127.0.0.1:${server.address().port}/`);
      await page.waitForFunction(() => !document.querySelector("#install-all")?.disabled);
      await page.locator(".setup-options-disclosure > summary").click();
      assert.equal(await page.locator("#include-codexpp").isChecked(), false);
      await page.locator("#setup-enable-proxy").uncheck();
      if (scenario === "both") {
        await page.locator("#include-codexpp").check();
        await page.locator('[data-page="account"]').click();
        await page.locator('[data-page="install"]').click();
        assert.equal(await page.locator("#include-codexpp").isChecked(), true);
      }
      await page.locator("#install-all").click();
      if (["install-failure", "config-failure", "new-retry", "offline", "empty-models", "model-error"].includes(scenario)) {
        await page.locator(".setup-result.error").waitFor();
        if (["config-failure", "new-retry"].includes(scenario)) {
          await page.evaluate(() => { window.fail = false; });
          await page.locator("#install-all").click();
          await page.getByText("坚果供应商配置完成 Codex 已启动。", { exact: true }).waitFor();
        }
      } else if (scenario === "cancel-download") {
        await page.waitForFunction(() => document.querySelector("#progress-panel")?.hidden && !document.querySelector("#install-all")?.disabled);
      } else {
        await page.getByText("坚果供应商配置完成 Codex 已启动。", { exact: true }).waitFor();
      }
      assert.equal(await page.locator("#provider-model").count(), 0);
      assert.doesNotMatch(await page.locator("#toast").innerText(), /请确认坚果供应商/);
      const calls = await page.evaluate(() => window.calls);
      const installs = calls.filter(call => call.command === "install_codex_windows_fallback" || call.command === "install_product");
      const configs = calls.filter(call => call.command === "configure_codexpp_provider" || call.command === "set_codexpp_active_provider");
      assert.equal(installs.filter(call => call.command === "install_product").length, scenario === "both" ? 1 : 0);
      // Already installed Windows apps still reach the backend to check the
      // selected disk and unfinished restore; matching installs do no download.
      assert.equal(installs.filter(call => call.command === "install_codex_windows_fallback").length, ["offline", "empty-models", "model-error"].includes(scenario) ? 0 : ["config-failure", "new-retry"].includes(scenario) ? 2 : 1);
      assert.equal(configs.length, ["install-failure", "offline", "empty-models", "model-error", "cancel-download"].includes(scenario) ? 0 : ["config-failure", "new-retry"].includes(scenario) ? 2 : 1);
      for (const config of configs) {
        if (config.command === "configure_codexpp_provider") assert.equal(config.args.baseUrl, "https://api.jianguo.ltd/v1");
        assert.equal(config.args.providerId, ["new", "new-retry"].includes(scenario) ? null : "nut-existing");
        assert.equal(config.command, "configure_codexpp_provider");
      }
      if (scenario === "new") {
        assert.equal(configs[0].args.apiKey, "test-login-card");
        assert.equal(configs[0].args.configureCodexAccess, true);
      }
      if (!["new", "new-retry"].includes(scenario) && configs.length) {
        assert.equal(configs[0].args.configureCodexAccess, true);
        assert.equal(configs[0].args.configureCodexChinese, true);
        assert.equal(configs[0].args.configureCodexppChinese, true);
      }
      if (["new", "new-retry", "missing-model"].includes(scenario)) {
        for (const config of configs) {
          assert.equal(config.args.model, "test-model");
          assert.deepEqual(config.args.models, ["zeta-model", "test-model", "alpha-model"]);
        }
        const fetch = calls.find(call => call.command === "fetch_codexpp_models");
        assert.equal(fetch.args.apiKey, scenario === "missing-model" ? "test-key" : "test-login-card");
        assert.ok(calls.indexOf(fetch) < calls.indexOf(installs[0]));
      }
      if (configs.length && installs.length) {
        assert.ok(calls.indexOf(configs[0]) > calls.indexOf(installs[0]));
        assert.ok(calls.indexOf(configs.at(-1)) > calls.indexOf(installs.at(-1)));
      }
      assert.deepEqual(errors, []);
      if (scenario === "both") {
        await page.screenshot({ path: path.join(root, "one-click-setup-test.png"), fullPage: true });
        await page.setViewportSize({ width: 760, height: 620 });
        await page.screenshot({ path: path.join(root, "one-click-setup-compact-test.png"), fullPage: true });
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
        await page.locator("#theme-toggle").click();
        await page.screenshot({ path: path.join(root, "one-click-setup-light-test.png"), fullPage: true });
      }
      console.log(`PASS: ${scenario}`);
      await page.close();
    }
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
