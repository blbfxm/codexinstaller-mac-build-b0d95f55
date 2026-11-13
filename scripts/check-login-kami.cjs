const { chromium } = require(process.env.PLAYWRIGHT_MODULE_PATH || "playwright");
const assert = require("node:assert/strict");
const path = require("node:path");

(async () => {
  const browser = await chromium.launch({ headless: true, channel: "msedge" });
  try {
    const page = await browser.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(error.message));
    await page.goto("http://127.0.0.1:1420/");
    const input = page.locator("#login-kami");
    const toggle = page.locator("#login-kami-visibility");
    await input.fill("test-card-not-a-real-secret");
    assert.equal(await input.getAttribute("type"), "password");
    await toggle.click();
    assert.equal(await input.getAttribute("type"), "text");
    assert.equal(await toggle.getAttribute("aria-label"), "隐藏卡密");
    assert.equal(await input.inputValue(), "test-card-not-a-real-secret");
    assert.equal(await page.locator("#login-error").innerText(), "");
    await toggle.click();
    assert.equal(await input.getAttribute("type"), "password");
    assert.equal(await toggle.getAttribute("aria-pressed"), "false");
    for (const theme of ["dark", "light"]) {
      await page.evaluate(theme => { document.documentElement.dataset.theme = theme; }, theme);
      for (const width of [1120, 390]) {
        await page.setViewportSize({ width, height: 820 });
        const field = await input.boundingBox();
        const button = await toggle.boundingBox();
        assert(button.x >= field.x && button.x + button.width <= field.x + field.width);
        assert(button.y >= field.y && button.y + button.height <= field.y + field.height);
        assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false);
        await page.screenshot({ path: path.resolve(__dirname, `../dist-portable/ui-check/login-kami-${theme}-${width}.png`), fullPage: true });
      }
    }
    await page.locator('[data-login-mode="password"]').click();
    await page.locator('[data-login-mode="kami"]').click();
    assert.equal(await input.getAttribute("type"), "password");
    assert.deepEqual(errors, []);
    console.log("PASS: login kami visibility toggle, preserved value, no form submission, default masking and responsive themes.");
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exit(1); });
