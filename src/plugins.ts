import { invoke } from "@tauri-apps/api/core";

type Status = { installed: boolean; enabled: boolean; credentialReady: boolean; version: string; baseUrl: string };
type Options = { codexInstalled: boolean; getApiKey: () => Promise<string>; goInstall: () => void; embedded?: boolean; compact?: boolean };
const message = (e: unknown) => typeof e === "string" ? e : e instanceof Error ? e.message : "插件操作失败，请稍后重试";
let view = 0;
const pending = new Map<string, Promise<Status>>();
const plugins = [
  { id: "image", title: "文生图", icon: "✧", status: "image_plugin_status", install: "install_image_plugin", description: "描述您想要的画面，生成图片或结合参考图创作。", features: ["文生图", "参考图创作", "比例与尺寸", "透明背景", "图片保存到本机"], example: "@文生图  使用 nano-banana-2 模型，生成一张 1:1 的产品海报。", hint: "使用时请告诉 Codex 要用哪个生图模型；不清楚有哪些模型时，可以先让它查看可用模型和价格。", billing: "安装插件免费，生成图片会从当前坚果账户余额中扣点，费用取决于所选模型。使用前可让 Codex 查询价格，余额不足时请到个人中心充值。", upload: "使用参考图时，图片会上传到坚果平台处理。生成的图片可保存到电脑；支持的尺寸和透明背景等功能，以所选模型为准。" },
  { id: "vector", title: "位图转矢量图", icon: "◇", status: "vector_plugin_status", install: "install_vector_plugin", description: "将 PNG、JPEG 等图片转换为可编辑的 SVG 矢量图。", features: ["PNG / JPEG / WebP", "AI 专业转矢量", "任务进度查询", "SVG 本机保存"], example: "@位图转矢量图 将项目中的 logo.png 转成 SVG，保存为 logo.svg。", hint: "告诉 Codex 图片位置和保存位置即可。转换后得到的 SVG 矢量图可导入设计软件继续编辑，放大后仍保持清晰。", billing: "安装插件免费，转换图片会从当前坚果账户余额中扣点，按坚果平台的转矢量价格收费。使用前可让 Codex 查询价格，余额不足时请到个人中心充值。", upload: "待转换的图片会上传到坚果平台处理。完成后将 SVG 矢量图保存到电脑，可用支持 SVG 的设计软件打开和编辑。" },
  { id: "video", title: "文生视频", icon: "▶", status: "video_plugin_status", install: "install_video_plugin", description: "按描述生成 MP4 视频，支持横屏、竖屏、时长和多档分辨率。", features: ["三款视频模型", "横屏 / 竖屏", "多档分辨率", "时长与参考素材", "MP4 本机保存"], example: "@文生视频 使用 wan-3.0 模型，生成一段 6 秒的横屏产品展示视频，保存到当前项目。", hint: "使用时请告诉 Codex 要用哪个视频模型、画面比例和时长；不清楚有哪些模型或价格时，可以先让它查看可用模型。视频生成通常比图片更久，请保留任务编号。", billing: "安装插件免费，视频按所选模型、分辨率和时长按秒计费，生成成功后从当前坚果账户余额扣点。提交时可能预留额度；失败或违规通常不扣费。使用前可让 Codex 查询实时价格。", upload: "参考图和参考音频会上传到坚果平台处理。成片会保存为您指定的 MP4 文件；支持的模型、分辨率、时长和参考素材以平台返回为准。" },
];

export function renderPlugins(host: HTMLElement, options: Options) {
  const version = ++view;
  const alive = () => version === view && host.isConnected && host.dataset.view === "plugins";
  host.innerHTML = (options.compact ? "" : `<div class="page-heading"><div>${options.embedded ? "<h2>内置插件</h2>" : "<h1>插件</h1>"}<p class="muted">为 Codex 安装扩展能力，使用当前账户的坚果 API Key。</p></div><button id="plugins-refresh" class="button quiet" type="button">刷新状态</button></div>`) + plugins.map(p => `
    <section class="panel builtin-plugin"><div class="builtin-plugin-heading"><span class="builtin-plugin-icon" aria-hidden="true">${p.icon}</span><div><div class="builtin-plugin-title"><h2>${p.title}</h2><span id="${p.id}-plugin-badge" class="plugin-status" role="status">检测中…</span></div><p class="muted">${p.description}</p></div></div>
    <button type="button" class="plugin-details plugin-help" data-plugin-help="${p.id}" aria-haspopup="dialog">使用说明与费用 ↗</button><template id="${p.id}-plugin-help-content"><div class="plugin-features">${p.features.map(f => `<span>${f}</span>`).join("")}</div>
    <div class="plugin-usage"><h3>安装后这样使用</h3><p>在 Codex 聊天框中输入 <strong>@</strong>，选择“${p.title}”，再输入您的要求并发送。</p><p>例如：</p><blockquote>“${p.example}”</blockquote><p class="microcopy">${p.hint}</p></div>
    <ul><li>插件使用您在本使用助手中登录的坚果账户，自动关联该账户的 API Key（卡密），无需在 Codex 对话中再次填写。</li><li>${p.billing}</li><li>${p.upload}</li><li>创作处理需要一些时间，请耐心等待。若网络中断，可让 Codex 继续查看刚才的处理结果，避免重复发起而再次扣点。</li><li>安装或更新后，请重新启动 Codex，再按上方示例使用。更换登录账户后，插件会自动使用新账户；如提示账户信息需要更新，请点击“更新插件”。</li><li>点击“更新插件”可安装本使用助手提供的最新版本。卸载插件不会删除已经生成并保存的文件。</li></ul></template>
    <footer class="plugin-footer"><div><p id="${p.id}-plugin-feedback" role="status" class="plugin-feedback"></p></div><div class="plugin-actions">${!options.codexInstalled ? `<button data-plugin-go-install class="button outline" type="button">前往安装 Codex</button>` : ""}<button id="${p.id}-plugin-uninstall" class="button outline danger-text" type="button" hidden>卸载</button><button id="${p.id}-plugin-install" class="button primary" type="button" disabled>检测中…</button></div></footer></section>`).join("");
  host.querySelectorAll<HTMLButtonElement>("[data-plugin-help]").forEach(trigger => trigger.addEventListener("click", () => {
    const plugin = plugins.find(p => p.id === trigger.dataset.pluginHelp)!;
    const source = host.querySelector<HTMLTemplateElement>(`#${plugin.id}-plugin-help-content`)!;
    const dialog = document.createElement("dialog"); dialog.className = "builtin-plugin-dialog";
    dialog.setAttribute("aria-labelledby", "plugin-help-title");
    dialog.innerHTML = `<header><div><h2 id="plugin-help-title">${plugin.title}</h2><p class="microcopy">使用说明与费用</p></div><button type="button" class="icon-button" data-plugin-help-close aria-label="关闭使用说明">×</button></header><div class="plugin-help-body"></div><footer><button type="button" class="button primary" data-plugin-help-close>我知道了</button></footer>`;
    dialog.querySelector(".plugin-help-body")!.append(source.content.cloneNode(true));
    dialog.querySelectorAll("[data-plugin-help-close]").forEach(button => button.addEventListener("click", () => dialog.close()));
    dialog.addEventListener("click", event => {
      if (event.target !== dialog) return;
      const rect = dialog.getBoundingClientRect();
      if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) dialog.close();
    });
    dialog.addEventListener("close", () => { dialog.remove(); if (trigger.isConnected) trigger.focus(); }, { once: true });
    host.append(dialog); dialog.showModal();
  }));
  const loaders: (() => Promise<void>)[] = [];
  for (const p of plugins) {
    let status: Status | null = null, busy = !!pending.get(p.id);
    let operation = "正在配置…";
    const button = host.querySelector<HTMLButtonElement>(`#${p.id}-plugin-install`)!;
    const uninstall = host.querySelector<HTMLButtonElement>(`#${p.id}-plugin-uninstall`)!;
    const feedback = host.querySelector<HTMLElement>(`#${p.id}-plugin-feedback`)!;
    const draw = () => {
      if (!alive()) return;
      const installed = status?.installed && status?.enabled;
      const badge = host.querySelector<HTMLElement>(`#${p.id}-plugin-badge`)!;
      badge.textContent = installed ? status?.credentialReady ? "已安装" : "待更新凭证" : status?.installed ? "未启用" : "未安装";
      badge.classList.toggle("is-installed", !!installed && !!status?.credentialReady);
      button.disabled = busy || !options.codexInstalled;
      button.textContent = busy ? operation : !options.codexInstalled ? "请先安装 Codex" : status?.installed || status?.enabled ? "更新插件" : "安装到 Codex";
      uninstall.hidden = !(status?.installed || status?.enabled);
      uninstall.disabled = busy;
    };
    const load = async () => {
      if (busy && !pending.get(p.id)) return;
      busy = true;
      try { if (pending.get(p.id)) await pending.get(p.id); const result = await invoke<Status>(p.status); if (!alive()) return; status = result; feedback.textContent = ""; feedback.classList.remove("is-error"); }
      catch (e) { if (alive()) { feedback.textContent = message(e); feedback.classList.add("is-error"); } }
      finally { busy = false; draw(); }
    };
    loaders.push(load);
    button.addEventListener("click", async () => {
      if (busy || !options.codexInstalled) return;
      const updating = !!(status?.installed || status?.enabled);
      operation = updating ? "正在更新…" : "正在安装…";
      busy = true; feedback.textContent = ""; feedback.classList.remove("is-error"); draw();
      const install = options.getApiKey().then(apiKey => invoke<Status>(p.install, { apiKey, baseUrl: "https://api.jianguo.ltd" }));
      pending.set(p.id, install);
      try { const result = await install; if (!alive()) return; status = result; feedback.textContent = `插件已${updating ? "更新" : "安装"}，已关联当前坚果账户，请重新启动 Codex 后使用。`; }
      catch (e) { if (alive()) { feedback.textContent = message(e); feedback.classList.add("is-error"); } }
      finally { if (pending.get(p.id) === install) pending.delete(p.id); busy = false; draw(); }
    });
    uninstall.addEventListener("click", async () => {
      if (busy || !window.confirm(`确定卸载“${p.title}”？已生成并保存的文件会保留。`)) return;
      operation = "正在卸载…"; busy = true; feedback.textContent = ""; feedback.classList.remove("is-error"); draw();
      const removal = invoke<Status>("uninstall_builtin_plugin", { plugin: p.id });
      pending.set(p.id, removal);
      try { const result = await removal; if (!alive()) return; status = result; feedback.textContent = "插件已卸载，请重新启动 Codex 使更改生效。"; }
      catch (e) { if (alive()) { feedback.textContent = message(e); feedback.classList.add("is-error"); } }
      finally { if (pending.get(p.id) === removal) pending.delete(p.id); busy = false; draw(); }
    });
    void load();
  }
  host.querySelector("#plugins-refresh")?.addEventListener("click", () => { void Promise.allSettled(loaders.map(load => load())); });
  host.querySelectorAll("[data-plugin-go-install]").forEach(button => button.addEventListener("click", options.goInstall));
}
