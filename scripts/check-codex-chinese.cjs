const { chromium } = require(process.env.PLAYWRIGHT_MODULE_PATH || "playwright");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const http = require("node:http");

(async () => {
  const script = fs.readFileSync(path.join(__dirname, "../src-tauri/src/codex_chinese.js"), "utf8");
  const server = http.createServer((req, res) => {
    if (req.url === "/assets/usage-assistant-chinese.js") {
      res.setHeader("Content-Type", "text/javascript");
      res.end(script);
    } else if (req.url === "/slow-image") {
      setTimeout(() => {
        res.setHeader("Content-Type", "image/gif");
        res.end(Buffer.from("R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7", "base64"));
      }, 1800);
    } else if (req.url === "/assets/zh-CN-test.js") {
      res.setHeader("Content-Type", "text/javascript");
      res.end('export const text = "中文";');
    } else if (req.url === "/assets/app-shared-test.js") {
      res.setHeader("Content-Type", "text/javascript");
      res.end(`export const host = { applicationState: {}, settings: {
        readAll: async () => ({ values: { localeOverride: localStorage.getItem("test-locale") } }),
        write: async (key, value) => {
          if (document.readyState !== "complete") throw new Error("write interrupted initial navigation");
          localStorage.setItem("test-locale", value);
        }
      }};`);
    } else if (req.url === "/assets/main.js") {
      res.setHeader("Content-Type", "text/javascript");
      res.end(`import { host } from "./app-shared-test.js";
        const client = {
          getLayer: () => ({ __value: { unrelated: 7 }, get: (key, fallback) => fallback }),
          _getLayerImpl: () => ({ get: (key, fallback) => fallback }),
          getDynamicConfig: () => ({ get: (key, fallback) => fallback })
        };
        window.__STATSIG__ = { instance: () => { throw new Error("not initialized"); } };
        setTimeout(async () => {
          window.__STATSIG__.firstInstance = client;
          const enabled = client.getLayer("72216192").get("enable_i18n", false);
          document.documentElement.lang = enabled ? "zh-CN" : "en";
          document.getElementById("test-content").textContent = enabled ? (await import("./zh-CN-test.js")).text : "English";
        }, 300);
      `);
    } else {
      res.setHeader("Content-Type", "text/html");
      res.end('<html><head>' + (req.url.includes("persistent") ? '<meta http-equiv="Content-Security-Policy" content="script-src &apos;self&apos;"><script src="./assets/usage-assistant-chinese.js"></script>' : '') + '</head><body><p id="test-content"></p>' + (req.url.includes("slow") ? '<img src="/slow-image">' : '') +
        '<script type="module" src="/assets/main.js"></script></body></html>');
    }
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  const browser = await chromium.launch({ channel: "msedge", headless: true });
  try {
    const page = await browser.newPage();
    await page.route("**/*", route => new URL(route.request().url()).hostname === "127.0.0.1" ? route.continue() : route.abort());
    await page.addInitScript(script);
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    await page.waitForFunction(() => window.__nutCodexChinese.rendered && window.__nutCodexChinese.locale && window.__nutCodexChinese.gate);
    assert.equal(await page.locator("body").innerText(), "中文");
    assert.equal(await page.evaluate(() => localStorage.getItem("test-locale")), "zh-CN");
    assert.equal(await page.evaluate(() => navigator.language), "zh-CN");
    const gates = await page.evaluate(() => {
      const c = window.__STATSIG__.firstInstance;
      return [c.getLayer("72216192").get("enable_i18n", false),
        c._getLayerImpl("72216192").get("enable_i18n", false),
        c.getDynamicConfig("72216192").get("locale_source", "IDE"),
        c.getLayer("other").get("enable_i18n", false),
        c.getLayer("72216192").__value.unrelated];
    });
    assert.deepEqual(gates, [true, true, "SYSTEM", false, 7]);
    await page.reload();
    await page.waitForFunction(() => window.__nutCodexChinese.rendered && window.__nutCodexChinese.locale && window.__nutCodexChinese.gate);
    assert.equal(await page.locator("body").innerText(), "中文");
    await page.evaluate(() => {
      document.documentElement.lang = "en";
      document.body.textContent = "English";
    });
    await page.waitForFunction(() => !window.__nutCodexChinese.rendered);
    await page.reload();
    await page.waitForFunction(() => window.__nutCodexChinese.rendered);
    // A wrapper being installed must not count as a gate that the application has read.
    const empty = await browser.newPage();
    await empty.goto(`http://127.0.0.1:${server.address().port}/assets/zh-CN-test.js`);
    await empty.evaluate(script);
    await empty.evaluate(() => {
      window.__STATSIG__ = { firstInstance: { getLayer: () => ({ get: (_, fallback) => fallback }) } };
    });
    assert.equal(await empty.evaluate(() => window.__nutCodexChinese.gate), false);
    assert.equal(await empty.evaluate(() => window.__nutCodexChinese.rendered), false);
    await empty.close();
    const context = await browser.newContext();
    await context.route("**/*", route => new URL(route.request().url()).hostname === "127.0.0.1" ? route.continue() : route.abort());
    const attachedPage = await context.newPage();
    await attachedPage.goto(`http://127.0.0.1:${server.address().port}`);
    await attachedPage.waitForFunction(() => document.body.textContent === "English");
    assert.equal(await attachedPage.locator("body").innerText(), "English");
    const session = await context.newCDPSession(attachedPage);
    await session.send("Page.enable");
    await session.send("Page.addScriptToEvaluateOnNewDocument", { source: script });
    await session.send("Page.reload");
    await attachedPage.waitForFunction(() => window.__nutCodexChinese?.rendered && window.__nutCodexChinese?.locale);
    // Exercise the production DevTools registration across multiple navigations.
    for (let i = 0; i < 2; i++) {
      await session.send("Page.reload");
      await attachedPage.waitForFunction(() => window.__nutCodexChinese?.rendered && window.__nutCodexChinese?.locale);
      assert.equal(await attachedPage.locator("body").innerText(), "中文");
    }
    await context.close();
    const slowContext = await browser.newContext();
    await slowContext.addInitScript(script);
    const slow = await slowContext.newPage();
    await slow.goto(`http://127.0.0.1:${server.address().port}/?slow=1`, { waitUntil: "domcontentloaded" });
    assert.equal(await slow.evaluate(() => localStorage.getItem("test-locale")), null);
    await slow.waitForFunction(() => window.__nutCodexChinese?.locale && window.__nutCodexChinese?.rendered);
    assert.equal(await slow.evaluate(() => localStorage.getItem("test-locale")), "zh-CN");
    await slowContext.close();
    // Simulate ordinary app restarts. No assistant injection or DevTools
    // registration is present; the installed entry-page script must suffice.
    let storageState;
    for (let launch = 0; launch < 3; launch++) {
      const fresh = await browser.newContext({ storageState });
      const direct = await fresh.newPage();
      await direct.goto(`http://127.0.0.1:${server.address().port}/persistent`);
      await direct.waitForFunction(() => window.__nutCodexChinese?.rendered && window.__nutCodexChinese?.locale);
      assert.equal(await direct.locator("body").innerText(), "中文");
      storageState = await fresh.storageState();
      await fresh.close();
    }
    console.log("PASS: offline Chinese, delayed clients, DevTools reloads, slow navigation, and three ordinary app launches without assistant injection");
  } finally {
    await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
