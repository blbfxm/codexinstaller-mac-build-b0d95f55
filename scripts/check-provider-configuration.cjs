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
    for (const platform of ["macos", "windows"]) {
      for (const scenario of ["running", "stopped", "catalog-failure", "stop-failure"]) {
        const page = await browser.newPage();
        const errors = [];
        page.on("pageerror", error => errors.push(error.message));
        await page.addInitScript(({ platform, scenario }) => {
          window.calls = [];
          window.confirm = () => { throw new Error("Native confirm is unavailable"); };
          let running = scenario !== "stopped";
          let providers = [{ id: "test", name: "Test Provider", baseUrl: "https://example.test/v1", model: "old-model", models: ["old-model"], apiKey: "test-key", hasApiKey: true, isActive: true, protocol: "responses", relayMode: "pureApi" }];
          window.__TAURI_INTERNALS__ = {
            metadata: { currentWindow: { label: "main" } },
            transformCallback: () => 1,
            invoke: async (command, args) => {
              window.calls.push({ command, args });
              if (command === "load_auto_login_kami") return "test-card";
              if (command === "account_login") return { ok: true, account: { id: 1 }, installerConfig: {} };
              if (command === "device_info") return { platform, arch: "arm64" };
              if (command === "fetch_site_config") return {};
              if (command === "list_codexpp_providers") return providers;
              if (command === "check_codexpp_provider") return ["old-model", "new-model"];
              if (command === "check_installations") return { codex: { installed: true, path: "Codex.app" }, codexpp: { installed: false, path: "" } };
              if (command === "product_process_running") return args.product === "codex" && running;
              if (command === "stop_codex_processes") {
                if (scenario === "stop-failure") throw "无法关闭 Codex";
                running = false;
              }
              if (command === "configure_codexpp_provider") {
                if (running) throw "Codex is still running";
                if (scenario === "catalog-failure") throw "无法读取 Codex 模型目录，未应用供应商配置";
                providers = [{ ...providers[0], model: args.model }];
                return "供应商配置完成";
              }
              if (command === "launch_installed_product") running = true;
              return null;
            },
          };
        }, { platform, scenario });
        await page.goto(`http://127.0.0.1:${server.address().port}/`);
        await page.locator('[data-page="config"]').click();
        await page.locator('[data-edit-provider="test"]').click();
        await page.locator("#provider-model").fill("new-model");
        const configure = page.locator("#configure-provider");
        await configure.click();
        if (scenario !== "stopped") {
          await page.locator("[data-confirm-cancel]").click();
          await configure.waitFor({ state: "visible" });
          assert.deepEqual(await page.evaluate(() => window.calls.filter(call => ["stop_codex_processes", "configure_codexpp_provider", "launch_installed_product"].includes(call.command))), []);
          await configure.click();
          await page.locator("[data-confirm-accept]").click();
        }
        if (scenario.endsWith("failure")) {
          await page.waitForFunction(scenario => document.querySelector("#toast").textContent.includes(scenario === "stop-failure" ? "无法关闭 Codex" : "无法读取 Codex 模型目录"), scenario);
          assert.equal(await configure.isEnabled(), true);
        } else {
          await page.locator('[data-edit-provider="test"]').waitFor();
          assert.match(await page.locator(".provider-item-main small").innerText(), /new-model/);
          if (scenario === "running") assert.match(await page.locator("#toast").innerText(), /已重启.*新建会话/);
        }
        const mutations = await page.evaluate(() => window.calls.filter(call => ["stop_codex_processes", "configure_codexpp_provider", "launch_installed_product"].includes(call.command)));
        assert.deepEqual(mutations.map(call => call.command), scenario === "stopped" ? ["configure_codexpp_provider"] : scenario === "stop-failure" ? ["stop_codex_processes"] : scenario === "catalog-failure" ? ["stop_codex_processes", "configure_codexpp_provider"] : ["stop_codex_processes", "configure_codexpp_provider", "launch_installed_product"]);
        const saved = mutations.find(call => call.command === "configure_codexpp_provider");
        if (saved) assert.equal(saved.args.model, "new-model");
        assert.deepEqual(errors, []);
        console.log(`PASS (${platform}, ${scenario}): provider configuration applies in order and reports failures.`);
        await page.close();
      }
    }
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
