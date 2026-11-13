const { chromium } = require(process.env.PLAYWRIGHT_MODULE_PATH || "playwright");
const assert = require("node:assert/strict");
const path = require("node:path");
const fs = require("node:fs");

(async () => {
  const output = path.resolve(__dirname, "../dist-portable/ui-check");
  fs.mkdirSync(output, { recursive: true });
  const browser = await chromium.launch({ headless: true, channel: "msedge" });
  try {
    const page = await browser.newPage({ viewport: { width: 1120, height: 820 } });
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.addInitScript(() => {
      window.testInstalled = { codex: false, codexpp: false };
      window.testSavedKami = "ui-test";
      window.testProviders = [
        { id: "nut", name: "坚果", baseUrl: "https://api.jianguo.ltd/v1", model: "test-model", models: ["test-model", "secondary-model"], apiKey: "", hasApiKey: true },
      ];
      window.testCalls = [];
      window.testInvokeArgs = {};
      window.testProxyEnabled = false;
      window.testCoreRunning = false;
      window.testNodeSelected = true;
      window.testNodeDelay = 38;
      window.testProxyProfiles = [
        { indexId: "one", remarks: "测试节点 · 香港", protocol: "vmess", address: "example.test", port: 443, delay: window.testNodeDelay, isCurrent: window.testNodeSelected },
        { indexId: "traffic", remarks: "剩余流量：970 GB", protocol: "vless", address: "example.test", port: 41000, delay: 0, isCurrent: false },
        { indexId: "promo", remarks: "TG群：https://t.me/example", protocol: "vless", address: "example.test", port: 41000, delay: 0, isCurrent: false },
      ];
      window.testLatencyResult = 126;
      window.testLatencyPolls = 0;
      window.testApiCalls = [];
      window.testCallbacks = {};
      window.__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: "main" } },
        transformCallback: callback => {
          const id = Object.keys(window.testCallbacks).length + 1;
          window.testCallbacks[id] = callback;
          return id;
        },
        invoke: async (command, args) => {
          window.testCalls.push(command);
          window.testInvokeArgs[command] = args;
          if (command === "plugin:event|listen") { window.testTrayCallback = args.handler; return 1; }
          if (command === "load_auto_login_kami") return window.testSavedKami;
          if (command === "save_auto_login_kami") { window.testSavedKami = args.kami; return null; }
          if (command === "clear_auto_login_kami") { window.testSavedKami = null; return null; }
          if (command === "account_balance") return 12.5;
          if (command === "account_usage") return {
            total: 2,
            items: [
              { time: "2026-10-28 10:00:00", type: "deduct", model: "gpt-test", points: 5, status: "success", detail: "测试扣点" },
              { time: "2026-10-28 09:00:00", type: "recharge", points: 10, status: "success" },
            ],
          };
          if (command === "account_redeem_kami") return 27.5;
          if (command === "open_account_center") { window.testAccountCenterTab = args.tab; return null; }
          if (command === "check_nut_subscription_access") return true;
          if (command === "set_system_proxy") { window.testProxyEnabled = args.enabled; return {}; }
          if (command === "clear_system_proxy") { window.testProxyEnabled = false; return {}; }
          if (command === "open_external_url") { window.testExternalUrl = args.url; return null; }
          if (command === "account_login") return {
            ok: true, account: { id: 1, mainKamiMasked: "TEST****" },
            installerConfig: { v2rayn_subscription_url: "https://example.test/sub" },
          };
          if (command === "fetch_site_config") return {};
          if (command === "device_info") return { platform: "windows", arch: "x64" };
          if (command === "check_installations") return Object.fromEntries(
            Object.entries(window.testInstalled).map(([key, installed]) =>
              [key, { installed, path: installed ? `C:\\Programs\\${key}.exe` : "", version: "" }]));
          if (command === "install_product") {
            if (window.testFailInstall) throw new Error("下载失败，请检查网络后重试");
            window.testInstalled[args.product] = true;
            return "installed";
          }
          if (command === "install_codex_windows_fallback") {
            if (window.testFailInstall) throw new Error("备用下载也失败");
            window.testInstalled.codex = true;
            return "installed";
          }
          if (command === "list_codexpp_providers") return window.testProviders;
          if (command === "fetch_codexpp_models") return { models: ["zeta-model", "alpha-model", "middle-model"], defaultModel: "alpha-model" };
          if (command === "system_proxy_state") return { supported: true, enabled: window.testProxyEnabled };
          if (command === "v2rayn_api") {
            window.testApiCalls.push({ method: args.method, path: args.path });
            if (args.path === "/api/profiles/one/latency") {
              if (window.testLatencyError) throw new Error("测速服务繁忙，请稍后重试");
              window.testLatencyPolls = 3;
            }
            if (args.path === "/api/operations") {
              const pending = window.testLatencyPolls > 0;
              if (pending) window.testLatencyPolls--;
              else window.testNodeDelay = window.testLatencyResult;
              return { data: { data: pending ? ["speedtest"] : [] } };
            }
            if (args.path === "/api/core/start") window.testCoreRunning = true;
            if (args.path === "/api/core/stop") window.testCoreRunning = false;
            const data = {
              "/api/status": { coreRunning: window.testCoreRunning, listeners: window.testCoreRunning ? [{ protocols: ["http"], listenAddress: "127.0.0.1", port: 10809 }] : [] },
              "/api/subscriptions": [{ id: "nut", remarks: "坚果订阅", url: "https://example.test/sub", enabled: true, updateTime: 1 }],
              "/api/settings/routing-profiles": [{ id: "global", remarks: "global", isActive: true }],
              "/api/profiles?subscriptionId=nut": window.testProxyProfiles.map(profile => ({ ...profile, delay: profile.indexId === "one" ? window.testNodeDelay : profile.delay, isCurrent: profile.indexId === "one" && window.testNodeSelected })),
            };
            return { data: { data: data[args.path] || [] } };
          }
          return null;
        },
      };
    });
    await page.goto("http://127.0.0.1:1420/");
    await page.locator("#install-all").waitFor();
    assert.equal(await page.locator(".product-row").first().locator(".install-one").innerText(), "安装");
    await page.locator(".product-row").first().locator(".more-button").click();
    assert.equal(await page.locator(".product-row").first().locator('[data-product-action="store-install"]').innerText(), "微软商店安装");
    await page.locator(".product-row").first().locator(".more-button").click();
    assert.equal(await page.locator(".product-main").filter({ hasText: "可选安装" }).count(), 1);
    assert.equal(await page.locator(".account-name").innerText(), "TEST****", "Stored kami should automatically sign in");
    await page.evaluate(() => window.testCallbacks[window.testTrayCallback]({ payload: true }));
    await page.locator("#proxy-agreement-accept").waitFor({ state: "visible" });
    assert.equal(await page.evaluate(() => window.testProxyEnabled), false);
    await page.locator("#proxy-agreement-cancel").click();
    await page.screenshot({ path: path.join(output, "install-desktop.png"), fullPage: true });
    await page.locator(".setup-options-disclosure > summary").click();
    await page.locator("#setup-enable-proxy").uncheck();
    await page.locator("#setup-full-launch").uncheck();
    await page.locator("#install-all").click();
    await page.getByText("Codex 和 Codex++ 已安装。下一步：配置供应商。", { exact: true }).waitFor();
    assert.equal(await page.locator("#progress-panel").isVisible(), false, "Completed installation should not leave a stale download panel visible");
    assert.equal(await page.locator("#install-all").isDisabled(), true);
    assert.equal(await page.evaluate(() => window.testCalls.filter(call => call === "install_product").length), 1);
    await page.locator("#setup-configure").click();
    await page.locator("#provider-new").waitFor();
    assert.equal(await page.locator(".page-heading h1").innerText(), "供应商配置");
    assert.equal(await page.locator(".nut-service-details").getAttribute("open"), "");
    assert.equal(await page.locator(".provider-item").innerText().then(text => text.includes("未设置模型")), false);
    await page.locator("#nut-service-open").click();
    assert.equal(await page.evaluate(() => window.testExternalUrl), "https://api.jianguo.ltd/llm.html");
    await page.locator("#nut-service-configure").click();
    assert.equal(await page.locator("#provider-model").inputValue(), "test-model");
    assert.equal(await page.locator("#open-key-purchase").count(), 0);
    await page.locator("#fetch-models").click();
    await page.getByText("已获取 3 个模型；已按上游默认值选择 alpha-model。", { exact: true }).waitFor();
    assert.deepEqual(await page.locator("[data-select-model]").evaluateAll(items => items.map(item => item.getAttribute("data-select-model"))), ["zeta-model", "alpha-model", "middle-model"]);
    assert.equal(await page.locator("#provider-model").inputValue(), "alpha-model");
    await page.locator("#provider-cancel").click();
    await page.screenshot({ path: path.join(output, "providers-desktop.png"), fullPage: true });
    await page.setViewportSize({ width: 390, height: 820 });
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, "Service recommendation overflows");
    await page.screenshot({ path: path.join(output, "providers-mobile.png"), fullPage: true });
    await page.setViewportSize({ width: 1120, height: 820 });
    await page.locator(".provider-actions-menu .more-button").click();
    await page.locator("[data-edit-provider]").click();
    assert.equal(await page.locator("#provider-model").inputValue(), "test-model");
    await page.locator("#provider-cancel").click();
    await page.locator("#provider-new").click();
    await page.locator("#provider-name").waitFor();
    assert.equal(await page.locator("#provider-name").inputValue(), "坚果");
    assert.equal(await page.locator(".provider-options").count(), 0, "Provider-specific settings belong to installation options");
    await page.locator("#provider-key").fill("test-api-key");
    await page.locator("#provider-model").fill("test-model");
    await page.locator("#provider-cancel").click();
    await page.locator('[data-page="install"]').click();
    await page.locator(".setup-options-disclosure > summary").click();
    await page.locator("#option-codex-access").uncheck();
    await page.locator("#option-codexpp-chinese").uncheck();
    await page.locator('[data-page="config"]').click();
    await page.locator("#provider-new").click();
    await page.locator("#provider-custom").click();
    await page.locator("#provider-key").fill("test-api-key");
    await page.locator("#provider-model").fill("test-model");
    await page.locator("#configure-provider").click();
    await page.waitForFunction(() => window.testCalls.includes("configure_codexpp_provider"));
    assert.equal(await page.evaluate(() => window.testInvokeArgs.configure_codexpp_provider.configureCodexAccess), false);
    assert.equal(await page.evaluate(() => window.testInvokeArgs.configure_codexpp_provider.configureCodexChinese), true);
    assert.equal(await page.evaluate(() => window.testInvokeArgs.configure_codexpp_provider.configureCodexppChinese), false);
    await page.screenshot({ path: path.join(output, "configure-desktop.png"), fullPage: true });
    await page.locator('[data-page="account"]').click();
    await page.locator(".account-profile-list").waitFor();
    await page.getByText("12.5 点", { exact: true }).waitFor();
    assert.equal(await page.locator("#account-change-form").count(), 0, "Personal info should not expose login-card replacement");
    assert.equal(await page.locator(".account-profile-list .account-balance").count(), 1, "Balance belongs in personal info");
    const kamiDisplay = page.locator(".account-login-key > span");
    assert.notEqual(await kamiDisplay.innerText(), "ui-test", "Login kami must be masked by default");
    await page.locator("#toggle-login-kami").click();
    assert.equal(await kamiDisplay.innerText(), "ui-test");
    await page.locator("#toggle-login-kami").click();
    assert.notEqual(await kamiDisplay.innerText(), "ui-test", "Login kami can be hidden again");
    await page.locator("#recharge-kami").fill("top-up-card");
    await page.locator("#account-recharge-form button[type=submit]").click();
    await page.waitForFunction(() => window.testCalls.includes("account_redeem_kami"));
    assert.equal(await page.evaluate(() => window.testSavedKami), "ui-test", "Recharge must not replace the login kami");
    await page.locator("#online-recharge").click();
    assert.equal(await page.evaluate(() => window.testAccountCenterTab), "wallet");
    assert.equal(await page.evaluate(() => window.testSavedKami), "ui-test", "Personal center should preserve current login credentials");
    await page.screenshot({ path: path.join(output, "account-desktop.png"), fullPage: true });
    await page.reload();
    await page.locator("#install-all").waitFor();
    await page.locator('[data-page="config"]').click();
    await page.locator("#provider-new").waitFor();
    await page.evaluate(() => { window.testProviders = []; });
    await page.locator("#nut-service-configure").click();
    assert.equal(await page.locator("#provider-key").inputValue(), "new-login-card", "Nut API Key should default to the current login kami when no Nut provider exists");
    await page.locator("#provider-cancel").click();
    await page.locator('[data-page="proxy"]').click();
    await page.locator("#proxy-agreement-accept").click();
    await page.getByText("测试节点 · 香港", { exact: true }).first().waitFor();
    assert.equal(await page.locator(".node-row").count(), 1, "Subscription traffic and promotion metadata should not appear as nodes");
    assert.equal(await page.locator(".proxy-advanced").getAttribute("open"), null);
    await page.screenshot({ path: path.join(output, "proxy-desktop.png"), fullPage: true });
    await page.locator("[data-profile-test]").click();
    await page.getByText("测速中…", { exact: true }).waitFor();
    assert.equal(await page.locator("[data-profile-test]").isDisabled(), true);
    await page.waitForTimeout(2100);
    assert.equal(await page.locator("[data-profile-test]").isDisabled(), true, "Slow tests must remain busy until completion");
    await page.getByText("126 ms", { exact: true }).waitFor();
    assert.equal(await page.locator("[data-profile-test]").isEnabled(), true);
    await page.evaluate(() => { window.testLatencyResult = -1; });
    await page.locator("[data-profile-test]").click();
    await page.locator(".node-delay").getByText("连接失败", { exact: true }).waitFor();
    await page.locator("#proxy-feedback").getByText("测试节点 · 香港 连接失败，请检查网络或更换节点", { exact: true }).waitFor();
    await page.evaluate(() => { window.testLatencyError = true; });
    await page.locator("[data-profile-test]").click();
    await page.locator("#proxy-feedback").getByText("测速服务繁忙，请稍后重试", { exact: true }).waitFor();
    assert.equal(await page.locator("[data-profile-test]").isEnabled(), true);
    await page.evaluate(() => { window.testLatencyError = false; window.testNodeDelay = 126; window.testNodeSelected = false; });
    await page.locator('[data-page="install"]').click();
    const noNodeCalls = await page.evaluate(() => ({
      proxy: window.testCalls.filter(call => call === "set_system_proxy").length,
      select: window.testCalls.filter(call => call === "select_proxy_profile").length,
      core: window.testApiCalls.filter(call => call.path === "/api/core/start").length,
    }));
    await page.evaluate(() => window.testCallbacks[window.testTrayCallback]({ payload: true }));
    await page.locator("#proxy-feedback").getByText("请先在节点列表中选择一个节点，再开启网络代理。", { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => window.testProxyEnabled), false);
    assert.equal(await page.evaluate(() => window.testCoreRunning), false);
    assert.deepEqual(await page.evaluate(() => ({
      proxy: window.testCalls.filter(call => call === "set_system_proxy").length,
      select: window.testCalls.filter(call => call === "select_proxy_profile").length,
      core: window.testApiCalls.filter(call => call.path === "/api/core/start").length,
    })), noNodeCalls, "Tray must not start a core, set a proxy or auto-select a node");
    for (const command of ["plugin:window|show", "plugin:window|unminimize", "plugin:window|set_focus"]) {
      assert.equal(await page.evaluate(command => window.testCalls.includes(command), command), true);
    }
    await page.evaluate(() => { window.testNodeSelected = true; });
    await page.locator("#proxy-refresh").click();
    await page.waitForFunction(() => !document.querySelector("#proxy-refresh").disabled);
    await page.locator(".proxy-advanced summary").click();
    assert.equal(await page.locator("#subscription-new").isVisible(), true);
    assert.equal(await page.locator("#system-proxy-mode").isVisible(), true);
    await page.locator("#system-proxy-mode").selectOption("clear");
    await page.getByText("系统代理已清除，浏览器不再使用系统代理。", { exact: true }).waitFor();
    assert.equal(await page.evaluate(() => window.testProxyEnabled), false);
    await page.locator(".proxy-advanced summary").click();
    await page.locator("#system-proxy-mode").selectOption("auto");
    await page.waitForFunction(() => window.testProxyEnabled);
    await page.waitForFunction(() => document.querySelector("#system-proxy-toggle")?.textContent === "关闭浏览器代理" && !document.querySelector("#system-proxy-toggle").disabled);
    await page.locator(".proxy-advanced summary").click();
    const proxyWrites = await page.evaluate(() => window.testCalls.filter(call => call === "set_system_proxy").length);
    await page.locator("#system-proxy-mode").selectOption("unchanged");
    assert.equal(await page.evaluate(() => window.testProxyEnabled), true);
    assert.equal(await page.evaluate(() => window.testCalls.filter(call => call === "set_system_proxy").length), proxyWrites);
    await page.evaluate(() => window.testCallbacks[window.testTrayCallback]({ payload: false }));
    await page.waitForFunction(() => !window.testProxyEnabled && document.querySelector("#system-proxy-toggle")?.textContent === "开启浏览器代理" && !document.querySelector("#system-proxy-toggle").disabled);
    await page.evaluate(() => window.testCallbacks[window.testTrayCallback]({ payload: true }));
    await page.waitForFunction(() => window.testProxyEnabled && document.querySelector("#system-proxy-toggle")?.textContent === "关闭浏览器代理" && !document.querySelector("#system-proxy-toggle").disabled);
    await page.locator(".proxy-advanced summary").click();
    await page.locator("#subscription-new").click();
    await page.locator(".proxy-advanced summary").click();
    assert.equal(await page.locator("#subscription-url").inputValue(), "");
    for (const width of [760, 390]) {
      await page.setViewportSize({ width, height: 820 });
      for (const view of ["install", "config", "proxy", "account"]) {
        await page.locator(`[data-page="${view}"]`).click();
        await page.waitForTimeout(200);
        const overflow = await page.evaluate(() => document.documentElement.scrollWidth > innerWidth);
        assert.equal(overflow, false, `${view} overflows at ${width}px`);
        await page.screenshot({ path: path.join(output, `${view}-${width}.png`), fullPage: true });
      }
    }
    await page.setViewportSize({ width: 1120, height: 820 });
    await page.evaluate(() => { window.testInstalled = { codex: true, codexpp: false }; window.testFailInstall = true; });
    await page.locator('[data-page="install"]').click();
    await page.locator("#refresh").click();
    await page.waitForTimeout(200);
    await page.locator("#install-all").click();
    await page.locator(".setup-result.error").waitFor();
    assert.match(await page.locator(".setup-result").innerText(), /Codex\+\+.*下载失败/);
    assert.deepEqual(errors, []);
    console.log("PASS: secure auto-login restore, account balance/recharge/change-kami/usage UI, delayed latency results, tray node selection, installation, providers, immediate proxy modes, consent gating, responsive layout and failure recovery; no page errors.");
    console.log(`Screenshots: ${output}`);
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exit(1); });
