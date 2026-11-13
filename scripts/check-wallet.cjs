const { chromium } = require(process.env.PLAYWRIGHT_MODULE_PATH || "playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const http = require("node:http");

(async () => {
  const root = path.resolve(__dirname, "../dist");
  const output = path.resolve(__dirname, "../dist-portable/ui-check");
  fs.mkdirSync(output, { recursive: true });
  const server = http.createServer((req, res) => {
    const name = new URL(req.url, "http://localhost").pathname;
    const file = path.resolve(root, name === "/" ? "index.html" : `.${name}`);
    if (!file.startsWith(root + path.sep) || !fs.existsSync(file)) return res.writeHead(404).end();
    res.setHeader("Content-Type", { ".html": "text/html", ".js": "text/javascript", ".css": "text/css" }[path.extname(file)] || "application/octet-stream");
    fs.createReadStream(file).pipe(res);
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  let browser;
  try {
    browser = await chromium.launch({ headless: true, channel: "msedge" });
    for (const scenario of ["package", "package-no-100", "custom", "catalog-error", "create-error", "poll-error", "unsafe", "logout", "card-login-zero"]) {
      const page = await browser.newPage({ viewport: { width: 1120, height: 820 } });
      const errors = [];
      page.on("pageerror", error => errors.push(error.message));
      await page.route("https://**/*", route => route.fulfill({
        contentType: "text/html",
        body: "<html><body style='margin:0;background:white'>支付宝官方付款码（测试）</body></html>",
      }));
      await page.addInitScript(scenario => {
        window.calls = [];
        window.paid = false;
        window.failCatalog = scenario === "catalog-error";
        window.__TAURI_INTERNALS__ = {
          metadata: { currentWindow: { label: "main" } },
          transformCallback: () => 1,
          invoke: async (command, args) => {
            window.calls.push({ command, args });
            if (command === "load_auto_login_kami") return "wallet-test-card";
            if (command === "account_login") return { ok: true, account: { id: scenario === "card-login-zero" ? 0 : 1 }, installerConfig: {} };
            if (command === "fetch_site_config") return {};
            if (command === "device_info") return { platform: "windows", arch: "x64" };
            if (command === "list_codexpp_providers") return [];
            if (command === "check_installations") return { codex: { installed: false }, codexpp: { installed: false } };
            if (command === "system_proxy_state") return { supported: true, enabled: false };
            if (command === "account_balance") return 12.5;
            if (command === "account_redeem_kami") return 100;
            if (command === "wallet_catalog") {
              if (window.failCatalog) throw "充值服务暂不可用";
              const amounts = scenario === "package-no-100" ? [20, 30, 50] : [20, 30, 50, 100, 200, 500, 1000];
              return { paymentReady: true, packages: amounts.map(amount => ({ amount, points: amount * 10 })) };
            }
            if (command === "wallet_create_order") {
              if (scenario === "create-error") throw "无法连接充值服务";
              const order = { orderNo: "TEST-ORDER", amount: args.custom ? args.value : args.value / 10, points: args.custom ? args.value * 10 : args.value,
                status: "pending", paymentInfo: "https://openapi.alipay.com/gateway.do?test=1", paymentType: "cashier" };
              if (scenario === "unsafe") order.paymentInfo = "https://evil.test/steal";
              if (scenario === "logout") return new Promise(resolve => { window.resolveOrder = () => resolve(order); });
              return order;
            }
            if (command === "wallet_order_status") {
              if (scenario === "poll-error" && !window.paid) throw "暂时无法确认到账";
              return { orderNo: args.orderNo, amount: 30, points: 300, status: window.paid ? "paid" : "pending", balance: 312.5 };
            }
            return null;
          },
        };
      }, scenario);
      await page.goto(`http://127.0.0.1:${server.address().port}/`);
      await page.locator('[data-page="account"]').click();
      assert.equal(await page.locator("#account-info-panel").isVisible(), true);
      await page.locator(".account-credit-note").filter({ hasText: "当前卡密即 API Key" }).waitFor();
      assert.equal(await page.locator(".account-profile-list .account-balance").innerText(), "12.5 点");
      assert.equal(await page.locator("#account-change-form").count(), 0);
      assert.equal(await page.locator(".account-summary").count(), 0);
      assert.equal(await page.locator("#account-recharge-panel").isVisible(), false);
      assert.equal(await page.evaluate(() => window.calls.some(c => c.command === "wallet_catalog")), false);
      const loginKami = page.locator(".account-login-key > span");
      assert.notEqual(await loginKami.innerText(), "wallet-test-card");
      await page.locator("#toggle-login-kami").click();
      assert.equal(await page.locator(".account-login-key > span").innerText(), "wallet-test-card");
      await page.locator("#toggle-login-kami").click();
      assert.notEqual(await page.locator(".account-login-key > span").innerText(), "wallet-test-card");
      if (scenario === "package") await page.screenshot({ path: path.join(output, "account-info.png"), fullPage: true });
      await page.setViewportSize({ width: 390, height: 820 });
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, "Personal info must fit a mobile viewport");
      await page.setViewportSize({ width: 1120, height: 820 });
      await page.locator("#account-recharge-tab").click();
      assert.equal(await page.locator("#account-info-panel").isVisible(), false);
      assert.equal(await page.locator("#account-recharge-panel").isVisible(), true);
      await page.locator("#wallet-catalog-retry").waitFor();
      assert.equal(await page.locator(".account-subnav").isVisible(), true);
      assert.equal(await page.locator("#account-recharge-tab").getAttribute("aria-selected"), "true");
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
      if (scenario === "package") await page.screenshot({ path: path.join(output, "wallet-layout.png"), fullPage: true });
      if (scenario === "catalog-error") {
        await page.locator("#wallet-payment-status").filter({ hasText: "读取充值配置失败" }).waitFor();
        assert(await page.locator("#wallet-pay").isDisabled());
        await page.evaluate(() => { window.failCatalog = false; });
        await page.locator("#wallet-catalog-retry").click();
      }
      await page.waitForFunction(() => document.querySelectorAll(".wallet-package").length > 0);
      assert.equal(await page.locator('.wallet-package input:checked').inputValue(), scenario === "package-no-100" ? "1" : "3");
      assert.equal(await page.locator("#wallet-online").isVisible(), true);
      await page.locator("#card-recharge").click();
      assert.equal(await page.locator("#wallet-online").isVisible(), false);
      assert.equal(await page.locator("#account-recharge-form").isVisible(), true);
      await page.locator("#online-recharge").click();
      if (scenario === "package") {
        await page.screenshot({ path: path.join(output, "wallet-dark.png"), fullPage: true });
        await page.locator("#theme-toggle").click();
        await page.screenshot({ path: path.join(output, "wallet-light.png"), fullPage: true });
        await page.setViewportSize({ width: 760, height: 820 });
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
        await page.screenshot({ path: path.join(output, "wallet-narrow.png"), fullPage: true });
        await page.setViewportSize({ width: 390, height: 820 });
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
        await page.screenshot({ path: path.join(output, "wallet-mobile.png"), fullPage: true });
        await page.setViewportSize({ width: 1120, height: 820 });
      }
      if (scenario === "custom") {
        await page.locator("#wallet-custom").fill("1.001");
        await page.locator("#wallet-pay").click();
        assert.equal(await page.evaluate(() => window.calls.filter(c => c.command === "wallet_create_order").length), 0);
        await page.locator("#wallet-custom").fill("12.34");
        assert.equal(await page.locator('.wallet-package input:checked').count(), 0, "Typing a custom amount clears the fixed package selection");
        await page.locator('.wallet-package input').first().evaluate(input => { input.checked = true; });
      }
      await page.locator("#wallet-pay").click();
      if (["unsafe", "create-error"].includes(scenario)) {
        await page.locator("#wallet-payment-status").filter({ hasText: "创建订单失败" }).waitFor();
        assert.equal(await page.locator(".wallet-qr iframe").count(), 0);
        assert.equal(await page.locator("#wallet-pay").isEnabled(), true);
      } else if (scenario === "logout") {
        await page.waitForFunction(() => typeof window.resolveOrder === "function");
        await page.locator("#logout").click();
        await page.evaluate(() => window.resolveOrder());
        await page.waitForTimeout(200);
        assert.equal(await page.locator(".wallet-qr iframe").count(), 0);
        assert.equal(await page.locator('[data-page="account"]').count(), 0);
      } else {
        await page.locator(".wallet-qr iframe").waitFor();
        const args = await page.evaluate(() => window.calls.find(c => c.command === "wallet_create_order").args);
        assert.equal(args.custom, scenario === "custom");
        assert.equal(args.value, scenario === "custom" ? 12.34 : scenario === "package-no-100" ? 300 : 1000);
        assert.equal(args.kami, "wallet-test-card");
        if (scenario === "card-login-zero") assert.equal(args.accountId, 0, "Card login must allow the server's unbound account id");
        assert.equal(await page.locator("#wallet-pay").isDisabled(), true);
        await page.locator("#account-info-tab").click();
        await page.locator("#account-recharge-tab").click();
        assert.equal(await page.locator("#wallet-order-summary").innerText().then(text => text.includes("TEST-ORDER")), true);
        if (scenario === "poll-error") {
          await page.locator("#wallet-check").click();
          await page.locator("#wallet-payment-status").filter({ hasText: "将继续重试" }).waitFor();
        }
        await page.evaluate(() => { window.paid = true; });
        if (scenario !== "package") await page.locator("#wallet-check").click();
        await page.locator(".wallet-paid").waitFor();
        assert.equal(await page.locator(".account-balance b").innerText(), "312.5 点");
        assert.equal(await page.locator("#wallet-pay").isEnabled(), true);
      }
      assert.equal(await page.evaluate(() => window.calls.some(c => c.command === "open_account_center" || c.command === "open_external_url")), false);
      assert.deepEqual(errors, []);
      await page.close();
      console.log(`Wallet ${scenario}: PASS`);
    }
  } finally {
    await browser?.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
