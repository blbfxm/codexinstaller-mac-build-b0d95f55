import { invoke } from "@tauri-apps/api/core";
import { confirmDialog } from "./confirm-dialog";

type Plugin = { id: string; name: string; displayName: string; description: string; descriptionZh: string; version: string; author: string; tags: string[]; license: string; bytes: number; skills: number; requiresAuth: boolean; tree: string };
type Installed = { id: string; version: string; tree: string; enabled: boolean };
type Catalog = { items: Plugin[]; installed: Installed[]; origin: string; updatedAt: string; warning: string };
type Options = { codexInstalled: boolean; goInstall: () => void; restartCodex: () => Promise<unknown> };
type Filter = "all" | "available" | "installed" | "updates" | "auth";
const esc = (s: unknown) => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
const errorMessage = (e: unknown) => typeof e === "string" ? e : e instanceof Error ? e.message : "操作失败，请重试";
const request = <T>(action: string, payload = {}) => invoke<T>("public_plugin_market", { action, payload });
let saved: Catalog | null = null;
let filters = { q: "", draft: "", filter: "all" as Filter, page: 0 };
const pending = new Map<string, string>();
const notices = new Map<string, string>();
let changes = 0;
export function resetPublicPluginMarket() { saved = null; filters = { q: "", draft: "", filter: "all", page: 0 }; notices.clear(); }
const size = (bytes: number) => bytes >= 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(1)} MiB` : `${(bytes / 1024).toFixed(1)} KiB`;

export function renderPublicPluginMarket(host: HTMLElement, options: Options) {
  host.innerHTML = `<div class="page-heading"><div><h1>插件市场</h1><p class="muted">发现适合您的 Codex 插件，查看中文介绍后按需安装。</p></div><div class="market-heading-actions"><button id="public-restart" class="button outline">重启 Codex</button><button id="public-refresh" class="button quiet">刷新市场</button></div></div><form id="public-search-form" class="skills-toolbar"><label class="skill-search"><input id="public-search" type="search" aria-label="搜索插件" placeholder="搜索名称、作者、中文说明或标签，按 Enter 搜索" value="${esc(filters.draft)}"></label><select aria-label="插件来源" disabled><option>公开精选</option></select><button class="button outline" type="submit">搜索</button></form><div class="public-market-filters" role="group" aria-label="插件筛选"></div><p id="public-source" class="microcopy"></p><p id="public-feedback" role="status" aria-live="polite"></p>${!options.codexInstalled ? '<div class="panel public-install-hint">安装插件前请先安装 Codex。<button id="public-go-install" class="button outline">前往安装</button></div>' : ""}<div id="public-results" class="market-grid public-market-grid"></div><div id="public-pagination" class="market-pagination"></div>`;
  const alive = () => host.isConnected && !!host.querySelector("#public-results");
  const feedback = host.querySelector<HTMLElement>("#public-feedback")!;
  const refresh = host.querySelector<HTMLButtonElement>("#public-refresh")!;
  let loading = false;
  const state = (p: Plugin) => saved?.installed.find(i => i.id === p.id);
  const matches = (p: Plugin, f: Filter) => {
    const i = state(p);
    return f === "all" || (f === "available" && !i) || (f === "installed" && !!i) || (f === "updates" && !!i && i.tree !== p.tree) || (f === "auth" && p.requiresAuth);
  };
  const label = (p: Plugin) => pending.get(p.id) || (!options.codexInstalled ? "请先安装 Codex" : !state(p) ? "安装" : state(p)!.tree !== p.tree ? "更新" : !state(p)!.enabled ? "启用" : "已安装");
  const draw = () => {
    if (!alive() || !saved) return;
    const items = saved.items;
    host.querySelector(".public-market-filters")!.innerHTML = ([["all", "全部插件"], ["available", "可安装"], ["installed", "已安装"], ["updates", "有更新"], ["auth", "需服务授权"]] as [Filter, string][]).map(([f, text]) => `<button type="button" data-public-filter="${f}" class="button ${filters.filter === f ? "primary" : "outline"}" aria-pressed="${filters.filter === f}">${text} <span>${items.filter(p => matches(p, f)).length}</span></button>`).join("");
    host.querySelectorAll<HTMLButtonElement>("[data-public-filter]").forEach(b => b.addEventListener("click", () => { filters.filter = b.dataset.publicFilter as Filter; filters.page = 0; draw(); }));
    const q = filters.q.toLocaleLowerCase();
    const rows = items.filter(p => matches(p, filters.filter) && [p.name, p.displayName, p.descriptionZh, p.description, p.author, ...p.tags].join(" ").toLocaleLowerCase().includes(q));
    const pages = Math.max(1, Math.ceil(rows.length / 48)); filters.page = Math.min(filters.page, pages - 1);
    host.querySelector("#public-source")!.textContent = `公开精选 · ${items.length} 个插件 · ${saved.origin === "online" ? "已在线刷新" : saved.origin === "cache" ? "已缓存目录" : "内置目录"}${saved.updatedAt ? ` · 索引更新于 ${saved.updatedAt}` : ""}`;
    host.querySelector("#public-results")!.innerHTML = rows.length ? rows.slice(filters.page * 48, (filters.page + 1) * 48).map(p => `<article class="panel market-card public-plugin-card"><header><h3><span aria-hidden="true">◇</span> ${esc(p.displayName)}</h3>${p.requiresAuth ? '<span class="plugin-badge" title="使用时需授权相应服务">需服务授权</span>' : ""}</header><p class="public-plugin-description">${esc(p.descriptionZh)}</p><small class="microcopy">${esc(p.author)}</small><footer><span class="microcopy">v${esc(p.version)} · ${size(p.bytes)}</span><div class="public-plugin-actions"><button class="button quiet" data-public-detail="${esc(p.id)}">详情</button><button class="button outline" data-public-install="${esc(p.id)}" ${pending.has(p.id) || !options.codexInstalled || (state(p)?.enabled && state(p)?.tree === p.tree) ? "disabled" : ""}>${esc(label(p))}</button></div></footer>${notices.has(p.id) ? `<p class="microcopy public-plugin-notice" role="status">${esc(notices.get(p.id))}</p>` : ""}</article>`).join("") : '<section class="panel market-empty">没有符合条件的插件，请调整搜索词或筛选条件。</section>';
    const find = (id: string) => items.find(p => p.id === id)!;
    host.querySelectorAll<HTMLButtonElement>("[data-public-install]").forEach(b => b.addEventListener("click", () => void operate(find(b.dataset.publicInstall!), false)));
    host.querySelectorAll<HTMLButtonElement>("[data-public-detail]").forEach(b => b.addEventListener("click", () => showDetail(find(b.dataset.publicDetail!))));
    const pagination = host.querySelector("#public-pagination")!;
    pagination.innerHTML = `<button class="button outline" id="public-prev" ${filters.page === 0 ? "disabled" : ""}>上一页</button><span>共 ${rows.length} 个 · 第 ${filters.page + 1} / ${pages} 页</span><button class="button outline" id="public-next" ${filters.page + 1 >= pages ? "disabled" : ""}>下一页</button>`;
    for (const [id, delta] of [["public-prev", -1], ["public-next", 1]] as const) pagination.querySelector(`#${id}`)!.addEventListener("click", () => { filters.page += delta; draw(); host.querySelector("#public-search-form")!.scrollIntoView({ block: "start" }); });
  };
  async function load(force = false) {
    if (loading) return;
    loading = true; refresh.disabled = true;
    if (!saved) host.querySelector("#public-results")!.innerHTML = '<section class="panel market-empty">正在读取插件目录…</section>';
    feedback.textContent = force ? "正在刷新公开插件目录…" : ""; feedback.className = "microcopy";
    const started = changes;
    try {
      const result = await request<Catalog>("list", { refresh: force });
      if (started !== changes && saved) result.installed = saved.installed;
      saved = result;
      if (alive()) { feedback.textContent = result.warning || ""; feedback.className = result.warning ? "error-line" : "microcopy"; draw(); }
    } catch (e) {
      if (alive()) { feedback.textContent = errorMessage(e); feedback.className = "error-line"; if (!saved) host.querySelector("#public-results")!.innerHTML = '<section class="panel market-empty">目录暂时无法加载，请点击“刷新市场”重试。</section>'; }
    } finally { loading = false; if (alive()) refresh.disabled = false; }
  }
  async function operate(p: Plugin, remove: boolean) {
    if (pending.has(p.id) || loading || (!remove && !options.codexInstalled)) return;
    if (remove && !await confirmDialog(`卸载 ${p.displayName}`, "卸载此市场安装的插件后，请重新启动 Codex。", "卸载插件")) return;
    pending.set(p.id, remove ? "正在卸载…" : "正在安装…"); notices.delete(p.id); draw();
    try {
      const result = await request<Installed>(remove ? "uninstall" : "install", { id: p.id, tree: p.tree });
      changes++;
      if (saved) { saved.installed = saved.installed.filter(i => i.id !== p.id); if (!remove) saved.installed.push(result); }
      notices.set(p.id, remove ? "已卸载，请重启 Codex。" : `已安装，请重启 Codex 后输入 @ 选择插件。${p.requiresAuth ? "使用时还需授权相应服务。" : ""}`);
    } catch (e) { notices.set(p.id, errorMessage(e)); }
    finally { pending.delete(p.id); draw(); }
  }
  function showDetail(p: Plugin) {
    const dialog = document.createElement("dialog"); dialog.className = "builtin-plugin-dialog public-plugin-dialog";
    dialog.setAttribute("aria-labelledby", "public-detail-title");
    dialog.innerHTML = `<header><h2 id="public-detail-title">${esc(p.displayName)}</h2><button type="button" class="icon-button" data-close aria-label="关闭详情">×</button></header><div class="plugin-help-body"><p>${esc(p.descriptionZh)}</p><dl class="public-plugin-metadata"><dt>作者</dt><dd>${esc(p.author)}</dd><dt>版本</dt><dd>${esc(p.version)}</dd><dt>大小</dt><dd>${size(p.bytes)}</dd><dt>技能数量</dt><dd>${p.skills}</dd><dt>许可</dt><dd>${esc(p.license || "未提供")}</dd><dt>安装状态</dt><dd>${esc(state(p) ? state(p)!.tree !== p.tree ? `有更新（当前 ${state(p)!.version}）` : state(p)!.enabled ? "已安装" : "已安装，未启用" : "未安装")}</dd></dl>${p.tags.length ? `<p class="microcopy">${p.tags.map(esc).join(" · ")}</p>` : ""}<h3>安装后如何使用</h3><p>安装后重新启动 Codex，在聊天框输入 @，选择插件并描述您的需求。</p><h3>服务授权与费用</h3><p>${p.requiresAuth ? "此插件声明需要外部服务授权。使用前请按插件指引登录或配置对应服务，服务费用由相应提供方决定。" : "此插件未声明额外授权要求；实际运行环境和外部服务费用请以插件及服务商说明为准。"}下载插件不会扣除坚果点数。</p><details><summary>原文说明</summary><p class="market-prose">${esc(p.description)}</p></details></div><footer>${state(p) ? '<button class="button outline danger-text" id="public-remove">卸载</button>' : ""}<button class="button outline" data-close>关闭</button><button class="button primary" id="public-detail-install" ${pending.has(p.id) || !options.codexInstalled || (state(p)?.enabled && state(p)?.tree === p.tree) ? "disabled" : ""}>${esc(label(p))}</button></footer>`;
    dialog.querySelectorAll("[data-close]").forEach(b => b.addEventListener("click", () => dialog.close()));
    dialog.querySelector("#public-detail-install")!.addEventListener("click", () => { dialog.close(); void operate(p, false); });
    dialog.querySelector("#public-remove")?.addEventListener("click", () => { dialog.close(); void operate(p, true); });
    dialog.addEventListener("click", e => { if (e.target === dialog) { const r = dialog.getBoundingClientRect(); if (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom) dialog.close(); } });
    dialog.addEventListener("close", () => dialog.remove(), { once: true }); host.append(dialog); dialog.showModal();
  }
  host.querySelector("#public-go-install")?.addEventListener("click", options.goInstall);
  const restart = host.querySelector<HTMLButtonElement>("#public-restart")!; restart.disabled = !options.codexInstalled;
  restart.addEventListener("click", async () => {
    restart.disabled = true;
    try { await options.restartCodex(); if (alive()) feedback.textContent = "Codex 已重新启动。"; }
    catch (e) { if (alive()) { feedback.textContent = errorMessage(e); feedback.className = "error-line"; } }
    finally { if (alive()) restart.disabled = !options.codexInstalled; }
  });
  refresh.addEventListener("click", () => void load(true));
  const search = host.querySelector<HTMLInputElement>("#public-search")!;
  search.addEventListener("input", () => { filters.draft = search.value; });
  host.querySelector("#public-search-form")!.addEventListener("submit", e => { e.preventDefault(); filters.q = search.value.trim(); filters.draft = search.value; filters.page = 0; draw(); });
  if (saved) draw();
  void load();
}
