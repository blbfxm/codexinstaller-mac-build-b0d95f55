// Enable Codex's built-in language resources before its renderer reads the locale gate.
(() => {
  if (window.__nutCodexChinese) return;
  const status = window.__nutCodexChinese = { locale: false, gate: false, rendered: false, error: "" };
  const locale = "zh-CN";
  const pageLoaded = new Promise(resolve => {
    if (document.readyState === "complete") resolve();
    else window.addEventListener("load", resolve, { once: true });
  }).then(() => new Promise(resolve => setTimeout(resolve, 1000)));
  for (const [key, value] of [["language", locale], ["languages", [locale, "zh", "en-US", "en"]]]) {
    try { Object.defineProperty(navigator, key, { configurable: true, get: () => value }); } catch {}
  }
  const patchedMethods = new WeakSet();
  const patchValue = value => {
    if (!value || typeof value !== "object") return value;
    for (const field of ["value", "__value"]) {
      try { value[field] = { ...value[field], enable_i18n: true, locale_source: "SYSTEM" }; } catch {}
    }
    if (typeof value.get === "function" && !value.__nutChinese) {
      const original = value.get.bind(value);
      value.get = (key, fallback) => key === "enable_i18n" ? true : key === "locale_source" ? "SYSTEM" : original(key, fallback);
      value.__nutChinese = true;
    }
    return value;
  };
  const patchClient = client => {
    if (!client || typeof client !== "object") return;
    for (const method of ["getLayer", "_getLayerImpl", "getDynamicConfig"]) {
      if (typeof client[method] !== "function" || patchedMethods.has(client[method])) continue;
      const original = client[method].bind(client);
      const wrapped = (name, ...args) => {
        const result = original(name, ...args);
        if (name !== "72216192") return result;
        const patched = patchValue(result);
        if (patched && typeof patched === "object") status.gate = true;
        return patched;
      };
      try {
        client[method] = wrapped;
        patchedMethods.add(wrapped);
      } catch {}
    }
  };
  const roots = new WeakSet();
  const scanRoot = root => {
    if (!root || typeof root !== "object") return;
    for (const key of ["firstInstance", "instance"]) {
      try {
        const value = root[key];
        patchClient(key === "instance" && typeof value === "function" ? value.call(root) : value);
      } catch {}
    }
    try { for (const client of Object.values(root.instances || {})) patchClient(client); } catch {}
  };
  const patchRoot = root => {
    if (!root || typeof root !== "object") return;
    scanRoot(root);
    if (roots.has(root)) return;
    roots.add(root);
    for (const key of ["firstInstance", "instance"]) {
      try {
        let current = root[key];
        Object.defineProperty(root, key, { configurable: true, get: () => current, set: next => { current = next; scanRoot(root); } });
      } catch {}
    }
  };
  let statsig = window.__STATSIG__;
  patchRoot(statsig);
  try {
    Object.defineProperty(window, "__STATSIG__", { configurable: true, get: () => statsig, set: next => { statsig = next; patchRoot(next); } });
  } catch {}
  // Keep watching late-created clients and renderer language changes for this page's lifetime.
  setInterval(() => {
    patchRoot(window.__STATSIG__);
    const chineseResourceLoaded = performance.getEntriesByType("resource").some(entry => {
      try {
        const url = new URL(entry.name, location.href);
        return url.origin === location.origin && /\/zh-CN-[^/]+\.js$/.test(url.pathname) &&
          (!entry.responseStatus || entry.responseStatus < 400);
      } catch { return false; }
    });
    status.rendered = document.documentElement.lang.toLowerCase() === "zh-cn" &&
      chineseResourceLoaded && Boolean(document.body?.textContent?.trim());
  }, 250);

  async function syncLocale() {
    await pageLoaded;
    const urls = new Set([
      ...Array.from(document.querySelectorAll("script[src],link[href]"), node => node.src || node.href),
      ...performance.getEntriesByType("resource").map(entry => entry.name),
    ].filter(url => {
      try { return url && new URL(url, location.href).origin === location.origin; } catch { return false; }
    }));
    // Host-service exports are minified, so locate them by their own property names.
    for (const url of [...urls]) {
      if (!url.includes("/assets/") || !url.endsWith(".js")) continue;
      if (url.includes("app-shared-") || url.includes("app-initial-")) continue;
      let source;
      try {
        const response = await fetch(url, { signal: AbortSignal.timeout(2000) });
        if (!response.ok) continue;
        source = await response.text();
      } catch { continue; }
      for (const match of source.matchAll(/["'`](\.?\/?app-(?:shared|initial)-[^"'`]+\.js)["'`]/g)) {
        urls.add(new URL(match[1], url).href);
      }
    }
    for (const url of urls) {
      if (!/app-(?:shared|initial)-[^/]+\.js/.test(url)) continue;
      let module;
      try { module = await import(url); } catch { continue; }
      for (const host of Object.values(module)) {
        if (!host || typeof host !== "object" ||
          !Object.hasOwn(host, "settings") || !Object.hasOwn(host, "applicationState")) continue;
        if (typeof host.settings?.readAll !== "function" || typeof host.settings?.write !== "function") continue;
        const all = await host.settings.readAll();
        if (all?.values?.localeOverride !== locale) {
          await host.settings.write("localeOverride", locale);
          const confirmed = await host.settings.readAll();
          if (confirmed?.values?.localeOverride !== locale) throw new Error("Codex did not save the Chinese language setting");
          const reloadKey = "nut-codex-chinese-reload";
          if (sessionStorage.getItem(reloadKey) !== locale) {
            sessionStorage.setItem(reloadKey, locale);
            if (sessionStorage.getItem(reloadKey) === locale) {
              location.reload();
              return;
            }
          }
        }
        status.locale = true;
        status.error = "";
        return;
      }
    }
    throw new Error("Codex language settings service unavailable");
  }
  async function syncWithRetry() {
    for (const wait of [0, 500, 1500, 3000, 6000, 12000, 20000]) {
      await new Promise(resolve => setTimeout(resolve, wait));
      try { await syncLocale(); return; }
      catch (error) { status.error = String(error?.message || error); }
    }
  }
  void syncWithRetry();
})();
