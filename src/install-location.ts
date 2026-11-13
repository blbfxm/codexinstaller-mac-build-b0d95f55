import { invoke } from "@tauri-apps/api/core";

type Options = { path: string; volumes: { path: string; label: string }[] };
let cached: Options | null = null;
let loading: Promise<Options> | null = null;
let saving: Promise<string> | null = null;
const escape = (text: string) => text.replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

export function installLocationPanel() {
  return `<section id="codex-install-location" class="codex-install-location" aria-label="Codex 安装位置"></section>`;
}

export async function waitForInstallLocationSave() {
  if (saving) await saving;
}

export function bindInstallLocation(isNative: boolean, busy: () => boolean) {
  const host = document.querySelector<HTMLElement>("#codex-install-location");
  if (!host) return;
  const draw = (error = "") => {
    if (!host.isConnected) return;
    const unavailable = !!cached?.path && !cached.volumes.some(v => v.path === cached!.path);
    host.innerHTML = `<label for="codex-install-path">Codex 安装位置</label><div class="install-location-controls"><select id="codex-install-path" ${busy() || saving || !cached || !isNative ? "disabled" : ""}>
      <option value="" ${!cached?.path ? "selected" : ""}>系统默认位置</option>
      ${unavailable ? `<option selected value="${escape(cached!.path)}">${escape(cached!.path)}（当前不可用）</option>` : ""}
      ${(cached?.volumes || []).map(v => `<option value="${escape(v.path)}" ${v.path === cached?.path ? "selected" : ""}>${escape(v.path)} · ${escape(v.label)}</option>`).join("")}
      </select><button type="button" class="button quiet" id="codex-install-location-refresh" ${busy() || saving || !isNative ? "disabled" : ""}>刷新磁盘</button></div>
      <small>自动保存，适用于一键安装和单独安装。官方安装包仅支持选择磁盘，目录由 Windows 管理；执行安装时，已安装的 Codex 若不在所选磁盘，将备份数据、卸载旧包并在所选磁盘重新安装。</small>
      <p class="install-location-status ${error ? "danger-text" : ""}" role="status">${escape(error || (saving ? "正在保存安装位置…" : cached ? (unavailable ? "所选磁盘不可用，请重新选择后安装。" : cached.path ? `已保存：${cached.path}` : "使用 Windows 默认安装位置。") : isNative ? "正在读取安装位置…" : "请在桌面程序中设置安装位置。"))}</p>`;
    host.querySelector<HTMLSelectElement>("#codex-install-path")!.addEventListener("change", async event => {
      if (busy() || saving) return;
      const path = (event.currentTarget as HTMLSelectElement).value;
      saving = invoke<string>("save_codex_install_location", { path });
      draw();
      let message = "";
      try { const saved = await saving; if (cached) cached.path = saved; }
      catch (e) { message = typeof e === "string" ? e : "保存安装位置失败，请重试"; }
      finally { saving = null; draw(message); }
    });
    host.querySelector("#codex-install-location-refresh")!.addEventListener("click", () => { cached = null; void load(); });
  };
  const load = async () => {
    draw();
    if (!isNative) return;
    let error = "";
    try {
      loading ||= invoke<Options>("codex_install_options");
      const result = await loading;
      // Older mocked backends can omit this optional UI command.
      cached = result || { path: "", volumes: [] };
    } catch (e) { error = typeof e === "string" ? e : "读取安装位置失败，请刷新磁盘后重试"; }
    finally { loading = null; draw(error); }
  };
  if (cached) draw(); else void load();
  if (saving) void saving.then(() => draw(), () => draw());
}
