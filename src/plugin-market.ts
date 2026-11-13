import { invoke } from "@tauri-apps/api/core";
import { renderPlugins } from "./plugins";

type Item = { id: number; name: string; slug: string; version: string; description: string; instructions?: string; author: string; price: number; downloads: number; status: string; reviewNote?: string; screenshots?: string[]; coverImage?: string; requiresAuthorization?: boolean | null; authorizationDetails?: string; billingDetails?: string };
type Installed = { id: number; slug: string; version: string };
type Options = { codexInstalled: boolean; getApiKey: () => Promise<string>; goInstall: () => void; goShare: () => void; goMarket: () => void; kind?: "plugin" | "skill" };
const esc = (value: unknown) => String(value ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
const message = (e: unknown) => typeof e === "string" ? e : e instanceof Error ? e.message : "操作失败，请稍后重试";
let generation = 0;
let filters = { q: "", draft: "", sort: "downloads", page: 0 };
const blankDraft = (author = "") => ({ name: "", author, description: "", instructions: "", price: "10", requiresAuthorization: "", authorizationDetails: "", billingDetails: "" });
let draft = blankDraft();
const blankFiles = () => ({ archive: null as File | null, screenshots: [] as File[], previews: [] as string[], imageError: "", imagePending: false, imageVersion: 0, refresh: null as (() => void) | null });
let files = blankFiles();
let editing: Item | null = null;
let screenshotsChanged = false;
let currentKind = "plugin";
const contexts = new Map<string, { filters: typeof filters; draft: typeof draft; files: typeof files; editing: Item | null; screenshotsChanged: boolean }>();
export function resetPluginMarket() { generation++; contexts.clear(); currentKind = "plugin"; filters = { q: "", draft: "", sort: "downloads", page: 0 }; draft = blankDraft(); files = blankFiles(); editing = null; screenshotsChanged = false; }
async function marketRequest<T>(action: string, payload: object = {}, apiKey?: string) {
  return invoke<T>("plugin_market", { action, payload, apiKey: apiKey || null });
}
function dataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => { const r = new FileReader(); r.onload = () => resolve(String(r.result)); r.onerror = () => reject(new Error("读取上传文件失败")); r.readAsDataURL(file); });
}
async function prepareScreenshot(file: File): Promise<File> {
  if (!["image/png", "image/jpeg"].includes(file.type) || file.size > 20 * 1024 * 1024) throw new Error("请选择不超过 20 MB 的 PNG 或 JPEG 图片。");
  const img = new Image();
  await new Promise<void>((resolve, reject) => { img.onload = () => resolve(); img.onerror = () => reject(new Error(`无法读取图片“${file.name}”，请重新选择。`)); void dataUrl(file).then(url => { img.src = url; }, reject); });
  if (img.naturalWidth * img.naturalHeight > 36_000_000) throw new Error("图片尺寸过大，请缩小后重新选择。");
  if (file.size <= 1024 * 1024 && img.naturalWidth <= 6000 && img.naturalHeight <= 6000) return file;
  const canvas = document.createElement("canvas"), ratio = Math.min(1, 1600 / Math.max(img.naturalWidth, img.naturalHeight));
  canvas.width = Math.max(1, Math.round(img.naturalWidth * ratio)); canvas.height = Math.max(1, Math.round(img.naturalHeight * ratio));
  const ctx = canvas.getContext("2d"); if (!ctx) throw new Error("无法处理介绍图，请重试。");
  ctx.fillStyle = "#fff"; ctx.fillRect(0, 0, canvas.width, canvas.height); ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
  for (const quality of [.88, .72, .55]) {
    const blob = await new Promise<Blob | null>(resolve => canvas.toBlob(resolve, "image/jpeg", quality));
    if (blob && blob.size <= 1024 * 1024) return new File([blob], file.name.replace(/\.[^.]+$/, "") + ".jpg", { type: "image/jpeg" });
  }
  throw new Error("图片压缩后仍过大，请缩小图片后重新选择。");
}
function sourceBadge() { return `<span class="plugin-badge is-shared" data-plugin-source="user">用户分享</span>`; }
function coverImage(item: Item) {
  const url = item.coverImage || item.screenshots?.[0] || "";
  if (!/^data:image\/(png|jpeg);base64,[A-Za-z0-9+/=]+$/.test(url) || url.length > 1500000) return "";
  return `<div class="market-card-cover"><img src="${url}" alt="${esc(item.name)}介绍图" loading="lazy" decoding="async"></div>`;
}
const statusName = (status: string) => ({ pending: "待审核", published: "已发布", rejected: "未通过", withdrawn: "已下架" })[status] || status;
const authorizationName = (item: Item) => item.requiresAuthorization === true ? "需服务授权" : item.requiresAuthorization === false ? "无需服务授权" : "授权信息待补充";
function serviceDisclosure(item: Item) {
  return `<section class="market-service-disclosure"><h3>服务授权与使用费用</h3><p><strong>${authorizationName(item)}</strong></p>${item.requiresAuthorization === true ? `<p class="market-prose">${esc(item.authorizationDetails || "作者暂未补充授权方式，请在使用前向作者确认。")}</p>` : item.requiresAuthorization === false ? '<p>作者声明：使用此插件无需另行申请服务授权。</p>' : '<p>作者尚未说明是否需要额外账号或 API Key，请在使用前确认。</p>'}<h4>服务使用如何计费</h4><p class="market-prose">${esc(item.billingDetails || "作者暂未补充服务费用说明，请在使用前向作者确认。")}</p><p class="microcopy">下载费用与服务使用费用分别计算。购买插件不代表已包含作者说明中的 API 或其他服务费用。</p></section>`;
}
function authorizationFields() {
  return `<fieldset class="share-service-fields"><legend>服务授权与使用费用</legend><label>是否需要服务授权<select name="requiresAuthorization" id="share-requires-authorization" required><option value="">请选择</option><option value="no" ${draft.requiresAuthorization === "no" ? "selected" : ""}>不需要额外账号或 API Key</option><option value="yes" ${draft.requiresAuthorization === "yes" ? "selected" : ""}>需要服务授权</option></select></label><div id="share-authorization-fields"><label>使用什么服务、如何授权<textarea name="authorizationDetails" maxlength="3000" rows="4" placeholder="写明使用哪些 API 或服务，去哪里注册或申请授权，以及如何设置账号或 API Key。请勿填写真实密钥。">${esc(draft.authorizationDetails)}</textarea></label></div><label>服务使用费用说明<textarea name="billingDetails" maxlength="3000" rows="4" required placeholder="写明服务是否免费、由谁收费、如何计费及费用查询方式。免费或离线插件也请明确说明。此处填写使用费用，不是插件下载价格。">${esc(draft.billingDetails)}</textarea></label></fieldset>`;
}

export function renderPluginMarket(host: HTMLElement, options: Options, mine = false) {
  const kind = options.kind || "plugin";
  if (currentKind !== kind) {
    contexts.set(currentKind, { filters, draft, files, editing, screenshotsChanged });
    const saved = contexts.get(kind);
    filters = saved?.filters || { q: "", draft: "", sort: "downloads", page: 0 }; draft = saved?.draft || blankDraft();
    files = saved?.files || blankFiles(); editing = saved?.editing || null; screenshotsChanged = saved?.screenshotsChanged || false;
    currentKind = kind;
  }
  const request = <T>(action: string, payload: object = {}, key?: string) => marketRequest<T>(action, { ...payload, kind }, key);
  const adaptSkillText = (root: HTMLElement) => {
    if (kind !== "skill") return;
    const targets = root.querySelectorAll(".page-heading h1, .page-heading .muted, #market-back, #market-search-form button, #market-pagination span, [data-market-detail], #market-install, #market-remove, .market-dialog-body > h3, .market-dialog-body > .microcopy, .market-service-disclosure h3, .market-service-disclosure .microcopy");
    targets.forEach(node => { node.textContent = node.textContent?.replace(/插件库/g, "用户分享技能").replace(/插件/g, "技能") || ""; });
    root.querySelectorAll("#plugin-share-form label").forEach(label => { if (label.firstChild?.nodeType === Node.TEXT_NODE) label.firstChild.textContent = label.firstChild.textContent?.replace(/插件/g, "技能") || ""; });
    if (!editing) { const summary = root.querySelector(".market-share-form summary"); if (summary) summary.textContent = "分享我的技能"; }
    root.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>("[placeholder]").forEach(input => input.placeholder = input.placeholder.replace(/插件/g, "技能"));
    const archiveNote = root.querySelector("#share-archive-name");
    if (archiveNote && !files.archive) archiveNote.textContent = editing ? "不选新 ZIP 则保留原技能包，更新时保持技能 name 不变。" : "最多 10 MB，包含 SKILL.md 及脚本、参考资料等文件；SKILL.md 头部须含 name 和 description。";
  };
  const version = ++generation;
  const alive = () => host.isConnected && version === generation;
  let installed: Installed[] = [], rows: Item[] = [], busy = false, reload = false;
  let authorBusy = false;
  host.innerHTML = mine ? `<div class="page-heading"><div><h2>我的分享</h2><p class="muted">提交自己的 Codex 插件，审核通过后向所有用户开放。下载定价 1～100 点，成交所得全部计入作者账户。</p></div><button id="market-refresh" class="button quiet">刷新</button></div><div id="market-mine-summary" class="microcopy"></div><div id="market-feedback" role="status"></div><div id="market-results" class="market-grid"></div>
    <details class="panel market-share-form" open><summary>${editing ? `编辑：${esc(editing.name)}` : "分享我的插件"}</summary>${editing ? `<p class="microcopy">修改后将重新审核，审核通过前暂不公开。累计下载 ${editing.downloads} 次，原有下载次数和购买记录保留。</p>` : ""}<form id="plugin-share-form"><div class="market-form-grid"><label>插件名称<input name="name" maxlength="120" required value="${esc(draft.name)}" placeholder="用户看到的插件名称"></label><label>作者名称<input name="author" maxlength="60" required value="${esc(draft.author)}" placeholder="展示您的昵称"></label></div><label>插件简介<textarea name="description" maxlength="2000" required rows="3" placeholder="介绍插件能做什么，适合哪些场景">${esc(draft.description)}</textarea></label><label>操作说明<textarea name="instructions" maxlength="12000" required rows="5" placeholder="说明安装后的使用步骤、示例，以及是否需要另行设置账户或密钥">${esc(draft.instructions)}</textarea></label><div class="market-form-grid"><label>下载定价（点）<input name="price" type="number" min="1" max="100" step="1" required value="${esc(draft.price)}"></label><label>插件 ZIP<input id="share-archive" type="file" accept=".zip,application/zip"><small id="share-archive-name">${esc(files.archive?.name || (editing ? "不选新 ZIP 则保留原插件包；更新时须保持清单中的 name 不变。" : "最多 10 MB，包含 .codex-plugin/plugin.json"))}</small></label></div>${editing?.screenshots?.length && !screenshotsChanged ? `<div class="share-existing-screenshots">${editing.screenshots.filter(url => /^data:image\/(png|jpeg);base64,[A-Za-z0-9+/=]+$/.test(url) && url.length < 1500000).map(url => `<img src="${url}" alt="已有介绍图">`).join("")}<button id="share-clear-screenshots" class="button quiet" type="button">移除原介绍图</button></div>` : ""}<label>介绍图（可选）<input id="share-screenshots" type="file" accept="image/png,image/jpeg" multiple><small id="share-screenshots-name">${files.screenshots.length ? esc(files.screenshots.map(f => f.name).join("、")) : "最多 3 张 PNG / JPEG，每张不超过 20 MB，上传前自动压缩"}</small></label><div id="share-image-preview" class="share-image-preview"></div><p id="share-image-feedback" role="status" class="microcopy"></p><label class="remember-login"><input type="checkbox" id="share-ownership" required>我有权分享此插件，介绍和价格真实准确，并同意审核后公开发布。</label><div class="provider-actions"><button class="button primary" id="share-submit" type="submit">${editing ? "保存并重新提交审核" : "提交审核"}</button>${editing ? `<button id="share-edit-cancel" class="button quiet" type="button">取消编辑</button>` : ""}<span id="share-feedback" role="status" class="microcopy"></span></div></form></details>`
    : `<div class="page-heading"><div><h2>用户分享</h2><p class="muted">发现社区作者的插件，查看说明后选择安装。付费插件成交所得全部归作者。</p></div><button id="market-refresh" class="button quiet">刷新市场</button></div><form id="market-search-form" class="skills-toolbar"><label class="skill-search"><input id="market-search" type="search" placeholder="搜索插件、用途或作者，按 Enter 搜索" value="${esc(filters.draft)}"></label><select id="market-sort" aria-label="插件排序"><option value="downloads" ${filters.sort === "downloads" ? "selected" : ""}>下载最多</option><option value="new" ${filters.sort === "new" ? "selected" : ""}>最新发布</option><option value="price" ${filters.sort === "price" ? "selected" : ""}>价格从低到高</option></select><button type="submit" class="button outline">搜索</button></form><div id="market-feedback" role="status"></div><div id="market-results" class="market-grid"></div><div id="market-pagination" class="market-pagination"></div>`;
  const refresh = host.querySelector<HTMLButtonElement>("#market-refresh")!;
  const actions = document.createElement("div"); actions.className = "market-heading-actions";
  refresh.before(actions); actions.append(refresh);
  const navigation = document.createElement("button"); navigation.type = "button";
  navigation.id = mine ? "market-back" : "market-share";
  navigation.className = mine ? "button outline" : "button primary";
  navigation.textContent = mine ? "返回插件库" : "我要分享";
  navigation.addEventListener("click", mine ? options.goMarket : options.goShare);
  actions.append(navigation);
  if (kind === "skill" && !mine) navigation.hidden = true;
  const feedback = host.querySelector<HTMLElement>("#market-feedback")!;
  const results = host.querySelector<HTMLElement>("#market-results")!;
  const heading = host.querySelector<HTMLElement>(".page-heading h2")!;
  const title = document.createElement("h1"); title.textContent = mine ? "分享" : "插件库"; heading.replaceWith(title);
  if (!mine && kind === "plugin") {
    host.querySelector<HTMLElement>(".page-heading .muted")!.textContent = "浏览实用插件，查看使用说明与服务费用后，选择安装到 Codex。";
    results.innerHTML = '<div id="builtin-plugins" class="market-builtins" data-view="plugins"></div><div id="market-shares" class="market-shares"><section class="panel market-empty">正在加载用户分享…</section></div>';
    renderPlugins(results.querySelector<HTMLElement>("#builtin-plugins")!, { ...options, compact: true });
  }
  const sharedResults = host.querySelector<HTMLElement>("#market-shares") || results;
  const draw = () => {
    sharedResults.innerHTML = rows.length ? rows.map(item => `<article class="panel market-card">${coverImage(item)}<header class="market-card-heading"><h3>${esc(item.name)}</h3>${sourceBadge()}</header><p class="market-card-description">${esc(item.description)}</p><div class="market-card-meta"><small class="microcopy market-card-author" title="作者：${esc(item.author)}">作者：${esc(item.author)}</small><small class="market-card-version">v${esc(item.version)}</small></div><div class="market-card-controls"><small class="market-authorization-summary">${authorizationName(item)}</small><div class="market-card-actions">${mine ? `<div class="market-author-actions"><span class="plugin-status">${esc(statusName(item.status))}</span><button type="button" class="button outline" data-market-edit="${item.id}">编辑</button>${item.status !== "withdrawn" ? `<button type="button" class="button quiet danger-text" data-market-end="${item.id}">结束分享</button>` : ""}</div>` : `<button class="button outline" data-market-detail="${item.id}">${installed.some(i => i.id === item.id) ? "已安装 · 查看" : "查看插件"}</button>`}</div></div><footer><b>${item.price} 点</b><span class="market-download-count" aria-label="下载 ${Number(item.downloads)} 次"><span aria-hidden="true">↓</span> ${Number(item.downloads)} 次</span></footer>${mine && item.reviewNote ? `<p class="market-review-note">审核说明：${esc(item.reviewNote)}</p>` : ""}</article>`).join("") : "";
    sharedResults.querySelectorAll<HTMLButtonElement>("[data-market-detail]").forEach(b => b.addEventListener("click", () => void showDetail(Number(b.dataset.marketDetail))));
    sharedResults.querySelectorAll<HTMLButtonElement>("[data-market-edit]").forEach(b => b.addEventListener("click", () => void editShared(Number(b.dataset.marketEdit))));
    sharedResults.querySelectorAll<HTMLButtonElement>("[data-market-end]").forEach(b => b.addEventListener("click", () => void endShared(Number(b.dataset.marketEnd))));
    adaptSkillText(host);
  };
  const setAuthorBusy = (value: boolean) => {
    authorBusy = value;
    host.querySelectorAll<HTMLButtonElement>("[data-market-edit], [data-market-end], #share-submit").forEach(b => { b.disabled = value || (b.id === "share-submit" && files.imagePending); });
    host.querySelectorAll<HTMLInputElement>("#share-screenshots, #share-archive").forEach(input => { input.disabled = value; });
  };
  async function editShared(id: number) {
    if (authorBusy) return;
    setAuthorBusy(true);
    try {
      const item = await request<Item>("edit", { id }, await options.getApiKey());
      if (!alive()) return;
      editing = item;
      draft = { name: item.name, author: item.author, description: item.description, instructions: item.instructions || "", price: String(item.price), requiresAuthorization: item.requiresAuthorization === true ? "yes" : item.requiresAuthorization === false ? "no" : "", authorizationDetails: item.authorizationDetails || "", billingDetails: item.billingDetails || "" };
      files = blankFiles(); screenshotsChanged = false;
      renderPluginMarket(host, options, true);
      host.querySelector(".market-share-form")?.scrollIntoView({ block: "start", behavior: "smooth" });
    } catch (e) { if (alive()) { feedback.className = "error-line"; feedback.textContent = message(e); } }
    finally { if (alive()) setAuthorBusy(false); }
  }
  async function endShared(id: number) {
    if (authorBusy || !window.confirm(`确定结束分享此${kind === "skill" ? "技能" : "插件"}？结束后其他用户将无法查看或下载，累计下载次数、收益和购买记录会保留。以后可编辑并重新提交审核。`)) return;
    setAuthorBusy(true);
    try {
      await request("end", { id }, await options.getApiKey());
      if (!alive()) return;
      await load();
      if (alive()) { feedback.className = "microcopy"; feedback.textContent = "已结束分享，历史下载次数和购买记录已保留。"; }
    } catch (e) { if (alive()) { feedback.className = "error-line"; feedback.textContent = message(e); } }
    finally { if (alive()) setAuthorBusy(false); }
  }
  const load = async () => {
    if (busy) { reload = true; return; }
    busy = true; feedback.textContent = "正在加载…"; feedback.className = "microcopy";
    try {
      const [data, local] = await Promise.all([
        mine ? options.getApiKey().then(key => request<{ items: Item[]; revenue: number }>("mine", {}, key)) : request<{ items: Item[]; total: number }>("list", filters),
        request<Installed[]>("installed"),
      ]);
      if (!alive()) return;
      rows = data.items; installed = local; draw(); feedback.textContent = "";
      if (mine) host.querySelector("#market-mine-summary")!.textContent = `累计${kind === "skill" ? "技能" : "插件"}收入：${"revenue" in data ? data.revenue : 0} 点 · 收益已计入当前账户余额`;
      else {
        const total = "total" in data ? data.total : 0, pages = Math.max(1, Math.ceil(total / 20));
        const pager = host.querySelector("#market-pagination")!;
        pager.innerHTML = `<button id="market-prev" class="button outline" ${filters.page === 0 ? "disabled" : ""}>上一页</button><span>第 ${filters.page + 1} / ${pages} 页 · 共 ${total} 个插件</span><button id="market-next" class="button outline" ${filters.page + 1 >= pages ? "disabled" : ""}>下一页</button>`;
        pager.querySelector("#market-prev")!.addEventListener("click", () => { filters.page--; void load(); });
        pager.querySelector("#market-next")!.addEventListener("click", () => { filters.page++; void load(); });
      }
      adaptSkillText(host);
    } catch (e) { if (alive()) { feedback.className = "error-line"; feedback.textContent = message(e); if (!rows.length) sharedResults.innerHTML = '<section class="panel market-empty">用户分享暂时无法加载，请稍后点击刷新重试。</section>'; } }
    finally { busy = false; if (reload && alive()) { reload = false; void load(); } }
  };
  async function showDetail(id: number) {
    const trigger = host.querySelector<HTMLButtonElement>(`[data-market-detail="${id}"]`);
    if (trigger) trigger.disabled = true;
    try {
      const item = await request<Item>("detail", { id });
      if (!alive()) return;
      host.querySelector(".market-modal")?.remove();
      const modal = document.createElement("div"); modal.className = "market-modal";
      modal.innerHTML = `<section class="panel market-dialog" role="dialog" aria-modal="true" aria-labelledby="market-dialog-title"><header><div><div class="builtin-plugin-title"><h2 id="market-dialog-title">${esc(item.name)}</h2>${sourceBadge()}</div><p class="microcopy">${esc(item.author)} · v${esc(item.version)} · 下载 ${item.downloads} 次</p></div><button class="icon-button" data-close aria-label="关闭">×</button></header><div class="market-dialog-body"><h3>插件简介</h3><p class="market-prose">${esc(item.description)}</p>${(item.screenshots || []).filter(url => /^data:image\/(png|jpeg);base64,[A-Za-z0-9+/=]+$/.test(url) && url.length < 1500000).map((url, i) => `<img class="market-screenshot" src="${url}" alt="${esc(item.name)}介绍图 ${i + 1}" loading="lazy">`).join("")}<h3>操作说明</h3><p class="market-prose">${esc(item.instructions)}</p><p class="microcopy">下载后安装到 Codex。插件可包含脚本和外部服务连接，请了解作者说明后再安装。已购插件重复下载安装不再扣点，使用插件所需的第三方服务可能另行收费。</p></div><p id="market-dialog-feedback" role="status" class="microcopy"></p><footer><b>${item.price} 点</b><div class="plugin-actions"><button id="market-remove" class="button outline danger-text" ${installed.some(i => i.id === id) ? "" : "hidden"}>卸载</button><button id="market-install" class="button primary">${!options.codexInstalled ? "前往安装 Codex" : installed.some(i => i.id === id) ? "更新插件" : "购买并安装"}</button></div></footer></section>`;
      modal.querySelector(".market-dialog-body")!.insertAdjacentHTML("beforeend", serviceDisclosure(item));
      host.append(modal);
      adaptSkillText(modal);
      const close = () => { modal.remove(); trigger?.focus(); };
      modal.querySelector("[data-close]")!.addEventListener("click", close);
      modal.addEventListener("click", e => { if (e.target === modal) close(); });
      modal.addEventListener("keydown", e => {
        if (e.key === "Escape") close();
        if (e.key === "Tab") {
          const targets = [...modal.querySelectorAll<HTMLButtonElement>("button:not([disabled]):not([hidden])")];
          const first = targets[0], last = targets[targets.length - 1];
          if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last?.focus(); }
          else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
        }
      });
      (modal.querySelector("[data-close]") as HTMLButtonElement).focus();
      const install = modal.querySelector<HTMLButtonElement>("#market-install")!;
      const remove = modal.querySelector<HTMLButtonElement>("#market-remove")!;
      const note = modal.querySelector<HTMLElement>("#market-dialog-feedback")!;
      let operation = false;
      install.addEventListener("click", async () => {
        if (operation) return;
        if (!options.codexInstalled) { close(); options.goInstall(); return; }
        if (!window.confirm(`确认安装“${item.name}”？首次购买扣除 ${item.price} 点，全部计入作者账户；已购买或作者本人下载不会重复扣点。`)) return;
        operation = true; install.disabled = true; remove.disabled = true; note.className = "microcopy"; note.textContent = "正在下载并安装…";
        try { const key = await options.getApiKey(); if (!alive() || !modal.isConnected) return; await request("install", { id, expectedPrice: item.price }, key); if (!alive() || !modal.isConnected) return; installed = await request<Installed[]>("installed"); if (!alive() || !modal.isConnected) return; note.textContent = "安装完成，请重启 Codex 后使用。"; install.textContent = "更新插件"; remove.hidden = false; draw(); }
        catch (e) { if (modal.isConnected) { note.className = "error-line"; note.textContent = message(e); } }
        finally { operation = false; install.disabled = false; remove.disabled = false; }
      });
      remove.addEventListener("click", async () => {
        if (operation || !window.confirm(`确定卸载“${item.name}”？购买记录会保留，之后可再次安装。`)) return;
        operation = true; install.disabled = true; remove.disabled = true;
        try { await request("uninstall", { id }); if (!alive() || !modal.isConnected) return; installed = await request<Installed[]>("installed"); note.className = "microcopy"; note.textContent = "插件已卸载，请重启 Codex 生效。"; install.textContent = "重新安装"; remove.hidden = true; draw(); }
        catch (e) { note.className = "error-line"; note.textContent = message(e); }
        finally { operation = false; install.disabled = false; remove.disabled = false; }
      });
    } catch (e) { if (alive()) { feedback.className = "error-line"; feedback.textContent = message(e); } }
    finally { if (trigger?.isConnected) trigger.disabled = false; }
  }
  host.querySelector("#market-refresh")!.addEventListener("click", () => void load());
  host.querySelector<HTMLInputElement>("#market-search")?.addEventListener("input", e => { filters.draft = (e.target as HTMLInputElement).value; });
  host.querySelector("#market-search-form")?.addEventListener("submit", e => { e.preventDefault(); filters.q = filters.draft; filters.page = 0; void load(); });
  host.querySelector("#market-sort")?.addEventListener("change", e => { filters.sort = (e.target as HTMLSelectElement).value; filters.page = 0; void load(); });
  const form = host.querySelector<HTMLFormElement>("#plugin-share-form");
  if (form) {
    form.querySelector("#share-edit-cancel")?.addEventListener("click", () => {
      if (authorBusy) return;
      editing = null; draft = blankDraft(draft.author); files = blankFiles(); screenshotsChanged = false;
      renderPluginMarket(host, options, true);
    });
    form.querySelector("#share-clear-screenshots")?.addEventListener("click", () => {
      if (authorBusy) return;
      screenshotsChanged = true; files.screenshots = [];
      form.querySelector(".share-existing-screenshots")?.remove();
      form.querySelector("#share-screenshots-name")!.textContent = "原介绍图将在重新提交时移除，可选择新的介绍图。";
    });
    form.querySelector("#share-ownership")!.closest("label")!.insertAdjacentHTML("beforebegin", authorizationFields());
    const authorization = form.querySelector<HTMLSelectElement>("#share-requires-authorization")!;
    const updateAuthorization = () => {
      const required = authorization.value === "yes";
      form.querySelector<HTMLElement>("#share-authorization-fields")!.hidden = !required;
      const details = form.elements.namedItem("authorizationDetails") as HTMLTextAreaElement;
      details.required = required; details.disabled = !required;
    };
    authorization.addEventListener("change", () => { draft.requiresAuthorization = authorization.value; updateAuthorization(); });
    updateAuthorization();
    form.addEventListener("input", e => { const input = e.target as HTMLInputElement; if (input.name in draft) draft = { ...draft, [input.name]: input.value }; });
    const note = form.querySelector<HTMLElement>("#share-feedback")!;
    const preview = form.querySelector<HTMLElement>("#share-image-preview")!;
    const imageFeedback = form.querySelector<HTMLElement>("#share-image-feedback")!;
    const showImageState = () => {
      const imageOwner = files;
      imageOwner.refresh = () => { if (alive() && files === imageOwner) showImageState(); };
      if (files.previews.length) form.querySelector(".share-existing-screenshots")?.remove();
      form.querySelector("#share-screenshots-name")!.textContent = files.screenshots.map(f => f.name).join("、") || "最多 3 张 PNG / JPEG，每张不超过 20 MB，上传前自动压缩";
      preview.innerHTML = files.previews.map((url, i) => `<figure><img src="${url}" alt="介绍图 ${i + 1}"><figcaption>${esc(files.screenshots[i]?.name)}</figcaption></figure>`).join("") + (files.previews.length || files.imageError ? '<button type="button" class="button quiet" id="share-images-clear">移除所选介绍图</button>' : "");
      imageFeedback.textContent = files.imagePending ? "正在处理介绍图，请稍候…" : files.imageError || (files.previews.length ? `已选择 ${files.previews.length} 张介绍图，第一张将用作卡片封面。` : "");
      imageFeedback.className = files.imageError ? "error-line" : "microcopy";
      form.querySelector<HTMLButtonElement>("#share-submit")!.disabled = files.imagePending || authorBusy;
      preview.querySelector("#share-images-clear")?.addEventListener("click", () => {
        if (authorBusy) return;
        files.imageVersion++; files.imagePending = false;
        files.screenshots = []; files.previews = []; files.imageError = "";
        screenshotsChanged = true;
        (form.querySelector("#share-screenshots") as HTMLInputElement).value = "";
        form.querySelector(".share-existing-screenshots")?.remove();
        showImageState();
      });
    };
    showImageState();
    form.querySelector<HTMLInputElement>("#share-archive")!.addEventListener("change", e => {
      const file = (e.target as HTMLInputElement).files?.[0] || null;
      files.archive = file && file.size <= 10 * 1024 * 1024 && /\.zip$/i.test(file.name) ? file : null;
      form.querySelector("#share-archive-name")!.textContent = files.archive?.name || "请选择不超过 10 MB 的 ZIP 插件包";
    });
    form.querySelector<HTMLInputElement>("#share-screenshots")!.addEventListener("change", async e => {
      const selected = [...((e.target as HTMLInputElement).files || [])];
      const target = files;
      const imageVersion = ++target.imageVersion;
      target.imagePending = true; target.imageError = ""; screenshotsChanged = true; showImageState();
      try {
        if (selected.length > 3) throw new Error("介绍图最多选择 3 张，请重新选择。");
        const prepared = await Promise.all(selected.map(prepareScreenshot));
        const previews = await Promise.all(prepared.map(dataUrl));
        if (target.imageVersion !== imageVersion) return;
        target.screenshots = prepared; target.previews = previews;
        if (alive() && files === target) form.querySelector(".share-existing-screenshots")?.remove();
      } catch (error) { if (target.imageVersion === imageVersion) target.imageError = message(error); }
      finally { if (target.imageVersion === imageVersion) { target.imagePending = false; target.refresh?.(); } }
    });
    let submitting = false;
    form.addEventListener("submit", async e => {
      e.preventDefault(); if (submitting || authorBusy) return;
      if (files.imagePending || files.imageError) { note.textContent = files.imageError || "介绍图仍在处理中，请稍候再提交。"; return; }
      if (!form.reportValidity()) return;
      if (!files.archive && !editing) { note.textContent = "请先选择 ZIP 插件包"; return; }
      const price = Number(draft.price);
      if (!Number.isInteger(price) || price < 1 || price > 100) { note.textContent = "下载定价须为 1～100 的整数点数"; return; }
      const archive = files.archive, images = files.screenshots.slice(), fields = { ...draft }, editingId = editing?.id;
      submitting = true; const button = form.querySelector<HTMLButtonElement>("#share-submit")!; button.disabled = true; note.className = "microcopy"; note.textContent = "正在上传，提交后需等待审核…";
      setAuthorBusy(true);
      try {
        const key = await options.getApiKey();
        const encoded = archive ? await dataUrl(archive) : "", screenshots = await Promise.all(images.map(dataUrl));
        if (!alive()) return;
        await request("upload", { ...fields, ...(editingId ? { id: editingId } : {}), requiresAuthorization: fields.requiresAuthorization === "yes", authorizationDetails: fields.requiresAuthorization === "yes" ? fields.authorizationDetails : "", price,
          ...(archive ? { archiveBase64: encoded.slice(encoded.indexOf(",") + 1) } : {}),
          ...(!editingId || screenshotsChanged ? { screenshots } : {}),
        }, key);
        if (!alive()) return;
        if (editingId) {
          editing = null; draft = blankDraft(fields.author); files = blankFiles(); screenshotsChanged = false;
          renderPluginMarket(host, options, true);
          host.querySelector("#share-feedback")!.textContent = "修改已提交审核，累计下载次数和原有购买记录保留。";
          return;
        }
        draft = blankDraft(fields.author); files = blankFiles();
        screenshotsChanged = false; showImageState();
        note.textContent = "提交成功，审核通过后将出现在插件库。";
        form.reset(); for (const name of ["name", "description", "instructions"]) (form.elements.namedItem(name) as HTMLInputElement).value = "";
        (form.elements.namedItem("author") as HTMLInputElement).value = fields.author;
        (form.elements.namedItem("price") as HTMLInputElement).value = "10";
        authorization.value = "";
        for (const name of ["authorizationDetails", "billingDetails"]) (form.elements.namedItem(name) as HTMLTextAreaElement).value = "";
        updateAuthorization();
        form.querySelector("#share-archive-name")!.textContent = "最多 10 MB，包含 .codex-plugin/plugin.json";
        form.querySelector("#share-screenshots-name")!.textContent = "未选择介绍图";
        await load();
      } catch (error) { if (alive()) { note.className = "error-line"; note.textContent = message(error); } }
      finally { submitting = false; if (alive()) setAuthorBusy(false); button.disabled = false; }
    });
  }
  adaptSkillText(host);
  void load();
}
