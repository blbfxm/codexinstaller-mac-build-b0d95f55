import { invoke } from "@tauri-apps/api/core";
import qrcode from "qrcode-generator";
type Connection = { bound: boolean; owner?: string; running: boolean; workspace: string; writable: boolean; phase: string; error: string; received: number; replied: number; activity: string; hasSession: boolean };
type Readiness = { ready: boolean; message: string };
const escape = (s: unknown) => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
const message = (e: unknown) => typeof e === "string" ? e : e instanceof Error ? e.message : "连接失败，请稍后重试";
async function action<T = Connection>(name: string, payload: Record<string, unknown> = {}): Promise<T> {
  const call = invoke<T>("wechat_action", { action: name, payload });
  const ms = ({ qr: 42_000, preflight: 30_000, pollQr: 40_000, status: 8_000, start: 60_000 } as Record<string, number>)[name];
  if (!ms) return call;
  let timer = 0;
  try { return await Promise.race([call, new Promise<T>((_, reject) => { timer = window.setTimeout(() => reject(new Error(name === "qr" ? "获取微信二维码超时，请检查网络，或切换网络代理后重试。" : name === "preflight" ? "检查 Codex 会话组件超时，请重新检测。" : "读取微信连接状态超时，请重试。")), ms); })]); }
  finally { window.clearTimeout(timer); }
}
let timer = 0, view = 0, cleanup: (() => void) | null = null;
export function stopWeChatView() { ++view; window.clearTimeout(timer); cleanup?.(); cleanup = null; }
export function renderWeChat(host: HTMLElement) {
  stopWeChatView();
  const version = view;
  let connection: Connection | null = null, operation = "", folderBusy = false;
  let readiness: Readiness | null = null, checking = false, localError = "", bridgeError = "";
  let qrMode: "idle" | "loading" | "waiting" = "idle", qrEpoch = 0;
  const alive = () => view === version && host.isConnected && host.dataset.view === "wechat";
  host.innerHTML = `
    <div class="page-heading"><div><h1>微信连接</h1><p class="muted">用个人微信发送任务，在这台电脑上由 Codex 处理并回复。</p></div><span id="wechat-badge" class="wechat-badge">读取状态中…</span></div>
    <div class="wechat-layout">
      <section class="panel wechat-environment" aria-label="Codex 环境检查"><div><h2>Codex 运行环境</h2><p id="wechat-environment-status" role="status">正在检查 Codex 会话组件…</p></div><button id="wechat-recheck" class="button outline" type="button">重新检测</button></section>
      <section class="panel wechat-project">
        <div class="panel-title"><div><h2>1 · 选择工作范围</h2><p>微信任务使用助手中当前生效的供应商和模型。</p></div></div>
        <form id="wechat-form">
          <label for="wechat-workspace">本机项目文件夹</label>
          <div class="wechat-folder"><input id="wechat-workspace" placeholder="请选择本机项目文件夹，或填写完整路径" value="${escape(localStorage.getItem("wechat-workspace") || "")}" required><button id="wechat-folder-select" class="button outline" type="button">选择文件夹</button></div>
          <label class="wechat-write"><input id="wechat-writable" type="checkbox"><span>允许修改项目文件<small>默认只读。勾选后允许 Codex 在所选文件夹内写入文件。</small></span></label>
          <p class="microcopy">仅接收扫码绑定的微信发来的消息。电脑和助手需保持运行；关闭窗口会进入托盘，退出助手会停止连接。</p>
        </form>
      </section>
      <section class="panel wechat-binding">
        <div class="panel-title"><div><h2>2 · 连接个人微信</h2><p id="wechat-owner">选择文件夹后，点击“扫码并开启连接”，在微信确认后自动连接。</p></div></div>
        <div id="wechat-qr" class="wechat-qr"><span>点击下方按钮开始扫码</span></div><p id="wechat-scan-status" class="muted" role="status"></p>
        <div class="wechat-actions"><button id="wechat-start" class="button primary" type="submit" form="wechat-form" disabled>扫码并开启连接</button><button id="wechat-stop" class="button outline" type="button" disabled>停止连接</button><button id="wechat-bind" class="button outline" type="button" hidden>重新扫码绑定</button><button id="wechat-cancel-scan" class="button outline" type="button" hidden>取消扫码</button><button id="wechat-unbind" class="button quiet danger-text" type="button" disabled>解除绑定</button></div>
      </section>
      <section class="panel wechat-activity">
        <div class="panel-title"><div><h2>3 · 发送任务</h2><p>开启连接后，向微信中的对应会话发送文字即可。</p></div></div>
        <div class="wechat-stats"><div><span>运行状态</span><b id="wechat-phase">尚未连接</b></div><div><span>收到消息</span><b id="wechat-received">0</b></div><div><span>已发回复</span><b id="wechat-replied">0</b></div></div>
        <p id="wechat-activity-time" class="microcopy"></p><p id="wechat-error" role="status" class="account-error" hidden></p>
        <div class="wechat-commands"><span><code>/new</code> 开始新会话</span><span><code>/help</code> 查看帮助</span></div>
        <p class="microcopy">支持文字与已转成文字的语音；图片、文件请在电脑端处理。模型请求会使用当前供应商的 API Key，并按供应商规则计费；本功能不会安装或启动 Codex++。</p>
      </section>
    </div>`;
  const query = <T extends HTMLElement>(id: string) => host.querySelector<T>(`#${id}`)!;
  const displayError = () => { if (!alive()) return; const node = query("wechat-error"); node.textContent = localError || bridgeError; node.hidden = !node.textContent; };
  const error = (text: string) => { localError = text; displayError(); };
  const controls = () => {
    if (!alive()) return;
    const running = !!connection?.running;
    query<HTMLButtonElement>("wechat-start").disabled = !!operation || running || checking || !readiness?.ready;
    query<HTMLButtonElement>("wechat-start").textContent = operation === "start" ? "正在开启…" : running ? "已开启微信连接" : qrMode !== "idle" ? "扫码确认后自动连接" : connection?.bound ? "开启微信连接" : "扫码并开启连接";
    query<HTMLButtonElement>("wechat-stop").disabled = !!operation || !running;
    query<HTMLButtonElement>("wechat-bind").disabled = !!operation || running || checking || !readiness?.ready || qrMode === "loading";
    query<HTMLButtonElement>("wechat-bind").hidden = running || (!connection?.bound && qrMode !== "waiting");
    query<HTMLButtonElement>("wechat-bind").textContent = qrMode === "waiting" ? "刷新二维码" : "重新扫码绑定";
    query<HTMLButtonElement>("wechat-recheck").disabled = checking || !!operation || qrMode !== "idle" || running;
    query<HTMLButtonElement>("wechat-recheck").textContent = checking ? "检查中…" : "重新检测";
    query<HTMLButtonElement>("wechat-cancel-scan").hidden = qrMode === "idle";
    query<HTMLButtonElement>("wechat-unbind").disabled = !!operation || !connection?.bound;
    query<HTMLInputElement>("wechat-workspace").disabled = !!operation || running;
    query<HTMLInputElement>("wechat-writable").disabled = !!operation || running;
    query<HTMLButtonElement>("wechat-folder-select").disabled = !!operation || running || folderBusy;
    query<HTMLButtonElement>("wechat-folder-select").textContent = folderBusy ? "选择中…" : "选择文件夹";
  };
  const draw = (data: Connection) => {
    if (!alive()) return;
    connection = data;
    const badge = query("wechat-badge"); badge.textContent = data.running ? "连接已开启" : data.bound ? "已绑定 · 未开启" : "尚未绑定"; badge.classList.toggle("connected", data.running);
    query("wechat-owner").textContent = data.bound ? `已绑定：${data.owner || "个人微信"}，可直接开启连接，无需重复扫码。` : "选择文件夹后，点击“扫码并开启连接”，在微信确认后自动连接。";
    query("wechat-phase").textContent = data.phase || "尚未连接";
    query("wechat-received").textContent = String(data.received);
    query("wechat-replied").textContent = String(data.replied);
    query("wechat-activity-time").textContent = data.activity ? `最近活动 ${data.activity}${data.hasSession ? " · 当前会话可连续对话" : ""}` : "";
    if (data.workspace && !query<HTMLInputElement>("wechat-workspace").value) query<HTMLInputElement>("wechat-workspace").value = data.workspace;
    if (qrMode === "idle") {
      query("wechat-qr").textContent = data.bound ? "✓ 微信已绑定" : "点击下方按钮开始扫码";
      query("wechat-qr").classList.toggle("bound", data.bound);
    }
    if (data.running) { query<HTMLInputElement>("wechat-writable").checked = data.writable; query("wechat-scan-status").textContent = "微信连接已开启，可发送文字任务。"; }
    bridgeError = data.error || ""; displayError();
    controls();
  };
  const checkEnvironment = async () => {
    if (checking || !alive()) return;
    checking = true; controls();
    query("wechat-environment-status").textContent = "正在检查 Codex 会话组件…";
    try { readiness = await action<Readiness>("preflight"); }
    catch (e) { readiness = { ready: false, message: message(e) }; }
    finally {
      checking = false;
      if (alive()) {
        query("wechat-environment-status").textContent = readiness?.message || "环境检查未完成，请重试。";
        query("wechat-environment-status").dataset.ready = String(!!readiness?.ready);
        controls();
      }
    }
  };
  const refresh = async () => {
    if (!alive()) return;
    try { draw(await action("status")); } catch (e) { error(message(e)); }
    if (alive()) timer = window.setTimeout(() => void refresh(), 2000);
  };
  const perform = async (name: string, payload: Record<string, unknown> = {}) => {
    if (operation) return;
    operation = name; error(""); controls();
    try { draw(await action(name, payload)); }
    catch (e) { error(message(e)); if (name === "start") void checkEnvironment(); }
    finally { operation = ""; controls(); }
  };
  const cancelScan = () => {
    ++qrEpoch; qrMode = "idle";
    void action("cancelQr").catch(() => {});
    controls();
  };
  cleanup = () => { if (qrMode !== "idle") cancelScan(); };
  const start = async () => {
    if (!alive() || operation || checking || !readiness?.ready || connection?.running) return;
    const workspace = query<HTMLInputElement>("wechat-workspace").value.trim();
    if (!workspace) { error("请先选择工作文件夹，或直接填写文件夹的完整路径。"); query<HTMLInputElement>("wechat-workspace").focus(); return; }
    localStorage.setItem("wechat-workspace", workspace);
    if (!connection?.bound) { if (qrMode === "idle") void bind(); else query("wechat-scan-status").textContent = "请用微信扫码确认，之后将自动开启连接。"; return; }
    if (qrMode !== "idle") cancelScan();
    await perform("start", { workspace, writable: query<HTMLInputElement>("wechat-writable").checked });
  };
  const bind = async () => {
    if (!alive() || operation || checking || !readiness?.ready || connection?.running || qrMode === "loading") return;
    const workspace = query<HTMLInputElement>("wechat-workspace").value.trim();
    if (!workspace) { error("请先选择工作文件夹，或直接填写文件夹的完整路径。"); query<HTMLInputElement>("wechat-workspace").focus(); return; }
    localStorage.setItem("wechat-workspace", workspace);
    if (qrMode === "waiting") { ++qrEpoch; await action("cancelQr").catch(() => {}); }
    const epoch = ++qrEpoch, current = () => alive() && epoch === qrEpoch;
    qrMode = "loading"; error(""); controls();
    query("wechat-qr").classList.remove("bound"); query("wechat-qr").textContent = "正在准备二维码…";
    const scan = query("wechat-scan-status"); scan.textContent = "正在检查环境并获取微信二维码…";
    try {
      const { content } = await action<{ content: string }>("qr", { workspace }); if (!current()) return;
      const code = qrcode(0, "M"); code.addData(content, "Byte"); code.make();
      query("wechat-qr").innerHTML = code.createSvgTag({ scalable: true, margin: 4 });
      qrMode = "waiting"; controls(); scan.textContent = "请用微信扫码，并在手机上确认。";
      let confirmed = false;
      const deadline = Date.now() + 300_000;
      while (current() && Date.now() < deadline) {
        const result = await action<{ status: string }>("pollQr"); if (!current()) return;
        if (result.status === "confirmed") { confirmed = true; qrMode = "idle"; query("wechat-qr").textContent = "✓ 微信已绑定"; scan.textContent = "微信已绑定，正在开启连接…"; draw(await action("status")); break; }
        if (result.status === "expired") { scan.textContent = "二维码已过期，请重新获取。"; break; }
        if (result.status.startsWith("scan")) scan.textContent = "已扫码，请在手机微信上确认。";
        await new Promise(resolve => window.setTimeout(resolve, 1200));
      }
      if (current() && !confirmed && Date.now() >= deadline) scan.textContent = "二维码已过期，请重新获取。";
      if (current() && confirmed) await start();
    } catch (e) {
      if (current()) { scan.textContent = "未能完成连接，请查看下方提示后重试。"; error(message(e)); void action("cancelQr").catch(() => {}); }
    } finally { if (current()) { qrMode = "idle"; controls(); } }
  };
  query("wechat-form").addEventListener("submit", event => { event.preventDefault(); if (!operation) void start(); });
  query("wechat-stop").addEventListener("click", () => void perform("stop"));
  query("wechat-unbind").addEventListener("click", () => {
    if (!window.confirm("解除微信绑定并停止当前连接？再次使用需要重新扫码。")) return;
    cancelScan(); query("wechat-qr").textContent = "点击下方按钮开始扫码"; query("wechat-scan-status").textContent = "";
    void perform("unbind");
  });
  query("wechat-folder-select").addEventListener("click", async () => {
    if (folderBusy) return;
    folderBusy = true; controls(); error("");
    try { const { path } = await action<{ path: string }>("chooseFolder"); if (alive() && path) { query<HTMLInputElement>("wechat-workspace").value = path; localStorage.setItem("wechat-workspace", path); } }
    catch (e) { error(message(e)); }
    finally { folderBusy = false; controls(); }
  });
  query("wechat-bind").addEventListener("click", () => void bind());
  query("wechat-cancel-scan").addEventListener("click", () => { cancelScan(); query("wechat-qr").textContent = connection?.bound ? "✓ 微信已绑定" : "点击下方按钮开始扫码"; query("wechat-qr").classList.toggle("bound", !!connection?.bound); query("wechat-scan-status").textContent = "已取消扫码，可以重新开始连接。"; });
  query("wechat-recheck").addEventListener("click", () => { error(""); void checkEnvironment(); });
  controls(); void refresh(); void checkEnvironment();
}
