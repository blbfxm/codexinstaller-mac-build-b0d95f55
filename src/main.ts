import { invoke } from "@tauri-apps/api/core";
import { installLocationPanel, bindInstallLocation, waitForInstallLocationSave } from "./install-location";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { renderWallet, resetWallet } from "./wallet";
import { showCodexResetDialog, type CodexResetReport } from "./codex-reset";
import { accountRequest, renderRegistration, type AccountResult } from "./registration";
import { renderUsage } from "./usage";
import { renderWeChat, stopWeChatView } from "./wechat";
import { renderPluginMarket, resetPluginMarket } from "./plugin-market";
import { renderPublicPluginMarket, resetPublicPluginMarket } from "./public-plugin-market";
import { finishCodexSetup, type CodexSetupState } from "./finish-codex-setup";
import { confirmDialog } from "./confirm-dialog";
import { createElement, Download, Globe, Settings, RefreshCw, LogOut, Check, ArrowRight, KeyRound, Monitor, Sun, Moon, Play, Pencil, Trash2, Circle, Search, Sparkles, ShieldCheck, Eye, EyeOff, LoaderCircle, User, Wallet, Info, Power, MessagesSquare } from "lucide";

const uiIcons = { Download, Globe, Settings, RefreshCw, LogOut, Check, ArrowRight, KeyRound, Monitor, Sun, Moon, Play, Pencil, Trash2, Circle, Search, Sparkles, ShieldCheck, Eye, EyeOff, LoaderCircle, User, Wallet, Info, Power, MessagesSquare };
const icon = (name: keyof typeof uiIcons) => createElement(uiIcons[name], { width: 18, height: 18, "aria-hidden": "true" }).outerHTML;

type ProductKey = "codex" | "codexpp";
type Product = { key: ProductKey; name: string; detail: string; size: string; bundled: boolean; ready: boolean; checked: boolean; running: boolean; path: string };
type InstalledStatus = { installed: boolean; path: string; version: string };
type InstallationDetectionResult = { codex: InstalledStatus; codexpp: InstalledStatus };
type AppConfig = Record<string, string>;
type CodexProvider = { id: string; name: string; baseUrl: string; model: string; models: string[]; apiKey: string; hasApiKey: boolean; isActive: boolean; isCurrent?: boolean; isGateway?: boolean; protocol?: string; relayMode?: string };
type ProviderHealth = { state: "checking" | "online" | "offline"; message: string };
type PageKey = "install" | "proxy" | "config" | "skills" | "plugins" | "account" | "wechat";
type SkillItem = { id: string; source: string; name: string; display_name: string; description: string; version: string; author: string; tags: string[]; license: string; path: string; tree: string; bytes: number; skills: number; requires_auth: boolean; category: string; name_zh: string; description_zh: string; installed: boolean };
type ProxySubscription = { id: string; remarks: string; url: string; enabled: boolean; updateTime: number };
type ProxyProfile = {
  indexId: string; remarks: string; protocol: string; address: string; port: number;
  delay: number; isCurrent: boolean; subscriptionId?: string;
};
type ProxyListener = { protocols: string[]; listenAddress: string; port: number };
type RoutingProfile = { id: string; remarks: string; isActive: boolean };
type SystemProxyMode = "auto" | "clear" | "unchanged" | "pac";
type ProxyState = { ready: boolean; running: boolean; systemProxySupported: boolean; systemProxyEnabled: boolean; addingSubscription: boolean; subscriptions: ProxySubscription[]; profiles: ProxyProfile[]; routingProfiles: RoutingProfile[]; selectedRoute: string; systemProxyMode: SystemProxyMode; selectedSubscription: string; socksAddress: string; httpAddress: string; message: string };
type LoginResult = {
  ok: boolean;
  message?: string;
  account?: { id: number; username?: string; email?: string; mainKami?: string; mainKamiMasked?: string; hasApiKey?: boolean };
  installerConfig?: AppConfig;
};
type DownloadProgress = { product: string; phase?: "download" | "cached" | "install" | "install-wait"; downloaded: number; total: number | null; bytes_per_second: number; resumable: boolean; status?: string };
type TaskProgress = { title: string; percent: string; detail: string; width: string; failed: boolean };
type ProviderConfiguration = { name: string; baseUrl: string; model: string; models: string[]; apiKey: string; providerId: string | null; protocol: string; relayMode: string; configureCodexAccess: boolean; configureCodexChinese: boolean; configureCodexppChinese: boolean; configureCodexppStartCodex: boolean };
type UpstreamModels = { models: string[]; defaultModel: string | null };
type ProxyBillingResult = { allowed: boolean; message?: string; balance: number; bytesUsed: number; billedUnits: number; dailyPrice: number; usageDate: string };

const state = {
  busy: false,
  installTaskActive: false,
  taskProgress: null as TaskProgress | null,
  taskProgressDocked: false,
  proxyOperationBusy: false,
  proxyLoading: false,
  proxyLoadingMessage: "",
  downloadSlowSince: 0,
  downloadSuggestionVisible: false,
  downloadActive: false,
  downloadPaused: false,
  downloadCancelled: false,
  lastDownloadProgress: null as DownloadProgress | null,
  taskWasCancelled: false,
  theme: (localStorage.getItem("codex-usage-theme") === "light" ? "light" : "dark") as "dark" | "light",
  setupMessage: "",
  setupFailed: false,
  page: "install" as PageKey,
  platform: navigator.userAgent.includes("Mac") ? "macos" : "windows",
  arch: navigator.userAgent.includes("ARM") ? "arm64" : "x64",
  installMode: "silent" as "silent" | "interactive",
  includeCodexpp: false,
  includeCodex: true,
  setupEnableProxy: false,
  setupConfigureNut: true,
  setupAfterAgreement: false,
  configureCodexAccess: true,
  configureCodexChinese: true,
  configureCodexppChinese: true,
  configureCodexppStartCodex: true,
  fullLaunch: false,
  fullLaunchStarting: false,
  fullLaunchLaunchAfterAgreement: false,
  setupProxyOpenedForTask: false,
  setupProxyCoreWasRunning: false,
  proxyView: "nodes" as "nodes" | "subscriptions" | "settings",
  setupEditor: false,
  loginMode: "kami" as "kami" | "password",
  selectedProvider: "nut" as "nut" | "custom",
  providerApiKey: "",
  providerModels: [] as string[],
  providers: [] as CodexProvider[],
  providerHealth: {} as Record<string, ProviderHealth>,
  skills: [] as SkillItem[],
  libraryTab: "plugins" as "skills" | "plugins" | "shares",
  pluginSection: "market" as "market" | "community",
  skillSection: "public" as "public" | "community" | "shares",
  libraryOpen: false,
  skillsLoaded: false,
  skillsLoading: false,
  skillsError: "",
  skillsNetworkError: false,
  installingSkills: new Set<string>(),
  skillSearch: "",
  skillSearchDraft: "",
  skillCategory: "全部",
  skillSort: "popular" as "popular" | "downloads" | "name",
  providersLoaded: false,
  editingProviderId: "",
  providerEditorOpen: false,
  providerCatalogOpen: false,
  providerSearch: "",
  providerPreset: null as ProviderPreset | null,
  providerAccessMode: "pureApi" as "pureApi" | "officialMixApi" | "gateway",
  providerProtocol: "responses" as "responses" | "chatCompletions",
  rememberLogin: true,
  autoLoginAttempted: false,
  personal: {
    balance: null as number | null,
    loading: false,
    error: "",
    loaded: false,
    revealLoginKami: false,
    section: "info" as "info" | "recharge" | "usage",
    keyBusy: false,
    rechargeMethod: "online" as "online" | "card",
  },
  proxy: {
    ready: false,
    running: false,
    systemProxySupported: false,
    systemProxyEnabled: false,
    addingSubscription: false,
    subscriptions: [] as ProxySubscription[],
    profiles: [] as ProxyProfile[],
    routingProfiles: [] as RoutingProfile[],
    selectedRoute: "",
    systemProxyMode: (localStorage.getItem("system-proxy-mode") as SystemProxyMode) || "auto",
    selectedSubscription: "",
    socksAddress: "未监听",
    httpAddress: "未监听",
    message: "",
  } as ProxyState,
  config: {} as AppConfig,
  account: null as LoginResult["account"] | null,
  nutSubscriptionKami: "",
  nutProxyCheckTimer: 0 as number,
  proxyBilling: { active: false, subscriptionId: "", localBaselineBytes: 0, serverBaselineBytes: 0, usageDate: "", timer: 0 as number },
  proxyAgreementOpen: false,
  proxyAgreementAcceptedThisRun: false,
  proxyStartAfterAgreement: false,
  proxySubscriptionInitialized: false,
  testingProxyProfileId: "",
  products: [
    { key: "codex", name: "Codex", detail: "官方桌面客户端", bundled: false, ready: false, checked: false, running: false, path: "" },
    { key: "codexpp", name: "Codex++", detail: "可选的网关与增强工具", bundled: false, ready: false, checked: false, running: false, path: "" },
  ] as Product[],
};

type ProviderPreset = { name: string; baseUrl: string; model: string; protocol: "responses" | "chatCompletions"; category: "account" | "model"; note?: string };
const providerPresets: ProviderPreset[] = [
  { name: "OpenAI Official", baseUrl: "", model: "", protocol: "responses", category: "account", note: "chatgpt.com" },
  { name: "MiniMax (Global)", baseUrl: "https://api.minimax.io/v1", model: "MiniMax-M2.5", protocol: "chatCompletions", category: "account", note: "platform.minimax.io" },
  { name: "DeepSeek", baseUrl: "https://api.deepseek.com", model: "deepseek-chat", protocol: "responses", category: "model", note: "platform.deepseek.com" },
  { name: "智谱 GLM", baseUrl: "https://open.bigmodel.cn/api/paas/v4", model: "glm-4.7", protocol: "chatCompletions", category: "model", note: "open.bigmodel.cn" },
  { name: "Kimi", baseUrl: "https://api.moonshot.cn/v1", model: "kimi-k2.5", protocol: "responses", category: "model", note: "platform.moonshot.cn" },
  { name: "百炼 (Qwen)", baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1", model: "qwen3.5-plus", protocol: "chatCompletions", category: "model", note: "bailian.console.aliyun.com" },
  { name: "StepFun", baseUrl: "https://api.stepfun.com/v1", model: "step-3.5-flash", protocol: "chatCompletions", category: "model", note: "platform.stepfun.com" },
  { name: "MiniMax (China)", baseUrl: "https://api.minimaxi.com/v1", model: "MiniMax-M2.5", protocol: "chatCompletions", category: "model", note: "platform.minimaxi.com" },
  { name: "火山引擎", baseUrl: "https://ark.cn-beijing.volces.com/api/v3", model: "", protocol: "chatCompletions", category: "model", note: "火山方舟" },
  { name: "小米 MiMo", baseUrl: "https://api.xiaomimimo.com/v1", model: "mimo-v2-flash", protocol: "chatCompletions", category: "model", note: "小米开放平台" },
];
const app = document.querySelector<HTMLDivElement>("#app")!;
const isNative = () => "__TAURI_INTERNALS__" in window;
const API_HOST = "https://api.jianguo.ltd";
const escapeHtml = (value: unknown) => String(value ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

document.documentElement.dataset.theme = state.theme;
const themeColor = document.querySelector<HTMLMetaElement>('meta[name="theme-color"]');
if (themeColor) themeColor.content = state.theme === "dark" ? "#171b1b" : "#f6f7f8";

function renderLogin() {
  app.innerHTML = `<main class="login-shell"><section class="login-panel"><div class="brand-mark">J</div><p class="eyebrow">CODEX WORKSTATION</p><h1>Codex 使用助手</h1><p class="muted intro">登录后安装、配置并使用 Codex 与 Codex++。</p><form id="login-form" class="login-form"><div class="segmented login-modes"><button type="button" data-login-mode="kami" class="${state.loginMode === "kami" ? "selected" : ""}">卡密登录</button><button type="button" data-login-mode="password" class="${state.loginMode === "password" ? "selected" : ""}">账号密码</button></div><div id="kami-login-fields" ${state.loginMode === "kami" ? "" : "hidden"}><label for="login-kami">卡密</label><input id="login-kami" name="kami" autocomplete="off" placeholder="输入坚果卡密" ${state.loginMode === "kami" ? "required" : ""}/><label class="remember-login"><input id="remember-login" type="checkbox" ${state.rememberLogin ? "checked" : ""}/>记住卡密并自动登录</label></div><div id="password-login-fields" ${state.loginMode === "password" ? "" : "hidden"}><label for="username">用户名或邮箱</label><input id="username" name="username" autocomplete="username" placeholder="由管理后台创建" ${state.loginMode === "password" ? "required" : ""}/><label for="password">密码</label><input id="password" name="password" type="password" autocomplete="current-password" placeholder="输入账户密码" ${state.loginMode === "password" ? "required" : ""}/></div><p id="login-error" class="error-line" aria-live="polite"></p><button class="button primary full" type="submit">登录并继续 <span>→</span></button></form><p class="fine-print">自动登录卡密保存在 Windows DPAPI 或 macOS 钥匙串中；退出登录后清除。</p></section><aside class="login-aside"><div class="aside-top"><span class="status-dot"></span> Codex 使用助手 <span class="version">PREVIEW</span></div><div class="aside-content"><span class="section-kicker">CODEX WORKSTATION</span><h2>把安装、代理与配置<br/>收进一个清晰流程。</h2><div class="mini-steps"><span>01</span><div><b>检查设备</b><small>匹配系统与架构</small></div></div><div class="mini-steps"><span>02</span><div><b>准备网络</b><small>按需加载授权订阅</small></div></div><div class="mini-steps"><span>03</span><div><b>完成配置</b><small>备份、写入并验证</small></div></div></div><div class="aside-footer">Windows · macOS <span>坚果工具</span></div></aside></main>`;
  const registrationLink = document.createElement("button");
  registrationLink.type = "button"; registrationLink.className = "button quiet full register-entry";
  registrationLink.textContent = "还没有账户？注册用户账号";
  registrationLink.addEventListener("click", () => renderRegistration(document.querySelector<HTMLElement>(".login-panel")!, renderLogin, async result => {
    state.loginMode = "password";
    await completeAccountLogin(result, "");
  }));
  document.querySelector("#login-form")!.after(registrationLink);
  document.querySelector<HTMLInputElement>("#username")!.placeholder = "用户名或邮箱";
  const kamiInput = document.querySelector<HTMLInputElement>("#login-kami")!;
  kamiInput.type = "password";
  kamiInput.spellcheck = false;
  const kamiControl = document.createElement("div");
  kamiControl.className = "login-kami-control";
  kamiInput.before(kamiControl);
  const visibilityButton = document.createElement("button");
  visibilityButton.id = "login-kami-visibility";
  visibilityButton.type = "button";
  visibilityButton.className = "icon-button";
  visibilityButton.setAttribute("aria-controls", "login-kami");
  const updateVisibilityButton = () => {
    const visible = kamiInput.type === "text";
    visibilityButton.title = visible ? "隐藏卡密" : "显示卡密";
    visibilityButton.setAttribute("aria-label", visibilityButton.title);
    visibilityButton.setAttribute("aria-pressed", String(visible));
    visibilityButton.innerHTML = icon(visible ? "EyeOff" : "Eye");
  };
  visibilityButton.addEventListener("click", () => {
    kamiInput.type = kamiInput.type === "password" ? "text" : "password";
    updateVisibilityButton();
  });
  updateVisibilityButton();
  kamiControl.append(kamiInput, visibilityButton);
  document.querySelector<HTMLFormElement>("#login-form")!.addEventListener("submit", onLogin);
  document.querySelector<HTMLInputElement>("#remember-login")?.addEventListener("change", event => {
    state.rememberLogin = (event.currentTarget as HTMLInputElement).checked;
  });
  document.querySelectorAll<HTMLButtonElement>("[data-login-mode]").forEach(button => button.addEventListener("click", () => {
    state.loginMode = button.dataset.loginMode as "kami" | "password";
    renderLogin();
  }));
}

async function onLogin(event: SubmitEvent) {
  event.preventDefault();
  const form = new FormData(event.currentTarget as HTMLFormElement);
  const kami = String(form.get("kami") ?? "").trim();
  const username = String(form.get("username") ?? "").trim();
  let password = String(form.get("password") ?? "");
  const error = document.querySelector<HTMLElement>("#login-error")!;
  const button = document.querySelector<HTMLButtonElement>("#login-form button[type=submit]")!;
  button.disabled = true; button.textContent = "正在验证…"; error.textContent = "";
  try {
    const passwordHash = state.loginMode === "password" ? await sha256(password) : "";
    password = "";
    document.querySelector<HTMLInputElement>("#password")!.value = "";
    const loginRequest = {
      loginType: state.loginMode,
      kami: state.loginMode === "kami" ? kami : "",
      username: state.loginMode === "password" ? username : "",
      passwordHash,
    };
    let result: LoginResult;
    if (isNative()) result = state.loginMode === "password" ? await accountRequest("login", { username, passwordHash }) : await invoke<LoginResult>("account_login", { baseUrl: API_HOST, ...loginRequest });
    else {
      const response = await fetch(`${API_HOST}/api/v1/installer/login`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(loginRequest) });
      result = await response.json() as LoginResult;
    }
    if (!result.ok || !result.account) throw new Error(result.message || "登录信息不正确");
    await completeAccountLogin(result, kami);
  } catch (cause) {
    error.textContent = errorText(cause, "登录失败，请检查账号、密码和网络");
    button.disabled = false; button.innerHTML = '登录并继续 <span>→</span>';
  }
}

async function completeAccountLogin(result: AccountResult, kami: string) {
  if (!result.ok || !result.account) throw new Error(result.message || "登录失败");
    resetPluginMarket(); resetPublicPluginMarket();
    accountSessionGeneration++;
    state.account = {
      id: result.account.id,
      username: result.account.username,
      mainKamiMasked: result.account.mainKamiMasked,
      email: result.account.email,
      hasApiKey: result.account.hasApiKey,
    };
    state.nutSubscriptionKami = state.loginMode === "kami" ? kami : result.apiKey || result.account.mainKami || "";
    state.personal.balance = typeof result.balance === "number" ? result.balance : null;
    state.config = result.installerConfig || {};
    await loadSiteConfig();
    if (isNative()) {
      try {
        if (state.loginMode === "kami" && state.rememberLogin) await invoke("save_auto_login_kami", { kami });
        else await invoke("clear_auto_login_kami");
      } catch (error) {
        toast(errorText(error, "安全保存自动登录卡密失败"), true);
      }
      const device = await invoke<{ platform: string; arch: string }>("device_info");
      state.platform = device.platform; state.arch = device.arch;
      await checkInstallations();
    }
    state.personal.loaded = false;
    renderDashboard();
    await syncBuiltinPluginCredentials();
}

async function syncBuiltinPluginCredentials() {
  if (!isNative()) return;
  try { await invoke("sync_builtin_plugin_credentials", { apiKey: state.nutSubscriptionKami }); }
  catch (error) { toast(errorText(error, "插件账户凭证同步失败，请到插件页更新凭证后使用"), true); }
}

async function sha256(value: string) {
  const bytes = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return [...new Uint8Array(bytes)].map(byte => byte.toString(16).padStart(2, "0")).join("");
}

async function loadSiteConfig() {
  try {
    const publicConfig = isNative()
      ? await invoke<AppConfig>("fetch_site_config", { baseUrl: API_HOST })
      : await fetch(`${API_HOST}/api/v1/site-config`).then(response => response.ok ? response.json() as Promise<AppConfig> : state.config);
    state.config = { ...state.config, ...publicConfig };
  } catch { /* Installer settings are optional until the backend endpoint is available. */ }
}

function renderDashboard() {
  const platform = state.platform === "macos" ? "macOS" : "Windows";
  const arch = state.arch.toUpperCase();
  app.innerHTML = `<div class="app-shell"><header class="topbar"><a class="brand" href="#"><span class="brand-mark small">J</span><span>Codex<span class="brand-light">使用助手</span></span></a><div class="topbar-right"><span class="device-chip"><i></i>${platform} · ${arch}</span><span class="account-name">${escapeHtml(state.account?.username || state.account?.mainKamiMasked || "已登录")}</span><button id="theme-toggle" class="icon-button" title="${state.theme === "dark" ? "切换亮色主题" : "切换暗色主题"}" aria-label="${state.theme === "dark" ? "切换亮色主题" : "切换亮色主题"}">${icon(state.theme === "dark" ? "Sun" : "Moon")}</button><button id="logout" class="icon-button" title="退出登录" aria-label="退出登录">↪</button></div></header><div class="workspace"><nav class="sidebar"><div class="nav-label">工作区</div><button class="nav-item ${state.page === "install" ? "active" : ""}" data-page="install"><span class="nav-icon">▦</span> 安装</button><button class="nav-item ${state.page === "proxy" ? "active" : ""}" data-page="proxy"><span class="nav-icon">↗</span> 网络代理</button><button class="nav-item ${state.page === "config" ? "active" : ""}" data-page="config"><span class="nav-icon">⚙</span> 配置</button><button class="nav-item ${state.page === "wechat" ? "active" : ""}" data-page="wechat">微信连接</button><button class="nav-item ${state.page === "plugins" ? "active" : ""}" data-page="plugins">技能与插件</button><button class="nav-item ${state.page === "account" ? "active" : ""}" data-page="account"><span class="nav-icon">◎</span> 个人中心</button><div class="sidebar-bottom"><span class="sidebar-version">Codex 使用助手 · 0.1.0</span></div></nav><main class="content"><div id="page-content"></div><section class="progress-panel" id="progress-panel" hidden><div class="progress-head"><b id="progress-title"></b><span id="progress-percent"></span><button id="progress-close" class="icon-button" aria-label="关闭任务提示" title="关闭" hidden>×</button></div><div class="progress-track"><span id="progress-bar"></span></div><p id="progress-detail"></p><details id="progress-error-details" class="progress-error-details" hidden><summary>查看详细错误</summary><pre></pre></details><div class="progress-actions"><button id="download-pause-toggle" class="button quiet" ${state.downloadActive ? "" : "hidden"}>${state.downloadPaused ? "继续下载" : "暂停下载"}</button><button id="download-stop" class="button quiet danger-text" ${state.busy ? "" : "hidden"}>停止下载</button><button id="download-proxy-suggestion" class="button quiet download-proxy-suggestion" hidden>下载较慢，去选择节点并开启网络代理</button></div></section><div id="toast" class="toast" role="status" aria-live="polite"></div></main></div><div id="proxy-agreement" class="agreement-backdrop" role="presentation" hidden><section class="agreement-dialog" role="dialog" aria-modal="true" aria-labelledby="proxy-agreement-title"><header><h2 id="proxy-agreement-title">网络代理服务说明与合规须知</h2><button id="proxy-agreement-close" class="icon-button" aria-label="关闭" title="暂不使用">×</button></header><div class="agreement-copy"></div><footer><button id="proxy-agreement-cancel" class="button quiet">暂不使用</button><button id="proxy-agreement-accept" class="button primary">我已阅读并同意</button></footer></section></div></div>`;
  const agreementCopy = document.querySelector<HTMLElement>(".agreement-copy");
  if (agreementCopy) agreementCopy.innerHTML = `
    <p><b>服务说明：</b>本功能基于开源 v2rayN 项目提供网络连接能力，仅用于 Codex 简体中文语言包的自动下载与切换、Codex 及其必要组件的安装与更新，以及保障 Codex 正常、完整使用所必需的相关网络连接。连接质量、速度和可用性受网络环境、节点状态及第三方服务影响。</p>
    <p><b>限定用途与合规要求：</b>本代理仅限用于 Codex 简体中文语言包的自动下载与切换、Codex 及其必要组件的安装与更新，以及保障 Codex 正常、完整使用所必需的相关网络连接；不得用于任何其他目的或行为，包括访问与上述用途无关的网站、服务或内容。您须遵守适用法律法规，不得利用本服务从事违法违规、侵害他人合法权益或危害网络与数据安全的活动。使用者应对自身行为依法承担相应责任；如发现超出限定用途或违反法律法规的使用，服务提供方有权采取限制、暂停或终止服务等措施。</p>
    <p><b>账号与安全：</b>卡密仅供您本人按授权范围使用，请妥善保管，不得转借、转售或以其他方式提供给他人。发现账号异常、卡密泄露或疑似未经授权使用时，请立即停止使用并联系服务提供方。</p>
    <p><b>服务与风险：</b>服务提供方可因维护、故障、节点异常、安全风险、法律法规或主管部门要求，采取通知、限制、暂停或终止服务等必要措施。代理服务不保证持续可用、特定速度、特定节点或访问结果。请勿通过代理传输您无权处理的信息，或在不可信网站提交密码、支付信息及其他敏感资料。</p>
    <p><b>信息处理与责任：</b>为提供服务、安全保障及故障处理，平台可能在法律允许范围内处理必要的账号、连接、流量和安全运行信息，并依法采取保护措施。您对自身使用行为及提交、传输内容承担法律规定的责任；服务提供方依法承担其相应义务，任何条款均不免除法律规定不得免除的责任。</p>
    <p class="agreement-note">点击“我已阅读并同意”或继续使用本工具，即表示您已充分阅读、理解并自愿同意以上所有条款。如不同意，请点击“暂不使用”并停止使用网络代理服务。</p>`;
  document.querySelector("#progress-close")!.addEventListener("click", () => {
    state.taskProgressDocked = !state.taskProgressDocked;
    renderTaskProgress();
  });
  document.querySelector("#logout")!.addEventListener("click", () => { void logoutAssistant(); });
  document.querySelector<HTMLButtonElement>("#theme-toggle")!.addEventListener("click", () => {
    state.theme = state.theme === "dark" ? "light" : "dark";
    localStorage.setItem("codex-usage-theme", state.theme);
    document.documentElement.dataset.theme = state.theme;
    const themeColor = document.querySelector<HTMLMetaElement>('meta[name="theme-color"]');
    if (themeColor) themeColor.content = state.theme === "dark" ? "#171b1b" : "#f6f7f8";
    renderDashboard();
  });
  document.querySelector<HTMLButtonElement>("#download-proxy-suggestion")!.addEventListener("click", () => {
    if (!state.proxyAgreementAcceptedThisRun) {
      openProxyAgreement();
      return;
    }
    state.page = "proxy";
    renderDashboard();
    void loadProxyState();
  });
  document.querySelector<HTMLButtonElement>("#download-pause-toggle")?.addEventListener("click", () => {
    void toggleDownloadPause();
  });
  document.querySelector<HTMLButtonElement>("#download-stop")?.addEventListener("click", () => {
    void stopDownload();
  });
  document.querySelectorAll<HTMLButtonElement>("[data-page]").forEach(button => button.addEventListener("click", () => {
    const page = button.dataset.page as PageKey;
    if (page === "plugins") { state.libraryOpen = !state.libraryOpen; syncLibraryNavigation(); return; }
    if (page === "proxy") {
      if (state.proxyAgreementAcceptedThisRun) {
        state.page = "proxy";
        renderDashboard();
        void loadProxyState();
        return;
      }
      // Opening the proxy agreement is independent from the one-click setup flow.
      // Clear any stale setup continuation so accepting the proxy terms cannot
      // accidentally resume installation.
      state.setupAfterAgreement = false;
      state.proxyStartAfterAgreement = false;
      state.fullLaunchLaunchAfterAgreement = false;
      openProxyAgreement();
      return;
    }
    state.page = page;
    renderDashboard();
    if (page === "account" && !state.personal.loaded) void refreshPersonalCenter();
  }));
  for (const selector of ["#proxy-agreement-close", "#proxy-agreement-cancel"]) {
    document.querySelector(selector)!.addEventListener("click", () => {
      state.proxyStartAfterAgreement = false;
      state.setupAfterAgreement = false;
      state.fullLaunchLaunchAfterAgreement = false;
      closeProxyAgreement();
      if (state.page === "install") renderPage();
    });
  }
  document.querySelector("#proxy-agreement-accept")!.addEventListener("click", () => {
    state.proxyAgreementAcceptedThisRun = true;
    closeProxyAgreement();
    if (state.fullLaunchLaunchAfterAgreement) {
      state.fullLaunchLaunchAfterAgreement = false;
      state.page = "install";
      renderDashboard();
      void startOrRestartProduct("codex");
      return;
    }
    if (state.page === "install" && state.setupAfterAgreement) {
      state.setupAfterAgreement = false;
      void beginSetup();
      return;
    }
    state.page = "proxy";
    renderDashboard();
    if (state.proxyStartAfterAgreement) {
      state.proxyStartAfterAgreement = false;
      void handleTrayProxyAction(true);
    } else {
      void loadProxyState();
    }
  });
  renderPage();
  renderTaskProgress();
  if (state.installTaskActive && state.lastDownloadProgress) updateDownloadProgress(state.lastDownloadProgress);
  const downloadStop = document.querySelector<HTMLButtonElement>("#download-stop");
  if (downloadStop) downloadStop.hidden = !state.downloadActive;
  if (isNative() && !state.providersLoaded) void loadCodexProviders();
  document.querySelector(".nav-label")!.textContent = "工作区";
  const navigation: [PageKey, keyof typeof uiIcons, string][] = [["install", "Monitor", "安装与开始"], ["config", "Settings", "供应商配置"], ["proxy", "Globe", "网络代理"], ["wechat", "MessagesSquare", "微信连接"], ["plugins", "Sparkles", "技能与插件"], ["account", "KeyRound", "个人中心"]];
  const sidebar = document.querySelector<HTMLElement>(".sidebar")!;
  const workspaceNavigation = document.createElement("div");
  workspaceNavigation.className = "workspace-navigation";
  const accountNavigation = document.createElement("div");
  accountNavigation.className = "account-navigation";
  for (const [page, glyph, label] of navigation) {
    const button = document.querySelector<HTMLButtonElement>(`[data-page="${page}"]`);
    if (button) {
      button.innerHTML = `${icon(glyph)}<span>${label}</span>`;
      button.setAttribute("aria-label", label);
      button.title = label;
      button.setAttribute("aria-current", page === state.page ? "page" : "false");
      (page === "account" ? accountNavigation : workspaceNavigation).append(button);
    }
  }
  sidebar.querySelector(".nav-label")!.after(workspaceNavigation);
  sidebar.querySelector(".sidebar-bottom")!.before(accountNavigation);
  const libraryParent = sidebar.querySelector<HTMLButtonElement>('[data-page="plugins"]')!;
  libraryParent.id = "library-toggle";
  libraryParent.setAttribute("aria-controls", "library-submenu");
  libraryParent.innerHTML += '<span class="library-chevron" aria-hidden="true">›</span>';
  const submenu = document.createElement("div");
  submenu.id = "library-submenu"; submenu.className = "library-submenu";
  submenu.setAttribute("role", "group"); submenu.setAttribute("aria-label", "技能与插件子菜单");
  submenu.innerHTML = ([["skills", "技能库"], ["plugins", "插件库"]] as const).map(([tab, label]) => `<button type="button" class="nav-subitem" data-library-tab="${tab}">${label}</button>`).join("");
  libraryParent.after(submenu);
  submenu.querySelectorAll<HTMLButtonElement>("[data-library-tab]").forEach(button => {
    button.addEventListener("click", () => {
      state.page = "plugins"; state.libraryTab = button.dataset.libraryTab as typeof state.libraryTab; state.libraryOpen = true;
      syncLibraryNavigation(); renderPage();
    });
    button.addEventListener("keydown", event => {
      const children = [...submenu.querySelectorAll<HTMLButtonElement>("button")];
      if (event.key === "ArrowLeft") { event.preventDefault(); state.libraryOpen = false; syncLibraryNavigation(); libraryParent.focus(); }
      else if (["ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) {
        event.preventDefault(); const index = children.indexOf(button);
        children[event.key === "Home" ? 0 : event.key === "End" ? children.length - 1 : (index + (event.key === "ArrowDown" ? 1 : children.length - 1)) % children.length]?.focus();
      }
    });
  });
  libraryParent.addEventListener("keydown", event => {
    if (!["ArrowRight", "ArrowLeft", "ArrowDown"].includes(event.key)) return;
    event.preventDefault(); state.libraryOpen = event.key !== "ArrowLeft"; syncLibraryNavigation();
    if (state.libraryOpen) submenu.querySelector<HTMLButtonElement>(`[data-library-tab="${state.libraryTab === "shares" ? "plugins" : state.libraryTab}"]`)?.focus();
  });
  syncLibraryNavigation();
  const logout = document.querySelector<HTMLButtonElement>("#logout");
  if (logout) { logout.innerHTML = icon("LogOut"); logout.title = "退出登录"; }
}

function syncLibraryNavigation() {
  const parent = document.querySelector<HTMLButtonElement>("#library-toggle");
  const submenu = document.querySelector<HTMLElement>("#library-submenu");
  if (!parent || !submenu) return;
  parent.setAttribute("aria-expanded", String(state.libraryOpen));
  parent.setAttribute("aria-current", "false");
  submenu.hidden = !state.libraryOpen;
  document.querySelectorAll<HTMLButtonElement>(".sidebar [data-page]").forEach(button => {
    const active = button.dataset.page === state.page;
    button.classList.toggle("active", active);
    if (button !== parent) button.setAttribute("aria-current", active ? "page" : "false");
  });
  submenu.querySelectorAll<HTMLButtonElement>("[data-library-tab]").forEach(button => {
    const active = state.page === "plugins" && button.dataset.libraryTab === (state.libraryTab === "shares" ? "plugins" : state.libraryTab);
    button.classList.toggle("active", active); button.setAttribute("aria-current", active ? "page" : "false");
  });
}

function openProxyAgreement() {
  const modal = document.querySelector<HTMLElement>("#proxy-agreement");
  if (modal) modal.hidden = false;
}

function closeProxyAgreement() {
  const modal = document.querySelector<HTMLElement>("#proxy-agreement");
  if (modal) modal.hidden = true;
}

function renderPage() {
  const content = document.querySelector<HTMLElement>("#page-content")!;
  content.dataset.view = state.page;
  content.classList.remove("provider-editing");
  stopWeChatView();
  if (state.page === "wechat") { renderWeChat(content); return; }
  if (state.page === "install") {
    const platform = state.platform === "macos" ? "macOS" : "Windows";
    const arch = state.arch.toUpperCase();
    const targets = state.products.filter(product => product.key === "codex" ? state.includeCodex : state.includeCodexpp);
    const installed = targets.filter(product => product.checked && product.ready).length;
    const softwareReady = targets.length > 0 && installed === targets.length;
    const usableProviders = state.providers.filter(provider => state.providerHealth[provider.id]?.state === "online");
    const activeProvider = state.providers.find(provider => provider.isActive);
    const currentProvider = state.providers.find(provider => provider.isCurrent);
    const providerStartupNotice = currentProvider
      ? `当前上游：${currentProvider.name}`
      : activeProvider?.isGateway
        ? "当前连接 Codex++ 网关，上游暂未识别"
        : activeProvider
          ? `当前供应商：${activeProvider.name}${state.providerHealth[activeProvider.id]?.state === "offline" ? "（连接异常）" : ""}`
          : "尚未启用供应商";
    const providerSummary = state.providersLoaded
      ? `${providerStartupNotice}${usableProviders.length ? ` · ${usableProviders.length} 个供应商连接正常` : ""}`
      : "正在读取供应商…";
    const codexReady = state.products.find(product => product.key === "codex")?.ready || false;
    const codexRunning = state.products.find(product => product.key === "codex")?.running || false;
    const launchCodexButton = (() => {
      const codex = state.products.find(product => product.key === "codex")!;
      const disabled = !codex.ready || state.busy;
      return `<button class="button quiet product-launch" data-launch-product="codex" ${disabled || state.fullLaunchStarting ? "disabled" : ""} title="${codex.ready ? "启动或重启 Codex" : "请先安装 Codex"}">${icon(codex.running ? "RefreshCw" : "Play")}${state.fullLaunchStarting ? "正在准备…" : `${codex.running ? "重启" : "启动"} Codex`}</button>`;
    })();
    const stopCodexButton = codexReady && codexRunning
      ? `<button class="button quiet" id="stop-codex" type="button" ${state.busy ? "disabled" : ""}>${icon("Power")}关闭 Codex</button>`
      : "";
    content.innerHTML = `<div class="page-heading"><div><h1>安装与开始</h1><p class="muted">准备环境，启动 Codex。</p></div><span class="environment-label">${icon("Monitor")}${platform} · ${arch}</span></div>
      <section class="setup-workflow" aria-labelledby="setup-title"><div class="section-heading"><div><h2 id="setup-title">一键安装与配置</h2><p class="muted">${softwareReady ? "已安装的软件会核对安装位置，位置一致时跳过安装。" : "按所选项目安装软件并应用配置。"}</p></div><span class="workflow-status">${targets.length ? `${installed} / ${targets.length} 已安装` : "自定义操作"}</span></div>
      <div class="workflow-actions"><button id="install-all" class="button primary">${icon("Download")}一键安装并配置</button></div>
      <details class="setup-options-disclosure"><summary>安装配置选项</summary><fieldset class="setup-options" ${state.busy || state.setupAfterAgreement ? "disabled" : ""}><label><input id="include-codex" type="checkbox" ${state.includeCodex ? "checked" : ""}><span><b>安装 Codex</b><small>已安装且位置一致时跳过；更换磁盘时备份数据后重装。</small></span></label><div class="setup-proxy-option"><label><input id="setup-enable-proxy" type="checkbox" ${state.setupEnableProxy ? "checked" : ""}><span><b>安装期间开启网络代理</b><small>仅供下载安装使用，任务结束后自动关闭。</small></span></label><details class="setup-tip"><summary aria-label="网络代理下载提示" aria-describedby="setup-proxy-tip">${icon("Info")}</summary><span id="setup-proxy-tip" role="tooltip">如果下载速度慢，可以开启网络代理试试；如果开启后速度更慢，请不要勾选。安装后不是中文界面时，也可以勾选此项重新安装配置。</span></details></div><div class="setup-proxy-option"><label><input id="setup-configure-nut" type="checkbox" ${state.setupConfigureNut ? "checked" : ""}><span><b>配置坚果中转服务</b><small>登录卡密将作为 API Key；余额可在个人中心查看或充值。</small></span></label><details class="setup-tip"><summary aria-label="坚果中转服务提示" aria-describedby="setup-nut-tip">${icon("Info")}</summary><span id="setup-nut-tip" role="tooltip">每个登录卡密就是坚果中转服务的 API Key。您可以在“个人中心”查看可用余额或充值。</span></details></div><label><input id="include-codexpp" type="checkbox" ${state.includeCodexpp ? "checked" : ""}><span><b>安装 Codex++（可选）</b><small>需要网关分流或其他增强功能时再安装。</small></span></label><label><input id="option-codex-access" type="checkbox" ${state.configureCodexAccess ? "checked" : ""}><span><b>启用 Codex 完整访问权限</b><small>允许访问文件并执行操作，无需逐次确认。</small></span></label><label><input id="option-codex-chinese" type="checkbox" ${state.configureCodexChinese ? "checked" : ""}><span><b>设置 Codex 简体中文</b><small>将 Codex 界面语言设为简体中文。</small></span></label><label><input id="option-codexpp-chinese" type="checkbox" ${state.configureCodexppChinese ? "checked" : ""}><span><b>设置 Codex 强制中文界面</b><small>通过助手启动 Codex 时使用简体中文界面。</small></span></label>${state.products.find(product => product.key === "codexpp")?.ready ? `<label><input id="option-codexpp-start-codex" type="checkbox" ${state.configureCodexppStartCodex ? "checked" : ""}><span><b>打开 Codex++ 时自动启动 Codex</b><small>仅在手动启动 Codex++ 时生效。</small></span></label>` : ""}</fieldset>${state.platform === "windows" ? installLocationPanel() : ""}</details>
      <div class="setup-result ${state.setupFailed ? "error" : ""}" role="status" ${state.setupMessage ? "" : "hidden"}>${escapeHtml(state.setupMessage)}</div>
      </section>
      <section class="startup-workspace" aria-labelledby="startup-title"><div class="section-heading"><div><h2 id="startup-title">启动</h2><p class="muted">${codexReady ? "启动或关闭 Codex" : "安装 Codex 后可在此启动"}</p></div></div><div class="startup-actions">${launchCodexButton}${stopCodexButton}<div class="setup-proxy-option startup-full-launch-option"><label class="startup-full-launch" ${state.busy || state.fullLaunchStarting ? "aria-disabled=\"true\"" : ""}><input id="setup-full-launch" type="checkbox" ${state.fullLaunch ? "checked" : ""} ${state.busy || state.fullLaunchStarting ? "disabled" : ""}><span>满配启动</span></label><details class="setup-tip"><summary aria-label="满配启动计费说明" aria-describedby="full-launch-tip">${icon("Info")}</summary><span id="full-launch-tip" role="tooltip">勾选后启动 Codex 时自动开启网络代理。网络代理按每天 5 点计费，每日流量上限为 30G。</span></details></div></div></section>
      <section class="software-workspace" aria-labelledby="software-title"><div class="section-heading"><div><h2 id="software-title">本机软件</h2><p class="muted">安装、启动与维护</p></div><button class="button quiet" id="refresh" title="重新检测安装状态">${icon("RefreshCw")}<span>重新检测</span></button></div><div class="product-list">${state.products.map(productRow).join("")}</div></section>
      <section class="provider-context"><div>${icon("Settings")}<div><b>供应商</b><p>${escapeHtml(providerSummary)}</p>${usableProviders.length ? `<small>可用：${escapeHtml(usableProviders.map(provider => provider.name).join("、"))}</small>` : ""}</div></div><button id="setup-configure" class="button quiet">${icon("ArrowRight")}管理供应商</button></section>
      <details class="advanced-settings"><summary>安装选项与设备信息</summary><section class="install-mode-panel"><div><b>Codex++ 安装方式</b><small>默认自动安装，也可手动选择安装步骤。</small></div><div class="segmented"><button data-mode="silent" class="${state.installMode === "silent" ? "selected" : ""}">自动安装</button><button data-mode="interactive" class="${state.installMode === "interactive" ? "selected" : ""}">手动安装</button></div></section><div class="device-facts"><div><small>系统</small><b>${platform} ${arch}</b></div><div><small>服务连接</small><b id="network-state">检测中</b></div><div><small>安装配置</small><b id="config-state">${Object.keys(state.config).length ? "已获取" : "待获取"}</b></div></div></details>
      `;
    bindInstallLocation(isNative(), () => state.busy || state.setupAfterAgreement);
    document.querySelector("#setup-configure")!.addEventListener("click", () => { state.page = "config"; renderDashboard(); });
    document.querySelectorAll<HTMLButtonElement>("[data-launch-product]").forEach(button => {
      button.addEventListener("click", () => void startOrRestartProduct(button.dataset.launchProduct as ProductKey));
    });
    document.querySelector("#stop-codex")?.addEventListener("click", async event => {
      const button = event.currentTarget as HTMLButtonElement;
      button.disabled = true;
      try {
        await invoke("stop_codex_processes");
        const codex = state.products.find(item => item.key === "codex");
        if (codex) codex.running = false;
        renderPage();
        toast("Codex 已关闭");
      } catch (error) {
        toast(errorText(error, "关闭 Codex 失败"), true);
        if (button.isConnected) button.disabled = false;
      }
    });
    document.querySelector("#refresh")!.addEventListener("click", async () => {
      const button = document.querySelector<HTMLButtonElement>("#refresh");
      if (button) button.disabled = true;
      try { await checkInstallations(); await loadSiteConfig(); checkNetwork(); renderPage(); toast("已重新检测安装状态"); }
      catch (error) { toast(error instanceof Error ? error.message : "检测安装状态失败", true); }
      finally { if (button?.isConnected) button.disabled = false; }
    });
    document.querySelectorAll<HTMLButtonElement>(".install-one").forEach(button => button.addEventListener("click", () => installProduct(button.dataset.key as ProductKey)));
    document.querySelectorAll<HTMLButtonElement>(".more-button").forEach(button => button.addEventListener("click", event => {
      event.stopPropagation();
      const menu = button.parentElement?.querySelector<HTMLElement>(".product-menu");
      const open = menu?.hidden === false;
      closeProductMenus();
      if (menu) menu.hidden = open;
      button.closest(".product-row")?.classList.toggle("menu-open", Boolean(menu && !open));
      button.setAttribute("aria-expanded", String(Boolean(menu && !open)));
    }));
    document.querySelectorAll<HTMLButtonElement>("[data-product-action]").forEach(button => button.addEventListener("click", async () => {
      const key = button.dataset.key as ProductKey;
      const action = button.dataset.productAction;
      closeProductMenus();
      // Paint the closed menu before a native confirmation can block the UI.
      await new Promise<void>(resolve => requestAnimationFrame(() => setTimeout(resolve, 0)));
      if (action === "reinstall") void installProduct(key, true);
      if (action === "uninstall") void uninstallProduct(key);
      if (action === "store-install") void installProduct(key, true, "store");
      if (action === "reset" && key === "codex") openCodexResetDialog();
      if (action === "localize" && key === "codex") void localizeInstalledCodex();
    }));
    document.onclick = event => {
      if (event.target instanceof Element && event.target.closest(".product-actions")) return;
      closeProductMenus();
    };
    const installAllButton = document.querySelector<HTMLButtonElement>("#install-all")!;
    installAllButton.disabled = state.busy || state.setupAfterAgreement || (state.setupConfigureNut && !state.providersLoaded) || !(state.includeCodex || state.includeCodexpp || state.setupEnableProxy || state.setupConfigureNut || state.fullLaunch);
    installAllButton.innerHTML = `${icon("Download")}${state.busy ? "正在安装并配置…" : "一键安装并配置"}`;
    installAllButton.addEventListener("click", () => void beginSetup());
    for (const [id, key] of [["include-codex", "includeCodex"], ["include-codexpp", "includeCodexpp"], ["setup-enable-proxy", "setupEnableProxy"], ["setup-configure-nut", "setupConfigureNut"]] as const) {
      document.querySelector<HTMLInputElement>(`#${id}`)!.addEventListener("change", event => {
        state[key] = (event.currentTarget as HTMLInputElement).checked;
        state.setupMessage = "";
        state.setupFailed = false;
        const optionsOpen = document.querySelector<HTMLDetailsElement>(".setup-options-disclosure")?.open || false;
        renderPage();
        if (optionsOpen) document.querySelector<HTMLDetailsElement>(".setup-options-disclosure")!.open = true;
      });
    }
    for (const [id, key] of [["option-codex-access", "configureCodexAccess"], ["option-codex-chinese", "configureCodexChinese"], ["option-codexpp-chinese", "configureCodexppChinese"], ["option-codexpp-start-codex", "configureCodexppStartCodex"], ["setup-full-launch", "fullLaunch"]] as const) {
      document.querySelector<HTMLInputElement>(`#${id}`)?.addEventListener("change", event => {
        state[key] = (event.currentTarget as HTMLInputElement).checked;
        state.setupMessage = "";
        state.setupFailed = false;
      });
    }
    document.querySelectorAll<HTMLButtonElement>("[data-mode]").forEach(button => button.addEventListener("click", () => { state.installMode = button.dataset.mode as "silent" | "interactive"; renderPage(); }));
    checkNetwork();
    return;
  }
  if (state.page === "account") {
    renderAccountPage(content);
    return;
  }
  if (state.page === "skills" || state.page === "plugins") {
    if (state.page === "skills") { state.page = "plugins"; state.libraryTab = "skills"; }
    content.innerHTML = '<section id="library-body" aria-label="技能与插件内容"></section>';
    const body = content.querySelector<HTMLElement>("#library-body")!;
    const options = {
      codexInstalled: !!state.products.find(product => product.key === "codex")?.ready,
      getApiKey: ensureAccountApiKey,
      goShare: () => { if (state.libraryTab === "skills") state.skillSection = "shares"; else state.libraryTab = "shares"; syncLibraryNavigation(); renderPage(); document.querySelector<HTMLElement>(".content")?.scrollTo(0, 0); },
      goMarket: () => { if (state.libraryTab === "skills") state.skillSection = "community"; else state.libraryTab = "plugins"; syncLibraryNavigation(); renderPage(); document.querySelector<HTMLElement>(".content")?.scrollTo(0, 0); },
      goInstall: () => document.querySelector<HTMLButtonElement>('[data-page="install"]')?.click(),
    };
    if (state.libraryTab === "skills") {
      body.innerHTML = `<nav class="skill-source-tabs" aria-label="技能来源"><button class="button ${state.skillSection === "public" ? "primary" : "outline"}" data-skill-source="public">在线公共技能</button><button class="button ${state.skillSection !== "public" ? "primary" : "outline"}" data-skill-source="community">用户分享技能</button><button id="skills-share" class="button outline">我要分享</button></nav><section id="skill-section-body"></section>`;
      body.querySelectorAll<HTMLButtonElement>("[data-skill-source]").forEach(button => button.addEventListener("click", () => { state.skillSection = button.dataset.skillSource as "public" | "community"; renderPage(); }));
      body.querySelector("#skills-share")!.addEventListener("click", options.goShare);
      if (state.skillSection === "shares") (body.querySelector("#skills-share") as HTMLButtonElement).hidden = true;
      const section = body.querySelector<HTMLElement>("#skill-section-body")!;
      if (state.skillSection === "public") renderSkillsPage(section);
      else renderPluginMarket(section, { ...options, kind: "skill" }, state.skillSection === "shares");
    }
    else if (state.libraryTab === "shares") renderPluginMarket(body, options, true);
    else {
      body.innerHTML = `<nav class="skill-source-tabs" aria-label="插件来源"><button class="button ${state.pluginSection === "market" ? "primary" : "outline"}" data-plugin-section="market">插件市场</button><button class="button ${state.pluginSection === "community" ? "primary" : "outline"}" data-plugin-section="community">内置与用户分享</button></nav><section id="plugin-section-body"></section>`;
      body.querySelectorAll<HTMLButtonElement>("[data-plugin-section]").forEach(button => button.addEventListener("click", () => { state.pluginSection = button.dataset.pluginSection as typeof state.pluginSection; renderPage(); }));
      const section = body.querySelector<HTMLElement>("#plugin-section-body")!;
      if (state.pluginSection === "market") renderPublicPluginMarket(section, { ...options, restartCodex: async () => { await invoke("launch_installed_product", { product: "codex", restart: true }); state.products.find(product => product.key === "codex")!.running = true; } });
      else renderPluginMarket(section, options);
    }
    return;
  }
  if (state.page === "proxy") {
    const managedSubscription = state.proxy.subscriptions.find(item => item.id === state.proxy.selectedSubscription);
    const configuredUrl = state.config.v2rayn_subscription_url || "";
    const hideConfiguredSubscriptionUrl = Boolean(managedSubscription && managedSubscription.url === configuredUrl);
    const subscriptionOptions = state.proxy.subscriptions.map(item => `<option value="${escapeHtml(item.id)}" ${item.id === state.proxy.selectedSubscription ? "selected" : ""}>${escapeHtml(item.remarks)}</option>`).join("");
    const routeName = (name: string) => {
      const normalized = name.toLowerCase();
      if (normalized.includes("white") || name.includes("绕过大陆")) return "V4 绕过大陆（白名单）";
      if (normalized.includes("black") || name.includes("黑名单")) return "V4 黑名单";
      if (normalized.includes("global") || name.includes("全局")) return "V4 全局";
      return name;
    };
    const routeOptions = state.proxy.routingProfiles.map(route => `<option value="${escapeHtml(route.id)}" ${route.id === state.proxy.selectedRoute || (!state.proxy.selectedRoute && route.isActive) ? "selected" : ""}>${escapeHtml(routeName(route.remarks))}</option>`).join("");
    const currentProfile = state.proxy.profiles.find(item => item.isCurrent);
    content.innerHTML = `<div class="page-heading"><div><p class="eyebrow">V2RAYN MANAGER</p><h1>网络代理</h1><p class="proxy-use-limit">本代理仅限用于 Codex 简体中文语言包的自动下载与切换、Codex 及其必要组件的安装与更新，以及保障 Codex 正常、完整使用所必需的相关网络连接；不得用于任何其他目的或行为。</p></div><div class="proxy-head-actions"><span class="state-pill ${state.proxy.running ? "active" : ""}">${state.proxy.running ? "核心运行中" : state.proxy.ready ? "核心已停止" : "服务未启动"}</span><button class="button quiet" id="proxy-refresh" title="刷新状态">↻</button></div></div>
      <section class="proxy-overview"><div><small>本机服务</small><b>${state.proxy.ready ? "已连接" : "按需启动"}</b></div><div><small>系统</small><b>${state.platform === "macos" ? "macOS" : "Windows"} ${state.arch.toUpperCase()}</b></div><div><small>订阅</small><b>${state.proxy.subscriptions.length} 个</b></div><div><small>节点</small><b>${visibleProxyProfiles().length} 个</b></div></section>
      <section class="proxy-manager-layout"><div class="proxy-main-column">
      <section class="panel proxy-preferences"><div class="panel-title"><div><h2>代理方式</h2><p>一键开关使用所选模式，并按路由模式转发流量。</p></div></div>
      <div class="proxy-preference-row"><label for="system-proxy-mode">系统代理模式</label><select id="system-proxy-mode"><option value="auto" ${state.proxy.systemProxyMode === "auto" ? "selected" : ""}>自动配置系统代理</option><option value="clear" ${state.proxy.systemProxyMode === "clear" ? "selected" : ""}>清除系统代理</option><option value="unchanged" ${state.proxy.systemProxyMode === "unchanged" ? "selected" : ""}>不改变系统代理</option><option value="pac" disabled>PAC 模式（助手暂不支持）</option></select></div>
      <div class="proxy-preference-row"><label for="routing-profile">路由模式</label><select id="routing-profile" ${state.proxy.routingProfiles.length ? "" : "disabled"}>${routeOptions || "<option>读取 v2rayN 路由中…</option>"}</select></div>
      <div class="system-proxy-row one-click-proxy"><div><b>浏览器代理</b><small>${state.proxy.systemProxyEnabled ? "已开启，浏览器流量经所选节点转发" : "开启后自动启动核心并配置所选代理模式"}</small>${state.proxy.systemProxyEnabled ? `<small class="system-proxy-node">当前节点：${escapeHtml(currentProfile?.remarks || "未选择")}</small>` : ""}</div><div class="proxy-primary-actions"><button id="system-proxy-toggle" class="button ${state.proxy.systemProxyEnabled ? "secondary" : "primary"}" ${state.proxy.systemProxySupported ? "" : "disabled"}>${state.proxy.systemProxyEnabled ? "关闭浏览器代理" : "开启浏览器代理"}</button><button id="clear-system-proxy" class="button quiet" ${state.proxy.systemProxySupported ? "" : "disabled"}>清理系统代理</button></div></div><p class="microcopy">关闭浏览器代理时会恢复原系统代理设置并停止核心。${state.platform === "macos" ? "首次修改网络设置时，请完成 macOS 系统授权。" : ""}PAC 模式暂未接入。</p></section>
      <section class="panel subscription-panel"><div class="panel-title"><div><h2>订阅管理</h2><p>订阅地址只保存在本机 v2rayN 配置中。</p></div><div class="subscription-actions">${state.proxy.subscriptions.length ? `<select id="subscription-select" aria-label="选择订阅">${subscriptionOptions}</select>` : ""}<button id="subscription-new" class="button quiet">新建</button><button id="subscription-update" class="button quiet" ${managedSubscription ? "" : "disabled"}>更新</button><button id="subscription-delete" class="button quiet danger-text" ${managedSubscription ? "" : "disabled"} title="删除订阅">删除</button></div></div><div class="subscription-entry"><input id="subscription-url" type="url" value="${escapeHtml(state.proxy.addingSubscription || hideConfiguredSubscriptionUrl ? "" : managedSubscription?.url || "")}" placeholder="粘贴订阅地址" autocomplete="off"><button id="subscription-add" class="button primary">${state.proxy.addingSubscription || !managedSubscription ? "添加订阅" : "保存地址"}</button></div><div class="subscription-current">${managedSubscription && !state.proxy.addingSubscription ? `<span class="status-dot"></span><b>${escapeHtml(managedSubscription.remarks)}</b><small>最近更新 ${managedSubscription.updateTime ? new Date(managedSubscription.updateTime * 1000).toLocaleString() : "尚未更新"}</small>` : `<span class="muted">${configuredUrl ? "后台订阅地址已载入，添加后可同步节点。" : "粘贴订阅地址后添加。"}</span>`}</div></section>
      <section class="panel node-panel" aria-busy="${state.proxyLoading}"><div class="panel-title"><div><h2>节点列表</h2><p>选择节点后启动核心；可对单个节点进行延迟测试。</p></div><span class="node-count">${visibleProxyProfiles().length} 节点</span></div><div class="proxy-loading" role="status" aria-live="polite" ${state.proxyLoading ? "" : "hidden"}>${icon("LoaderCircle")}<div><b id="proxy-loading-message">${escapeHtml(state.proxyLoadingMessage)}</b><small>请稍候，加载期间可切换其他页面。</small></div></div><div class="node-list">${visibleProxyProfiles().length ? visibleProxyProfiles().map(proxyProfileRow).join("") : state.proxyLoading ? "" : `<div class="empty-state">${state.proxy.message ? "节点加载未完成，请点击右上角刷新重试。" : state.proxy.ready ? "添加并更新订阅以加载节点。" : "连接本机服务后加载节点列表。"}</div>`}</div></section>
      </div><aside class="proxy-side-column"><section class="panel core-panel"><div class="panel-title"><div><h2>代理核心</h2><p>${state.proxy.running ? "核心正在处理代理请求" : "核心当前未运行"}</p></div><span class="core-indicator ${state.proxy.running ? "running" : ""}"></span></div><dl><div><dt>当前节点</dt><dd>${escapeHtml(currentProfile?.remarks || "未选择")}</dd></div><div><dt>本地 SOCKS</dt><dd>${escapeHtml(state.proxy.socksAddress)}</dd></div><div><dt>本地 HTTP</dt><dd>${escapeHtml(state.proxy.httpAddress)}</dd></div><div><dt>运行状态</dt><dd>${state.proxy.running ? "已启动" : "已停止"}</dd></div></dl></section><section class="panel service-panel"><h2>内置服务</h2><p>${state.proxy.ready ? "v2rayN WebAPI 与 Core 已随绿色包运行。" : "助手将按需启动包内的 v2rayN 管理服务和 Core。"}</p><button id="proxy-service-start" class="button outline">${state.proxy.ready ? "重新读取状态" : "连接本机服务"}</button></section></aside></section><p class="proxy-feedback" id="proxy-feedback" aria-live="polite">${escapeHtml(state.proxy.message)}</p>`;
    document.querySelector<HTMLButtonElement>("#proxy-refresh")!.addEventListener("click", loadProxyState);
    document.querySelector<HTMLButtonElement>("#proxy-service-start")!.addEventListener("click", loadProxyState);
    document.querySelector<HTMLButtonElement>("#subscription-add")!.addEventListener("click", saveProxySubscription);
    document.querySelector<HTMLButtonElement>("#subscription-new")!.addEventListener("click", () => { state.proxy.addingSubscription = true; state.proxy.selectedSubscription = ""; renderPage(); });
    document.querySelector<HTMLButtonElement>("#subscription-delete")?.addEventListener("click", deleteProxySubscription);
    document.querySelector<HTMLButtonElement>("#subscription-update")!.addEventListener("click", updateProxySubscription);
    document.querySelector<HTMLSelectElement>("#subscription-select")?.addEventListener("change", event => { state.proxy.selectedSubscription = (event.currentTarget as HTMLSelectElement).value; state.proxy.addingSubscription = false; void loadProxyState(); });
    document.querySelector<HTMLButtonElement>("#system-proxy-toggle")!.addEventListener("click", () => void toggleSystemProxy());
    document.querySelector<HTMLButtonElement>("#clear-system-proxy")!.addEventListener("click", () => void applySystemProxyMode("clear"));
    document.querySelector<HTMLSelectElement>("#system-proxy-mode")!.addEventListener("change", event => {
      void applySystemProxyMode((event.currentTarget as HTMLSelectElement).value as SystemProxyMode);
    });
    document.querySelector<HTMLSelectElement>("#routing-profile")!.addEventListener("change", event => { void activateRoutingProfile((event.currentTarget as HTMLSelectElement).value); });
    document.querySelectorAll<HTMLButtonElement>("[data-profile-select]").forEach(button => button.addEventListener("click", () => selectProxyProfile(button.dataset.profileSelect!)));
    document.querySelectorAll<HTMLButtonElement>("[data-profile-test]").forEach(button => button.addEventListener("click", () => testProxyProfile(button.dataset.profileTest!)));
    simplifyProxyLayout(content);
    setProxyBusy(state.proxyLoading || state.proxyOperationBusy || (state.busy && !state.installTaskActive));
    return;
  }
  if (state.page === "config" && !state.providerEditorOpen && !state.providersLoaded) {
    content.innerHTML = `<div class="page-heading"><div><p class="eyebrow">PROVIDER</p><h1>供应商配置</h1><p class="muted">正在读取本机供应商…</p></div></div>`;
    void loadCodexProviders();
    return;
  }
  if (!state.providerEditorOpen && state.providerCatalogOpen) {
    const matches = providerPresets.filter(preset => `${preset.name} ${preset.note} ${preset.baseUrl}`.toLowerCase().includes(state.providerSearch.toLowerCase()));
    const cards = (category: ProviderPreset["category"]) => matches.filter(preset => preset.category === category).map(preset =>
      `<button class="provider-catalog-card" type="button" data-provider-preset="${escapeHtml(preset.name)}"><span class="provider-catalog-mark">${escapeHtml(preset.name.slice(0, 1))}</span><span><b>${escapeHtml(preset.name)}</b><small>${escapeHtml(preset.note || preset.baseUrl)}</small></span><span class="provider-card-arrow">${icon("ArrowRight")}</span></button>`
    ).join("");
    content.innerHTML = `<div class="page-heading provider-catalog-heading"><button id="provider-catalog-back" class="icon-button" aria-label="返回供应商列表">${icon("ArrowRight")}</button><div><h1>添加新供应商</h1><p class="muted">Codex++</p></div></div><section class="provider-catalog"><label class="provider-search">${icon("Search")}<input id="provider-search" type="search" placeholder="搜索名称或网址，例如 kimi、智谱、openrouter" value="${escapeHtml(state.providerSearch)}"></label><button id="provider-custom" class="provider-custom-card" type="button"><span class="provider-catalog-mark">${icon("Sparkles")}</span><span><b>自定义配置</b><small>手动填写请求地址和 API Key</small></span></button><h2>账号登录</h2><div class="provider-catalog-grid">${cards("account") || `<p class="provider-catalog-empty">没有匹配的账号登录入口。</p>`}</div><h2>模型厂商</h2><div class="provider-catalog-grid">${cards("model") || `<p class="provider-catalog-empty">没有匹配的模型厂商。</p>`}</div></section>`;
    document.querySelector("#provider-catalog-back")!.addEventListener("click", () => { state.providerCatalogOpen = false; state.providerSearch = ""; renderPage(); });
    document.querySelector<HTMLInputElement>("#provider-search")!.addEventListener("input", event => { state.providerSearch = (event.currentTarget as HTMLInputElement).value; const pos = event.currentTarget as HTMLInputElement; renderPage(); document.querySelector<HTMLInputElement>("#provider-search")?.focus(); const input = document.querySelector<HTMLInputElement>("#provider-search"); input?.setSelectionRange(pos.selectionStart, pos.selectionEnd); });
    document.querySelector("#provider-custom")!.addEventListener("click", () => openProviderEditor("", { name: "", baseUrl: "", model: "", protocol: "responses", category: "model" }));
    content.querySelectorAll<HTMLButtonElement>("[data-provider-preset]").forEach(button => button.addEventListener("click", () => {
      const preset = providerPresets.find(item => item.name === button.dataset.providerPreset)!;
      if (preset.name === "OpenAI Official") {
        void startOrRestartProduct("codexpp");
        toast("请在 Codex++ 中完成 OpenAI 官方账号登录；助手不收集或保存官方账号凭据。");
        return;
      }
      openProviderEditor("", preset);
    }));
    return;
  }
  if (!state.providerEditorOpen) {
    const existingNutProvider = state.providers.find(provider => provider.baseUrl.replace(/\/+$/, "") === "https://api.jianguo.ltd/v1");
    content.innerHTML = `<div class="page-heading"><div><p class="eyebrow">PROVIDER</p><h1>供应商配置</h1><p class="muted">Codex 与 Codex++</p></div></div><section class="panel settings-panel"><div class="panel-title"><div><h2>供应商管理</h2><p>本机已配置的 Codex++ 供应商</p></div><button id="provider-new" class="button primary" type="button">添加供应商</button></div><div class="provider-list">${state.providers.length ? state.providers.map(provider => `<div class="provider-item" data-provider-id="${escapeHtml(provider.id)}"><div class="provider-item-main"><b>${escapeHtml(provider.name)}${provider.isActive ? `<span class="provider-active-label">${provider.isGateway ? "网关入口" : "使用中"}</span>` : ""}${provider.isCurrent ? `<span class="provider-current-label">当前上游</span>` : ""}</b><small>${escapeHtml(provider.baseUrl)} · ${escapeHtml(provider.model || "未设置模型")}${provider.hasApiKey ? " · 已配置密钥" : " · 未配置密钥"}</small></div><div class="provider-action-toolbar" role="group" aria-label="${escapeHtml(provider.name)}操作"><button class="provider-icon-action ${provider.isActive ? "is-active" : ""}" data-use-provider="${escapeHtml(provider.id)}" aria-label="${provider.isActive ? "使用中" : "使用"}" title="${provider.isActive ? "使用中" : "使用"}" ${provider.isActive ? "disabled" : ""}>${icon(provider.isActive ? "Check" : "Circle")}</button><button class="provider-icon-action" data-start-codexpp aria-label="启动 Codex++" title="启动 Codex++">${icon("Play")}</button><button class="provider-icon-action" data-edit-provider="${escapeHtml(provider.id)}" aria-label="编辑供应商" title="编辑">${icon("Pencil")}</button><button class="provider-icon-action danger" data-delete-provider="${escapeHtml(provider.id)}" aria-label="删除供应商" title="删除">${icon("Trash2")}</button></div></div>`).join("") : `<p class="empty-provider-list">尚未找到已配置的供应商。</p>`}</div></section>`;
    document.querySelector<HTMLButtonElement>("#provider-new")!.addEventListener("click", () => { state.providerCatalogOpen = true; state.providerSearch = ""; renderPage(); });
    const recommendation = document.createElement("section");
    recommendation.className = "nut-service";
    recommendation.setAttribute("aria-labelledby", "nut-service-title");
    recommendation.innerHTML = `<details class="nut-service-details" open><summary><span class="nut-service-mark" aria-hidden="true">J</span><span><span class="nut-service-label">推荐服务</span><span class="nut-service-title" id="nut-service-title">坚果专用中转服务</span></span><span class="nut-service-chevron" aria-hidden="true">⌄</span></summary><div class="nut-service-body"><p class="nut-service-description">为 Codex 与 Codex++ 提供供应商接入服务。开通后获取 API Key，即可在助手中完成配置。</p><ul class="nut-service-highlights"><li>${icon("Check")}预设坚果接口，减少手动填写</li><li>${icon("Check")}从上游获取可用模型，按需保留</li><li>${icon("Check")}一键写入供应商，并可选择附加设置</li></ul><div class="nut-service-actions"><button id="nut-service-open" class="button quiet" type="button">${icon("ArrowRight")}了解与开通</button><button id="nut-service-configure" class="button primary nut-service-configure" type="button">${icon("Settings")}${existingNutProvider ? "编辑坚果供应商" : "配置坚果供应商"}</button></div><p class="nut-service-note">模型、价格及服务范围请在服务页面查看。</p></div></details>`;
    content.querySelector(".page-heading")!.after(recommendation);
    document.querySelector("#nut-service-open")!.addEventListener("click", openKeyPurchasePage);
    document.querySelector("#nut-service-configure")!.addEventListener("click", () => {
      openProviderEditor(existingNutProvider?.id);
      if (!existingNutProvider && state.nutSubscriptionKami) {
        state.providerApiKey = state.nutSubscriptionKami;
        const apiKeyInput = document.querySelector<HTMLInputElement>("#provider-key");
        if (apiKeyInput) apiKeyInput.value = state.providerApiKey;
      }
    });
    content.querySelectorAll<HTMLButtonElement>("[data-use-provider]").forEach(button => button.addEventListener("click", () => void activateCodexProvider(button.dataset.useProvider!)));
    content.querySelectorAll<HTMLButtonElement>("[data-start-codexpp]").forEach(button => button.addEventListener("click", () => void startCodexppWithProvider(button.closest<HTMLElement>(".provider-item")?.dataset.providerId || "")));
    content.querySelectorAll<HTMLButtonElement>("[data-edit-provider]").forEach(button => button.addEventListener("click", () => openProviderEditor(button.dataset.editProvider!)));
    content.querySelectorAll<HTMLButtonElement>("[data-delete-provider]").forEach(button => button.addEventListener("click", () => void deleteCodexProvider(button.dataset.deleteProvider!)));
    return;
  }

  content.innerHTML = `<div class="page-heading"><div><h1>供应商配置</h1><p class="muted">${state.editingProviderId ? "编辑供应商" : `配置 ${escapeHtml(state.providerPreset?.name || "自定义供应商")}`}</p></div><button id="provider-cancel" class="button quiet" type="button">返回${state.editingProviderId ? "列表" : "供应商目录"}</button></div><section class="panel settings-panel"><label class="provider-name-field">供应商名称<input id="provider-name" value="${escapeHtml(state.providerPreset?.name ?? (state.selectedProvider === "nut" ? "坚果" : ""))}" placeholder="供应商名称"></label><section class="provider-access"><h2>接入方式</h2><p class="microcopy">如果要显示官网插件和技能，请先在 Codex 中完成官方账号登录，再选择“官方登录 + API”。官方登录态只保存在 Codex，本助手不会收集。</p><div class="provider-access-options"><label><input type="radio" name="provider-access" value="pureApi" ${state.providerAccessMode === "pureApi" ? "checked" : ""}><span><b>纯 API 直连</b><small>仅使用供应商 API Key；官网插件和技能需要官方登录态。</small></span></label><label><input type="radio" name="provider-access" value="officialMixApi" ${state.providerAccessMode === "officialMixApi" ? "checked" : ""}><span><b>官方登录 + API</b><small>模型走供应商 API，同时保留官方账号访问插件、技能等官网内容。</small></span></label><label><input type="radio" name="provider-access" value="gateway" ${state.providerAccessMode === "gateway" ? "checked" : ""}><span><b>本地网关</b><small>Codex 连接助手本地网关，由网关适配上游供应商协议。</small></span></label></div><p id="gateway-status" class="microcopy" hidden></p></section><div class="provider-fields"><label>Base URL<input id="provider-url" value="${escapeHtml(state.providerPreset?.baseUrl ?? (state.selectedProvider === "nut" ? "https://api.jianguo.ltd/v1" : ""))}" placeholder="HTTPS 接口地址"></label><label>API Key<span class="api-key-control"><input id="provider-key" type="password" autocomplete="off" value="${escapeHtml(state.providerApiKey)}" placeholder="留空以保留已有密钥"></span></label><label>上游协议<select id="provider-protocol"><option value="responses" ${state.providerProtocol === "responses" ? "selected" : ""}>Responses API</option><option value="chatCompletions" ${state.providerProtocol === "chatCompletions" ? "selected" : ""}>Chat Completions</option></select></label></div><details class="provider-model-settings" open><summary>模型配置</summary><label>默认模型<span class="model-control"><input id="provider-model" value="${escapeHtml(state.providerPreset?.model || "")}" placeholder="手动输入或从上游获取"><button id="fetch-models" class="button quiet" type="button">从上游获取</button><button id="probe-protocol" class="button quiet" type="button">检测协议</button></span></label><div id="provider-models" class="provider-models">${renderProviderModels()}</div></details><p id="model-feedback" class="microcopy model-feedback" aria-live="polite"></p><div class="provider-actions"><button id="configure-provider" class="button primary">一键配置</button><span class="microcopy">API Key 仅保留在助手运行内存中，退出后清除。</span></div></section>`;
  content.classList.add("provider-editing");
  const connectionFields = content.querySelector<HTMLElement>(".provider-fields")!;
  const connectionSection = document.createElement("section");
  connectionSection.className = "provider-connection-section";
  connectionSection.innerHTML = "<h2>连接凭据</h2>";
  connectionFields.before(connectionSection);
  connectionSection.append(connectionFields);
  content.querySelector(".provider-actions")!.classList.add("editor-footer");
  document.querySelectorAll<HTMLButtonElement>("[data-provider]").forEach(button => button.addEventListener("click", () => { state.selectedProvider = button.dataset.provider as "nut" | "custom"; renderPage(); }));
  document.querySelectorAll<HTMLInputElement>('input[name="provider-access"]').forEach(input => input.addEventListener("change", () => { state.providerAccessMode = input.value as typeof state.providerAccessMode; }));
  document.querySelectorAll<HTMLInputElement>('input[name="provider-access"]').forEach(input => input.addEventListener("change", () => { updateGatewayStatus(); }));
  document.querySelector<HTMLButtonElement>("#provider-cancel")!.addEventListener("click", closeProviderEditor);
  document.querySelector<HTMLButtonElement>("#configure-provider")!.addEventListener("click", configureCustomProvider);
  if (state.setupEditor) {
    document.querySelector(".page-heading p")!.textContent = `一键准备：Codex${state.includeCodexpp ? " + Codex++" : ""} → 配置坚果供应商`;
    document.querySelector("#configure-provider")!.textContent = "一键安装并配置";
    document.querySelector("#provider-cancel")!.textContent = "取消并返回安装页";
    document.querySelector<HTMLElement>(".provider-access")!.hidden = true;
    document.querySelector<HTMLSelectElement>("#provider-protocol")!.disabled = true;
    document.querySelector<HTMLInputElement>("#provider-url")!.readOnly = true;
    document.querySelector<HTMLInputElement>("#provider-name")!.readOnly = true;
  }
  document.querySelector<HTMLButtonElement>("#fetch-models")!.addEventListener("click", fetchProviderModels);
  document.querySelector<HTMLButtonElement>("#probe-protocol")!.addEventListener("click", probeProviderProtocol);
  void updateGatewayStatus();
  document.querySelector<HTMLInputElement>("#provider-key")!.addEventListener("input", event => {
    state.providerApiKey = (event.currentTarget as HTMLInputElement).value;
  });
  document.querySelector<HTMLElement>("#provider-models")!.addEventListener("click", event => {
    const button = (event.target as Element).closest<HTMLButtonElement>("[data-remove-model]");
    if (button) {
      event.stopPropagation();
      state.providerModels.splice(Number(button.dataset.removeModel), 1);
      document.querySelector("#provider-models")!.innerHTML = renderProviderModels();
      return;
    }
    const modelItem = (event.target as Element).closest<HTMLElement>("[data-select-model]");
    if (!modelItem) return;
    const modelInput = document.querySelector<HTMLInputElement>("#provider-model");
    if (modelInput) {
      modelInput.value = modelItem.dataset.selectModel || "";
      document.querySelector("#provider-models")!.innerHTML = renderProviderModels();
    }
  });
  document.querySelector<HTMLElement>("#provider-models")!.addEventListener("keydown", event => {
    if (!(event.target instanceof HTMLElement) || !["Enter", " "].includes(event.key)) return;
    const modelItem = event.target.closest<HTMLElement>("[data-select-model]");
    if (!modelItem || (event.target as Element).closest("[data-remove-model]")) return;
    event.preventDefault();
    const modelInput = document.querySelector<HTMLInputElement>("#provider-model");
    if (modelInput) {
      modelInput.value = modelItem.dataset.selectModel || "";
      document.querySelector("#provider-models")!.innerHTML = renderProviderModels();
    }
  });
  document.querySelector<HTMLInputElement>("#provider-key")!.placeholder = state.editingProviderId && state.providers.find(provider => provider.id === state.editingProviderId)?.hasApiKey ? "已配置密钥；留空以保留" : "请输入 API Key";
  return;
}

function openProviderEditor(providerId = "", preset?: ProviderPreset) {
  state.editingProviderId = providerId;
  state.providerEditorOpen = true;
  state.providerCatalogOpen = false;
  state.providerPreset = preset || null;
  const provider = state.providers.find(item => item.id === providerId);
  state.providerApiKey = provider?.apiKey || "";
  state.providerModels = provider ? [...provider.models] : [];
  state.selectedProvider = provider ? provider.name === "坚果" ? "nut" : "custom" : "nut";
  state.providerProtocol = preset?.protocol || "responses";
  state.providerAccessMode = "pureApi";
  renderPage();
  if (!provider) return;
  state.providerProtocol = provider.protocol === "chatCompletions" ? "chatCompletions" : "responses";
  state.providerAccessMode = provider.relayMode === "gateway" ? "gateway" : provider.relayMode === "officialMixApi" ? "officialMixApi" : "pureApi";
  document.querySelector<HTMLSelectElement>("#provider-protocol")!.value = state.providerProtocol;
  const accessInput = document.querySelector<HTMLInputElement>(`input[name="provider-access"][value="${state.providerAccessMode}"]`);
  if (accessInput) accessInput.checked = true;
  document.querySelector<HTMLInputElement>("#provider-name")!.value = provider.name;
  document.querySelector<HTMLInputElement>("#provider-url")!.value = provider.baseUrl;
  document.querySelector<HTMLInputElement>("#provider-model")!.value = provider.model;
  document.querySelector<HTMLSelectElement>("#provider-protocol")!.value = "responses";
  document.querySelector("#provider-models")!.innerHTML = renderProviderModels();
}

function simplifyProxyLayout(content: HTMLElement) {
  const preferences = content.querySelector<HTMLElement>(".proxy-preferences")!;
  const overview = content.querySelector<HTMLElement>(".proxy-overview")!;
  const toggleRow = preferences.querySelector<HTMLElement>(".system-proxy-row")!;
  const node = state.proxy.profiles.find(profile => profile.isCurrent);
  const heading = content.querySelector(".page-heading p.muted");
  if (heading) heading.textContent = "本代理仅限用于 Codex 简体中文语言包的自动下载与切换、Codex 及其必要组件的安装与更新，以及保障 Codex 正常、完整使用所必需的相关网络连接；不得用于任何其他目的或行为。";
  const connection = document.createElement("section");
  connection.className = "proxy-connection";
  const status = document.createElement("div");
  status.className = "connection-status";
  status.innerHTML = `<span class="connection-icon ${state.proxy.systemProxyEnabled ? "connected" : ""}">${icon("Globe")}</span><div><h2>${state.proxy.systemProxyEnabled ? "浏览器代理已开启" : "浏览器代理未开启"}</h2><p>${escapeHtml(node?.remarks || (state.proxy.ready ? "请先从下方选择节点" : "正在准备本机服务"))}</p></div>`;
  const actions = toggleRow.querySelector<HTMLElement>(".proxy-primary-actions")!;
  connection.append(status, actions);
  overview.replaceWith(connection);
  toggleRow.remove();
  const advanced = document.createElement("section");
  advanced.className = "proxy-advanced";
  const subscription = content.querySelector<HTMLElement>(".subscription-panel")!;
  const side = content.querySelector<HTMLElement>(".proxy-side-column")!;
  advanced.append(preferences, subscription, side);
  const layout = content.querySelector<HTMLElement>(".proxy-manager-layout")!;
  const tabs = document.createElement("div");
  tabs.className = "proxy-tabs";
  tabs.setAttribute("role", "tablist");
  tabs.innerHTML = `<button type="button" role="tab" data-proxy-view="nodes" aria-selected="${state.proxyView === "nodes"}">节点与连接</button><button type="button" role="tab" data-proxy-view="subscriptions" aria-selected="${state.proxyView === "subscriptions"}">订阅管理</button><button type="button" role="tab" data-proxy-view="settings" aria-selected="${state.proxyView === "settings"}">连接设置</button>`;
  layout.before(tabs);
  layout.after(advanced);
  const applyProxyView = () => {
    layout.hidden = state.proxyView !== "nodes";
    advanced.hidden = state.proxyView === "nodes";
    preferences.hidden = state.proxyView === "subscriptions";
    subscription.hidden = state.proxyView === "settings";
    side.hidden = state.proxyView === "subscriptions";
    tabs.querySelectorAll<HTMLButtonElement>("[data-proxy-view]").forEach(button => {
      const selected = button.dataset.proxyView === state.proxyView;
      button.setAttribute("aria-selected", String(selected));
      button.tabIndex = selected ? 0 : -1;
    });
  };
  tabs.querySelectorAll<HTMLButtonElement>("[data-proxy-view]").forEach(button => button.addEventListener("click", () => {
    state.proxyView = button.dataset.proxyView as typeof state.proxyView;
    tabs.querySelectorAll<HTMLButtonElement>("[data-proxy-view]").forEach(item => item.setAttribute("aria-selected", String(item === button)));
    applyProxyView();
  }));
  tabs.addEventListener("keydown", event => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const buttons = [...tabs.querySelectorAll<HTMLButtonElement>("[data-proxy-view]")];
    const index = buttons.findIndex(button => button.dataset.proxyView === state.proxyView);
    const next = event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1
      : (index + (event.key === "ArrowRight" ? 1 : -1) + buttons.length) % buttons.length;
    buttons[next].click();
    buttons[next].focus();
  });
  applyProxyView();
  const coreDescription = preferences.querySelector(".panel-title p");
  if (coreDescription) coreDescription.textContent = "默认使用自动配置系统代理。";
  const feedback = content.querySelector<HTMLElement>("#proxy-feedback")!;
  connection.after(feedback);
  const title = content.querySelector(".node-panel h2");
  if (title) title.textContent = "选择代理节点";
  const nodeDescription = content.querySelector(".node-panel .panel-title p");
  if (nodeDescription) nodeDescription.textContent = "当前节点会用于浏览器代理连接。";
}

function renderProviderModels() {
  if (!state.providerModels.length) return `<div class="provider-model-empty">获取上游模型后，可删除不需要的项；保留项会写入该供应商模型列表。</div>`;
  const selectedModel = document.querySelector<HTMLInputElement>("#provider-model")?.value || state.providerPreset?.model || "";
  return `<div class="provider-model-heading">模型列表 <span>${state.providerModels.length}</span><small>点击模型即可设为默认模型</small></div><div class="provider-model-list">${state.providerModels.map((model, index) => `<div class="provider-model-item ${model === selectedModel ? "selected" : ""}" data-select-model="${escapeHtml(model)}" role="button" tabindex="0" title="设为默认模型"><span>${escapeHtml(model)}</span><button type="button" data-remove-model="${index}" aria-label="删除模型 ${escapeHtml(model)}" title="删除模型">×</button></div>`).join("")}</div>`;
}

function renderSkillsPage(content: HTMLElement) {
  if (!state.skillsLoaded && !state.skillsLoading && !state.skillsError) queueMicrotask(() => void loadSkillCatalog());
  const categories = ["全部", ...new Set(state.skills.map(skill => skill.category))];
  const query = state.skillSearch.trim().toLowerCase();
  const filtered = state.skills.filter(skill => {
    const matchesCategory = state.skillCategory === "全部" || skill.category === state.skillCategory;
    const haystack = `${skill.name_zh} ${skill.display_name} ${skill.description_zh} ${skill.author} ${skill.tags.join(" ")}`.toLowerCase();
    return matchesCategory && (!query || haystack.includes(query));
  }).sort((a, b) => state.skillSort === "name" ? a.name_zh.localeCompare(b.name_zh, "zh-CN") : state.skillSort === "downloads" ? (b.bytes * b.skills - a.bytes * a.skills) : (b.skills - a.skills || a.name_zh.localeCompare(b.name_zh, "zh-CN")));
  const groups = [...new Set(filtered.map(skill => skill.category))];
  content.innerHTML = `<div class="page-heading"><div><p class="eyebrow">CODEX SKILLS</p><h1>技能库</h1></div><button id="skills-refresh" class="button quiet" ${state.skillsLoading ? "disabled" : ""}>${icon("RefreshCw")}<span>${state.skillsLoading ? "读取中…" : "刷新目录"}</span></button></div>
    <section class="skills-toolbar"><label class="skill-search">${icon("Search")}<input id="skill-search" type="search" placeholder="搜索技能名称、用途或作者，按 Enter 搜索" value="${escapeHtml(state.skillSearchDraft)}"></label><select id="skill-sort" aria-label="排序"><option value="popular" ${state.skillSort === "popular" ? "selected" : ""}>按技能数量</option><option value="downloads" ${state.skillSort === "downloads" ? "selected" : ""}>按包大小</option><option value="name" ${state.skillSort === "name" ? "selected" : ""}>按名称</option></select></section>
    <section class="skill-category-panel"><div class="skill-category-heading"><b>技能分类</b><small>共 ${categories.length - 1} 个分类</small></div><div class="skill-category-list">${categories.map(category => `<button type="button" class="skill-category-chip ${category === state.skillCategory ? "selected" : ""}" data-skill-category="${escapeHtml(category)}">${escapeHtml(category)}</button>`).join("")}</div></section>
    ${state.skillsLoading ? `<section class="panel skill-loading">正在读取 Codex++ 技能目录…</section>` : state.skills.length === 0 ? `<section class="panel skill-loading">暂无技能目录，请检查网络后刷新。</section>` : `<div class="skill-summary">${filtered.length} 个技能 · ${state.skills.filter(skill => skill.installed).length} 个已安装</div><div class="skill-groups">${groups.map(category => { const items = filtered.filter(skill => skill.category === category); return `<details class="skill-group" open><summary><span><b>${escapeHtml(category)}</b><small>${items.length} 个技能</small></span><span>⌄</span></summary><div class="skill-list">${items.map(renderSkillItem).join("")}</div></details>`; }).join("") || `<section class="panel skill-loading">没有匹配的技能。</section>`}</div>`}`;
  document.querySelector<HTMLInputElement>("#skill-search")?.addEventListener("input", event => {
    state.skillSearchDraft = (event.currentTarget as HTMLInputElement).value;
  });
  document.querySelector<HTMLInputElement>("#skill-search")?.addEventListener("keydown", event => {
    if (event.key !== "Enter") return;
    event.preventDefault();
    state.skillSearch = state.skillSearchDraft;
    renderSkillsPage(content);
  });
  if (state.skillsError) {
    const emptyState = content.querySelector(".skill-loading");
    if (emptyState) emptyState.remove();
    content.querySelector(".page-heading")!.insertAdjacentHTML("afterend", `<section class="skill-load-error" role="status"><p>${escapeHtml(state.skillsError)}</p><div class="provider-actions"><button id="skills-retry" class="button outline" type="button">${icon("RefreshCw")}重试加载</button>${state.skillsNetworkError ? `<button id="skills-open-proxy" class="button primary" type="button">${icon("Globe")}前往网络代理</button>` : ""}</div></section>`);
    document.querySelector("#skills-retry")?.addEventListener("click", () => void loadSkillCatalog(true));
    document.querySelector("#skills-open-proxy")?.addEventListener("click", () => {
      document.querySelector<HTMLButtonElement>('[data-page="proxy"]')?.click();
    });
  }
  document.querySelectorAll<HTMLButtonElement>("[data-skill-category]").forEach(button => button.addEventListener("click", () => { state.skillCategory = button.dataset.skillCategory || "全部"; renderPage(); }));
  document.querySelector<HTMLSelectElement>("#skill-sort")?.addEventListener("change", event => { state.skillSort = (event.currentTarget as HTMLSelectElement).value as typeof state.skillSort; renderPage(); });
  document.querySelector<HTMLButtonElement>("#skills-refresh")?.addEventListener("click", () => void loadSkillCatalog(true));
  document.querySelectorAll<HTMLButtonElement>("[data-install-skill]").forEach(button => button.addEventListener("click", () => void installSkill(button.dataset.installSkill!)));
}

function renderSkillItem(skill: SkillItem) {
  const installing = state.installingSkills.has(skill.id);
  const size = skill.bytes > 1024 * 1024 ? `${(skill.bytes / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(skill.bytes / 1024))} KB`;
  const sizeLabel = skill.bytes ? ` · ${size}` : "";
  const badge = skill.source === "composio" ? `<span class="skill-badge">社区收录</span>` : skill.requires_auth ? `<span class="skill-badge">需授权</span>` : `<span class="skill-badge free">开源</span>`;
  return `<article class="skill-item"><div class="skill-mark">${escapeHtml(skill.name_zh.slice(0, 1))}</div><div class="skill-info"><div><b>${escapeHtml(skill.name_zh)}</b>${badge}</div><p>${escapeHtml(skill.description_zh || "正在生成中文简介…")}</p><small>${escapeHtml(skill.author || "社区贡献者")} · ${skill.source === "composio" ? "Composio 社区精选" : `${skill.skills} 项技能`}${sizeLabel}</small></div><button class="button ${skill.installed ? "secondary" : "outline"}" data-install-skill="${escapeHtml(skill.id)}" ${skill.installed || installing ? "disabled" : ""}>${installing ? "安装中…" : skill.installed ? "已安装" : "安装"}</button></article>`;
}

async function loadSkillCatalog(force = false) {
  if (!isNative() || state.skillsLoading || (state.skillsLoaded && !force)) return;
  state.skillsError = "";
  state.skillsNetworkError = false;
  state.skillsLoading = true;
  if (state.page === "plugins" && state.libraryTab === "skills") renderPage();
  try {
    state.skills = await invoke<SkillItem[]>("list_codex_skills");
    state.skillsLoaded = true;
  } catch (error) {
    const raw = errorText(error, "");
    const unavailable = /SKILL_CATALOG_UNAVAILABLE|404|410|not found/i.test(raw);
    const invalid = /SKILL_CATALOG_INVALID|格式|缺少 plugins/i.test(raw);
    state.skillsNetworkError = !unavailable && !invalid;
    state.skillsError = unavailable
      ? "技能目录暂不可用，请稍后重试。若持续出现，请更新 Codex 使用助手。"
      : invalid
      ? "技能目录内容异常，暂时无法读取，请稍后刷新或更新 Codex 使用助手。"
      : state.proxy.systemProxyEnabled
      ? "无法连接技能库，请在网络代理中检查当前节点或更换节点，然后返回重试。"
      : "无法连接技能库，请先前往网络代理，选择节点并开启浏览器代理，然后返回重试。";
    if (state.skills.length) state.skillsError += " 当前保留上次已加载的技能。";
  } finally {
    state.skillsLoading = false;
    if (state.page === "plugins" && state.libraryTab === "skills") renderPage();
  }
}

async function installSkill(id: string) {
  const skill = state.skills.find(item => item.id === id);
  if (!skill || skill.installed || state.installingSkills.has(id)) return;
  if (!isNative()) { toast("请在桌面应用中安装技能", true); return; }
  state.installingSkills.add(id);
  const button = document.querySelector<HTMLButtonElement>(`[data-install-skill="${CSS.escape(id)}"]`);
  if (button) { button.disabled = true; button.textContent = "安装中…"; }
  try {
    await checkInstallations();
    if (!state.products.find(product => product.key === "codex")?.ready) {
      throw new Error("尚未检测到已安装的 Codex，请先在安装页面完成 Codex 安装，再安装技能。");
    }
    await invoke("install_codex_skill", { name: skill.name, tree: skill.tree, path: skill.path, source: skill.source });
    skill.installed = true;
    const currentSkill = state.skills.find(item => item.id === id);
    if (currentSkill) currentSkill.installed = true;
    toast(`${skill.name_zh} 已安装`);
  } catch (error) {
    toast(errorText(error, "技能安装失败"), true);
  } finally {
    state.installingSkills.delete(id);
    if (state.page === "plugins" && state.libraryTab === "skills") renderPage();
  }
}

function closeProviderEditor() {
  if (state.busy) return;
  if (state.setupEditor) {
    state.setupEditor = false;
    state.providerEditorOpen = false;
    state.providerCatalogOpen = false;
    state.page = "install";
    renderDashboard();
    return;
  }
  const wasEditing = Boolean(state.editingProviderId);
  state.providerEditorOpen = false;
  state.editingProviderId = "";
  state.providerApiKey = "";
  state.providerPreset = null;
  state.providerCatalogOpen = !wasEditing;
  renderPage();
}

async function deleteCodexProvider(providerId: string) {
  if (state.busy) return;
  const provider = state.providers.find(item => item.id === providerId);
  if (!provider) return;
  const warning = provider.isActive || provider.isCurrent
    ? "\n\n此供应商正在使用。删除后不会自动切换到其他供应商，请重新配置后再使用；已运行的 Codex 或网关会话可能仍保留旧配置，请重启相应程序。"
    : "";
  if (!await confirmDialog("删除供应商", `确定删除供应商“${provider.name}”？此操作会移除本机供应商条目及匹配的当前直连配置。${warning}`, "删除")) return;
  if (state.busy) return;
  try {
    await invoke("delete_codexpp_provider", { providerId });
    state.providers = await invoke<CodexProvider[]>("list_codexpp_providers");
    renderPage();
    toast(`供应商“${provider.name}”已删除${provider.isActive || provider.isCurrent ? "，请重新选择供应商并重启 Codex" : ""}`);
  } catch (error) {
    toast(errorText(error, "删除供应商失败"), true);
  }
}

async function loadCodexProviders() {
  if (!isNative()) return;
  try {
    state.providers = await invoke<CodexProvider[]>("list_codexpp_providers");
    state.providersLoaded = true;
    state.providerHealth = Object.fromEntries(state.providers.map(provider => [provider.id, { state: "checking", message: "检测中" }]));
    if (!state.providerEditorOpen) renderPage();
    const health = await Promise.all(state.providers.map(async provider => {
      if (!provider.hasApiKey || !provider.apiKey || !provider.baseUrl) {
        return [provider.id, { state: "offline", message: "缺少 API Key 或接口地址" }] as const;
      }
      try {
        await invoke<string[]>("check_codexpp_provider", { baseUrl: provider.baseUrl, apiKey: provider.apiKey });
        return [provider.id, { state: "online", message: "连接正常" }] as const;
      } catch (error) {
        return [provider.id, { state: "offline", message: errorText(error, "连接失败") }] as const;
      }
    }));
    state.providerHealth = Object.fromEntries(health);
    if (!state.providerEditorOpen) renderPage();
  } catch (error) {
    state.providersLoaded = true;
    const list = document.querySelector<HTMLElement>(".provider-list");
    if (list) list.innerHTML = `<p class="empty-provider-list">${escapeHtml(errorText(error, "读取供应商列表失败"))}</p>`;
  }
}

async function fetchProviderModels() {
  const baseUrl = document.querySelector<HTMLInputElement>("#provider-url")?.value.trim() || "";
  const apiKey = document.querySelector<HTMLInputElement>("#provider-key")?.value ?? state.providerApiKey;
  state.providerApiKey = apiKey;
  const button = document.querySelector<HTMLButtonElement>("#fetch-models");
  const feedback = document.querySelector<HTMLElement>("#model-feedback");
  if (!baseUrl || !apiKey) { toast("请先填写接口地址和 API Key", true); return; }
  if (!baseUrl.startsWith("https://")) { toast("供应商接口必须使用 HTTPS", true); return; }
  if (!isNative()) { toast("请在绿色桌面版中获取上游模型", true); return; }
  if (button) { button.disabled = true; button.textContent = "获取中…"; }
  if (feedback) feedback.textContent = "正在读取上游模型列表…";
  try {
    const result = await invoke<UpstreamModels>("fetch_codexpp_models", { baseUrl, apiKey });
    if (!result.models.length) throw new Error("上游没有返回可用模型");
    state.providerModels = [...new Set(result.models)];
    const input = document.querySelector<HTMLInputElement>("#provider-model");
    if (input) {
      const selected = result.defaultModel && state.providerModels.includes(result.defaultModel)
        ? result.defaultModel
        : state.providerModels.includes(input.value) ? input.value : "";
      input.value = selected;
    }
    document.querySelector("#provider-models")!.innerHTML = renderProviderModels();
    if (feedback) feedback.textContent = result.defaultModel && state.providerModels.includes(result.defaultModel)
      ? `已获取 ${state.providerModels.length} 个模型；已按上游默认值选择 ${result.defaultModel}。`
      : `已获取 ${state.providerModels.length} 个模型；上游未提供默认值，已保留当前选择，请手动选择默认模型。`;
  } catch (error) {
    if (feedback) feedback.textContent = errorText(error, "获取模型失败");
  } finally {
    if (button?.isConnected) { button.disabled = false; button.textContent = "获取模型"; }
  }
}

async function probeProviderProtocol() {
  const baseUrl = document.querySelector<HTMLInputElement>("#provider-url")?.value.trim() || "";
  const apiKey = document.querySelector<HTMLInputElement>("#provider-key")?.value ?? state.providerApiKey;
  const model = document.querySelector<HTMLInputElement>("#provider-model")?.value.trim() || "";
  const button = document.querySelector<HTMLButtonElement>("#probe-protocol");
  const feedback = document.querySelector<HTMLElement>("#model-feedback");
  if (!baseUrl || !apiKey || !model) {
    toast("请先填写接口地址、API Key 和模型", true);
    return;
  }
  if (!isNative()) {
    toast("请在绿色桌面版中检测供应商协议", true);
    return;
  }
  if (button) {
    button.disabled = true;
    button.textContent = "检测中…";
  }
  if (feedback) feedback.textContent = "正在分别测试 Responses API 和 Chat Completions API…";
  try {
    const result = await invoke<{ responses: boolean; chatCompletions: boolean; recommendedMode: string }>(
      "probe_provider_protocol",
      { baseUrl, apiKey, model },
    );
    if (result.responses) {
      state.providerProtocol = "responses";
      const select = document.querySelector<HTMLSelectElement>("#provider-protocol");
      if (select) select.value = "responses";
      if (feedback) feedback.textContent = "检测通过：当前模型支持 Responses API，可使用纯 API 直连。";
    } else if (result.chatCompletions) {
      state.providerProtocol = "chatCompletions";
      const select = document.querySelector<HTMLSelectElement>("#provider-protocol");
      if (select) select.value = "chatCompletions";
      if (feedback) feedback.textContent = "检测结果：当前模型支持 Chat Completions，配置助手网关后使用。";
    } else {
      if (feedback) feedback.textContent = "检测失败：当前模型未通过 Responses 或 Chat Completions 接口检测。";
    }
  } catch (error) {
    if (feedback) feedback.textContent = errorText(error, "协议检测失败");
  } finally {
    if (button?.isConnected) {
      button.disabled = false;
      button.textContent = "检测协议";
    }
  }
}

async function updateGatewayStatus() {
  const status = document.querySelector<HTMLElement>("#gateway-status");
  if (!status) return;
  const gatewaySelected = state.providerAccessMode === "gateway";
  status.hidden = !gatewaySelected;
  if (!gatewaySelected) return;
  if (!isNative()) {
    status.textContent = "本地网关：桌面版运行时可用";
    return;
  }
  try {
    const result = await invoke<{ configured: boolean; running: boolean; address: string; upstreamBaseUrl?: string }>(
      "local_gateway_status",
    );
    status.textContent = result.running
      ? `本地网关：运行中 · 监听 ${result.address}`
      : `本地网关：尚未启动 · 预计监听 ${result.address}（点击一键配置后启动）`;
  } catch (error) {
    status.textContent = `本地网关：状态读取失败 · ${errorText(error, "请稍后重试")}`;
  }
}

async function openKeyPurchasePage() {
  const url = "https://api.jianguo.ltd/llm.html";
  try {
    if (isNative()) await invoke("open_external_url", { url });
    else window.open(url, "_blank", "noopener,noreferrer");
  } catch (error) {
    toast(errorText(error, "无法打开开通页面"), true);
  }
}

function productRow(product: Product) {
  const tag = product.ready ? "已安装" : product.checked ? "未安装" : "检测中";
  const detail = product.ready && product.path ? `${product.detail} · ${product.path}` : product.detail;
  const codexppNote = product.key === "codexpp"
    ? `<small class="product-note">可选安装：使用 Responses API 直连供应商时，无需安装 Codex++；如需网关分流、协议转换或其他增强功能，请安装。</small>`
    : "";
  const storeInstall = product.key === "codex" && state.platform === "windows"
    ? `<button role="menuitem" data-product-action="store-install" data-key="codex">微软商店安装</button>`
    : "";
  const launch = product.ready && product.key !== "codex"
    ? `<button class="button quiet product-launch" data-launch-product="${product.key}">${icon(product.running ? "RefreshCw" : "Play")}${product.running ? "重启" : "启动"} ${product.name}</button>`
    : "";
  const reset = product.key === "codex"
    ? `<button role="menuitem" data-product-action="reset" data-key="codex">恢复初始状态（保留记录）</button>` : "";
  const localize = product.ready && product.key === "codex"
    ? `<button id="localize-codex" role="menuitem" data-product-action="localize" data-key="codex" title="直接切换为简体中文，无需重装。将重启 Codex，请先保存工作。" ${state.busy ? "disabled" : ""}>汉化</button>` : "";
  return `<article class="product-row"><div class="product-icon ${product.key}">${product.key === "codex" ? "C" : "C+"}</div><div class="product-main"><div class="product-name">${product.name}<span class="product-tag">${tag}</span></div><p title="${escapeHtml(product.path)}">${escapeHtml(detail)}</p>${codexppNote}</div><div class="install-actions">${launch}<button class="button outline install-one" data-key="${product.key}" ${product.ready ? "disabled" : ""}>${product.ready ? "已安装" : "安装"}</button></div><div class="product-actions"><button class="more-button" aria-label="${product.name}更多操作" aria-haspopup="menu" aria-expanded="false" title="更多操作"><span class="more-icon" aria-hidden="true"><i></i><i></i><i></i></span></button><div class="product-menu" role="menu" hidden>${storeInstall}${localize}${reset}<button role="menuitem" data-product-action="reinstall" data-key="${product.key}" ${product.ready ? "" : "disabled"}>重装</button><button role="menuitem" class="danger" data-product-action="uninstall" data-key="${product.key}" ${product.ready ? "" : "disabled"}>删除</button></div></div></article>`;
}

async function localizeInstalledCodex() {
  if (!isNative() || state.busy) return;
  await runTask("Codex 汉化", async () => {
    const detail = document.querySelector<HTMLElement>("#progress-detail");
    if (detail) detail.textContent = "正在保存中文设置并重新启动 Codex，请等待中文界面加载…";
    rememberTaskProgress();
    const result = await invoke<string>("localize_codex");
    await checkInstallations();
    toast(result);
  });
}

function closeProductMenus() {
  document.querySelectorAll<HTMLElement>(".product-menu").forEach(menu => { menu.hidden = true; });
  document.querySelectorAll<HTMLElement>(".product-row").forEach(row => row.classList.remove("menu-open"));
  document.querySelectorAll<HTMLButtonElement>(".more-button").forEach(button => button.setAttribute("aria-expanded", "false"));
}

async function checkInstallations() {
  if (!isNative()) {
    state.products.forEach(product => { product.checked = true; product.ready = false; product.path = ""; });
    return;
  }
  const results = await invoke<InstallationDetectionResult>("check_installations");
  for (const product of state.products) {
    const detected = results[product.key];
    product.ready = detected.installed;
    product.checked = true;
    product.path = detected.path;
    product.running = product.ready
      ? await invoke<boolean>("product_process_running", { product: product.key }).catch(() => false)
      : false;
  }
}

async function activateCodexProvider(providerId: string) {
  try {
    await invoke("set_codexpp_active_provider", { providerId });
    state.providers = state.providers.map(provider => ({ ...provider, isActive: provider.id === providerId }));
    renderPage();
    toast("已切换当前供应商");
  } catch (error) {
    toast(errorText(error, "切换供应商失败"), true);
  }
}

async function startCodexppWithProvider(providerId: string) {
  if (providerId && !state.providers.find(provider => provider.id === providerId)?.isActive) {
    try {
      await invoke("set_codexpp_active_provider", { providerId });
      state.providers = state.providers.map(provider => ({ ...provider, isActive: provider.id === providerId }));
    } catch (error) {
      toast(errorText(error, "切换供应商失败"), true);
      return;
    }
  }
  await startOrRestartProduct("codexpp");
}

async function startOrRestartProduct(key: ProductKey) {
  const product = state.products.find(item => item.key === key);
  if (!product?.ready) {
    toast(`${product?.name || key} 尚未安装`, true);
    return;
  }
  if (key === "codex") {
    if (state.busy || state.fullLaunchStarting) return;
    if (state.fullLaunch && !state.proxyAgreementAcceptedThisRun) {
      state.fullLaunchLaunchAfterAgreement = true;
      openProxyAgreement();
      return;
    }
    state.fullLaunchStarting = state.fullLaunch;
    if (state.page === "install" && state.fullLaunch) renderPage();
    try {
      const nodeName = state.fullLaunch ? await enableFullLaunchProxy() : "";
      await invoke("launch_installed_product", { product: "codex", restart: product.running });
      product.running = true;
      if (state.page === "install") renderPage();
      toast(state.fullLaunch
        ? `已开启美国节点「${nodeName}」的网络代理，Codex 已启动`
        : "Codex 已启动");
    } catch (error) {
      toast(errorText(error, "Codex 启动失败"), true);
    } finally {
      state.fullLaunchStarting = false;
      if (state.page === "install") renderPage();
    }
    return;
  }
  const activeProvider = state.providers.find(provider => provider.isActive);
  if (activeProvider?.isGateway || state.providers.some(provider => provider.isCurrent)) {
    // Codex++ gateway already owns provider selection; do not replace it with
    // the first reachable profile when starting the application.
  } else {
    const onlineProvider = state.providers.find(provider => provider.isActive && state.providerHealth[provider.id]?.state === "online")
    || state.providers.find(provider => state.providerHealth[provider.id]?.state === "online");
    if (onlineProvider && !onlineProvider.isActive) {
      try {
        await invoke("set_codexpp_active_provider", { providerId: onlineProvider.id });
        state.providers = state.providers.map(provider => ({ ...provider, isActive: provider.id === onlineProvider.id }));
      } catch (error) {
        toast(errorText(error, "切换到可用供应商失败"), true);
        return;
      }
    }
  }
  if (product.running && !window.confirm(`${product.name} 正在运行。重启会关闭当前会话，确定继续吗？`)) return;
  try {
    await invoke("launch_installed_product", { product: key, restart: product.running });
    product.running = true;
    if (state.page === "install") renderPage();
    toast(product.running ? `${product.name} 已启动` : `${product.name} 已启动`);
  } catch (error) {
    toast(errorText(error, `${product.name} 启动失败`), true);
  }
}

function isUsProxyProfile(profile: ProxyProfile) {
  return /\b(?:us|usa|united states|america)\b|美国|美西|美东|洛杉矶|圣何塞|纽约|西雅图|达拉斯|芝加哥|迈阿密|凤凰城|硅谷|🇺🇸/i.test(profile.remarks);
}

async function enableFullLaunchProxy() {
  if (!isNative()) throw new Error("满配启动需要在桌面应用中运行");
  if (!state.account) throw new Error("满配启动需要先登录");
  await prepareProxySubscriptionForStart();
  const usNodes = visibleProxyProfiles().filter(isUsProxyProfile);
  if (!usNodes.length) throw new Error("订阅中没有识别到美国节点，请更新订阅后重试。未启动 Codex。");
  let node = usNodes.find(profile => profile.isCurrent);
  if (!node) {
    const random = new Uint32Array(1);
    crypto.getRandomValues(random);
    node = usNodes[random[0] % usNodes.length];
    const selectedId = node.indexId;
    await invoke("select_proxy_profile", { profileId: selectedId });
    await loadProxyState();
    node = state.proxy.profiles.find(profile => profile.indexId === selectedId);
    if (!node?.isCurrent) throw new Error("代理服务未确认美国节点选择。未启动 Codex。");
  }
  if (!state.proxy.systemProxyEnabled) {
    const enabled = await toggleSystemProxy(true);
    if (!enabled) throw new Error(state.proxy.message || "开启浏览器代理失败。未启动 Codex。");
  }
  return node.remarks;
}

let accountSessionGeneration = 0;
let pendingAccountKey: Promise<string> | null = null;
async function ensureAccountApiKey(): Promise<string> {
  if (state.nutSubscriptionKami) return state.nutSubscriptionKami;
  if (pendingAccountKey) return pendingAccountKey;
  const pending = requestAccountApiKey();
  pendingAccountKey = pending;
  try { return await pending; } finally { if (pendingAccountKey === pending) pendingAccountKey = null; }
}
async function requestAccountApiKey() {
  const session = accountSessionGeneration;
  state.personal.keyBusy = true;
  if (state.page === "account") renderPage();
  try {
    const result = await accountRequest("ensureKey");
    if (session !== accountSessionGeneration || !state.account) throw new Error("登录状态已变化，请重试");
    if (!result.ok || !result.apiKey) throw new Error(result.message || "账户 API Key 创建失败");
    state.nutSubscriptionKami = result.apiKey;
    await syncBuiltinPluginCredentials();
    if (result.account) state.account = { id: result.account.id, username: result.account.username, email: result.account.email, mainKamiMasked: result.account.mainKamiMasked, hasApiKey: true };
    if (typeof result.balance === "number") state.personal.balance = result.balance;
    state.personal.error = "";
    return result.apiKey;
  } finally {
    state.personal.keyBusy = false;
    if (state.page === "account") renderPage();
  }
}

function renderAccountPage(content: HTMLElement) {
  const personal = state.personal;
  const balance = personal.balance === null ? "--" : `${personal.balance.toLocaleString("zh-CN", { maximumFractionDigits: 2 })} 点`;
  const kami = state.nutSubscriptionKami;
  const maskedKami = kami ? state.personal.revealLoginKami ? kami : `${kami.slice(0, 3)}${"*".repeat(Math.max(4, kami.length - 6))}${kami.slice(-3)}` : state.account?.mainKamiMasked || "尚未添加 API Key";
  content.innerHTML = `<div class="page-heading"><div><h1>个人中心</h1><p class="muted">查看账户资料与余额，管理充值</p></div><div class="account-header-actions"><button class="button primary" id="account-web-center">${icon("Globe")}<span>网页账户中心</span></button><button class="button quiet" id="account-refresh" ${personal.loading ? "disabled" : ""}>${icon("RefreshCw")}<span>${personal.loading ? "刷新中…" : "刷新账户"}</span></button></div></div>
    ${personal.error ? `<p class="account-error" role="alert">${escapeHtml(personal.error)}</p>` : ""}
    <div class="account-layout"><aside class="account-subnav"><p>个人中心</p><div class="account-tabs" role="tablist" aria-label="个人中心" aria-orientation="vertical"><button id="account-info-tab" type="button" role="tab" aria-controls="account-info-panel" aria-selected="${personal.section === "info"}" tabindex="${personal.section === "info" ? 0 : -1}" data-account-section="info">${icon("User")}<span><b>个人信息</b><small>账户资料与可用余额</small></span></button><button id="account-recharge-tab" type="button" role="tab" aria-controls="account-recharge-panel" aria-selected="${personal.section === "recharge"}" tabindex="${personal.section === "recharge" ? 0 : -1}" data-account-section="recharge">${icon("Wallet")}<span><b>用户充值</b><small>在线支付与卡密充值</small></span></button><button id="account-usage-tab" type="button" role="tab" aria-controls="account-usage-panel" aria-selected="${personal.section === "usage"}" tabindex="${personal.section === "usage" ? 0 : -1}" data-account-section="usage">${icon("Monitor")}<span><b>用量</b><small>当天、7天、30天与全部</small></span></button></div></aside><div class="account-section-content">
    <div id="account-info-panel" role="tabpanel" aria-labelledby="account-info-tab" ${personal.section === "info" ? "" : "hidden"}>
    <section class="account-profile" aria-labelledby="account-profile-title"><div class="panel-title"><div><h2 id="account-profile-title">账户资料</h2><p>登录身份与余额信息</p></div></div>
    <dl class="account-profile-list"><div><dt>登录账户</dt><dd>${escapeHtml(state.account?.username || "坚果用户")}</dd></div><div><dt>登录卡密 / API Key</dt><dd class="account-login-key"><span>${escapeHtml(maskedKami)}</span>${kami ? `<button id="toggle-login-kami" type="button" class="key-visibility" aria-label="${state.personal.revealLoginKami ? "隐藏登录卡密" : "显示登录卡密"}">${state.personal.revealLoginKami ? "隐藏" : "显示"}</button>` : state.loginMode === "password" ? `<button id="account-add-key" type="button" class="button primary" ${personal.keyBusy ? "disabled" : ""}>${personal.keyBusy ? "添加中…" : "添加 API Key"}</button>` : ""}</dd></div><div class="account-profile-balance"><dt>可用余额</dt><dd class="account-balance"><b>${escapeHtml(balance)}</b></dd></div></dl>
    <p class="account-credit-note">当前卡密即 API Key；充值点数是预存的大模型算力额度，使用模型服务时按实际用量扣点。</p></section>
    </div>
    <div id="account-recharge-panel" role="tabpanel" aria-labelledby="account-recharge-tab" ${personal.section === "recharge" ? "" : "hidden"}>
    <section class="account-recharge"><div class="panel-title"><div><h2>用户充值</h2><p>支付宝扫码充值，点数自动到账；已有充值卡密也可直接核销。</p></div></div><div class="recharge-tabs" role="tablist" aria-label="充值方式"><button id="online-recharge" type="button" role="tab" aria-controls="wallet-online" aria-selected="${personal.rechargeMethod === "online"}" tabindex="${personal.rechargeMethod === "online" ? 0 : -1}" data-recharge-method="online"><b>在线充值</b><small>支付宝扫码 · 自动到账</small></button><button id="card-recharge" type="button" role="tab" aria-controls="wallet-card" aria-selected="${personal.rechargeMethod === "card"}" tabindex="${personal.rechargeMethod === "card" ? 0 : -1}" data-recharge-method="card"><b>卡密充值</b><small>已有卡密直接核销</small></button></div>
    <div id="wallet-online" role="tabpanel" aria-labelledby="online-recharge" ${personal.rechargeMethod === "online" ? "" : "hidden"}></div>
    <div id="wallet-card" role="tabpanel" aria-labelledby="card-recharge" ${personal.rechargeMethod === "card" ? "" : "hidden"}><form id="account-recharge-form" class="account-form"><label for="recharge-kami">充值卡密</label><input id="recharge-kami" autocomplete="off" placeholder="输入一张新的充值卡密" ${kami ? "" : "disabled"}><div class="account-actions"><button class="button primary" type="submit" ${kami ? "" : "disabled"}>立即充值</button></div><p class="account-status" id="recharge-status" aria-live="polite">${kami ? "一次核销一张卡密，点数计入当前账户，登录卡密保持不变。" : "请先在个人信息中添加 API Key，再进行账户充值。"}</p></form></div></section>
    </div><div id="account-usage-panel" role="tabpanel" aria-labelledby="account-usage-tab" ${personal.section === "usage" ? "" : "hidden"}><div id="account-usage-content"></div></div></div></div>
    `;
  document.querySelector<HTMLButtonElement>("#account-refresh")!.addEventListener("click", () => void refreshPersonalCenter());
  document.querySelector<HTMLButtonElement>("#toggle-login-kami")?.addEventListener("click", () => {
    state.personal.revealLoginKami = !state.personal.revealLoginKami;
    renderPage();
  });
  document.querySelector<HTMLButtonElement>("#account-web-center")!.addEventListener("click", () => void openAccountCenter());
  document.querySelector<HTMLButtonElement>("#account-add-key")?.addEventListener("click", async () => {
    try { await ensureAccountApiKey(); toast("账户 API Key 已添加，可用于一键配置"); }
    catch (error) { toast(errorText(error, "添加 API Key 失败"), true); }
  });
  const tabs = [...content.querySelectorAll<HTMLButtonElement>("[data-recharge-method]")];
  tabs.forEach(button => button.addEventListener("click", () => {
    personal.rechargeMethod = button.dataset.rechargeMethod as "online" | "card";
    tabs.forEach(tab => { tab.setAttribute("aria-selected", String(tab === button)); tab.tabIndex = tab === button ? 0 : -1; });
    content.querySelector<HTMLElement>("#wallet-online")!.hidden = personal.rechargeMethod !== "online";
    content.querySelector<HTMLElement>("#wallet-card")!.hidden = personal.rechargeMethod !== "card";
  }));
  content.querySelector(".recharge-tabs")!.addEventListener("keydown", event => {
    const key = (event as KeyboardEvent).key;
    if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(key)) return;
    event.preventDefault();
    const next = key === "Home" ? tabs[0] : key === "End" ? tabs[1] : tabs.find(tab => tab.getAttribute("aria-selected") !== "true")!;
    next.click(); next.focus();
  });
  const mountWallet = () => renderWallet(content.querySelector<HTMLElement>("#wallet-online")!, kami && state.account ? { accountId: state.account.id, kami } : null, balance => {
    if (balance === null) { void refreshPersonalCenter(); return; }
    state.personal.balance = balance;
    const label = document.querySelector("#account-info-panel .account-balance b");
    if (label) label.textContent = `${balance.toLocaleString("zh-CN", { maximumFractionDigits: 2 })} 点`;
  });
  const accountTabs = [...content.querySelectorAll<HTMLButtonElement>("[data-account-section]")];
  accountTabs.forEach(button => button.addEventListener("click", () => {
    personal.section = button.dataset.accountSection as "info" | "recharge" | "usage";
    accountTabs.forEach(tab => { tab.setAttribute("aria-selected", String(tab === button)); tab.tabIndex = tab === button ? 0 : -1; });
    content.querySelector<HTMLElement>("#account-info-panel")!.hidden = personal.section !== "info";
    content.querySelector<HTMLElement>("#account-recharge-panel")!.hidden = personal.section !== "recharge";
    content.querySelector<HTMLElement>("#account-usage-panel")!.hidden = personal.section !== "usage";
    if (personal.section === "usage") renderUsage(content.querySelector<HTMLElement>("#account-usage-content")!);
    if (personal.section === "recharge" && !content.querySelector("#wallet-online")!.childElementCount) mountWallet();
  }));
  content.querySelector(".account-tabs")!.addEventListener("keydown", event => {
    const key = (event as KeyboardEvent).key;
    if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(key)) return;
    event.preventDefault();
    const next = key === "Home" ? accountTabs[0] : key === "End" ? accountTabs[accountTabs.length - 1] : accountTabs[(accountTabs.findIndex(tab => tab.getAttribute("aria-selected") === "true") + (key === "ArrowLeft" || key === "ArrowUp" ? -1 : 1) + accountTabs.length) % accountTabs.length];
    next.click(); next.focus();
  });
  if (personal.section === "usage") renderUsage(content.querySelector<HTMLElement>("#account-usage-content")!);
  if (personal.section === "recharge") mountWallet();
  document.querySelector<HTMLFormElement>("#account-recharge-form")!.addEventListener("submit", event => { event.preventDefault(); void rechargeAccountKami(); });
}

async function refreshPersonalCenter() {
  if (!state.account) return;
  state.personal.loading = true;
  state.personal.error = "";
  renderPage();
  try {
    if (state.loginMode === "password") {
      const profile = await accountRequest("profile");
      if (!profile.ok) throw new Error(profile.message || "账户信息读取失败，请重新登录");
      if (profile.account) {
        state.account = { id: profile.account.id, username: profile.account.username, email: profile.account.email, hasApiKey: profile.account.hasApiKey, mainKamiMasked: profile.account.mainKamiMasked };
        state.nutSubscriptionKami = profile.account.mainKami || "";
        await syncBuiltinPluginCredentials();
      }
      if (typeof profile.balance === "number") state.personal.balance = profile.balance;
    }
    if (!state.nutSubscriptionKami) { state.personal.balance ??= 0; state.personal.loaded = true; return; }
    const balance = await invoke<number>("account_balance", { baseUrl: API_HOST, kami: state.nutSubscriptionKami });
    state.personal.balance = Number(balance);
    state.personal.loaded = true;
  } catch (error) {
    state.personal.error = errorText(error, "读取账户信息失败，请检查网络后重试");
  } finally {
    state.personal.loading = false;
    if (state.page === "account") renderPage();
  }
}

async function rechargeAccountKami() {
  const input = document.querySelector<HTMLInputElement>("#recharge-kami");
  const status = document.querySelector<HTMLElement>("#recharge-status");
  const redeemKami = input?.value.trim() || "";
  if (!state.nutSubscriptionKami || !state.account || !redeemKami) {
    if (status) status.textContent = "请输入有效的充值卡密。";
    return;
  }
  const button = document.querySelector<HTMLButtonElement>("#account-recharge-form button[type=submit]");
  if (button) { button.disabled = true; button.textContent = "正在核销…"; }
  if (status) status.textContent = "正在提交充值，请勿重复操作。";
  try {
    const balance = await invoke<number>("account_redeem_kami", {
      baseUrl: API_HOST,
      accountId: state.account.id,
      mainKami: state.nutSubscriptionKami,
      redeemKami,
    });
    state.personal.balance = Number(balance);
    state.personal.loaded = false;
    if (input) input.value = "";
    if (status) status.textContent = `充值成功，当前余额 ${Number(balance).toLocaleString("zh-CN", { maximumFractionDigits: 2 })} 点。`;
    toast("卡密充值成功");
    await refreshPersonalCenter();
  } catch (error) {
    if (status) status.textContent = errorText(error, "充值失败，请检查卡密后重试");
  } finally {
    if (button?.isConnected) { button.disabled = false; button.textContent = "立即充值"; }
  }
}

async function openAccountCenter(tab?: string) {
  if (!state.nutSubscriptionKami) {
    try { await invoke("open_external_url", { url: `${API_HOST}/llm.html` }); }
    catch (error) { toast(errorText(error, "打开网页账户中心失败"), true); }
    return;
  }
  try {
    await invoke("open_account_center", { baseUrl: API_HOST, kami: state.nutSubscriptionKami, tab: tab || null });
  } catch (error) {
    toast(errorText(error, "打开网页账户中心失败"), true);
  }
}

function isProxyMetadataProfile(profile: ProxyProfile) {
  return /剩余流量|下次重置|重置剩余|套餐到期|到期时间|只有新加坡.*更换客户端|直连地址|tg群|邀请好友.*返佣/i.test(profile.remarks);
}

function visibleProxyProfiles() {
  return visibleProxyProfilesFrom(state.proxy.profiles);
}

function visibleProxyProfilesFrom(profiles: ProxyProfile[]) {
  return profiles.filter(profile => !isProxyMetadataProfile(profile));
}

function proxyProfileRow(profile: ProxyProfile) {
  const testing = state.testingProxyProfileId === profile.indexId;
  const result = testing ? "测速中…" : profile.delay > 0 ? `${profile.delay} ms` : profile.delay < 0 ? "连接失败" : "未测速";
  return `<article class="node-row ${profile.isCurrent ? "selected" : ""}"><div class="node-state"></div><div class="node-info"><b>${escapeHtml(profile.remarks)}</b><small>${escapeHtml(profile.protocol)} · ${escapeHtml(profile.address)}:${profile.port}</small></div><span class="node-delay ${!testing && profile.delay > 0 && profile.delay < 500 ? "good" : ""}" aria-live="polite">${result}</span><button class="button outline" data-profile-test="${escapeHtml(profile.indexId)}">${testing ? "测速中" : "测速"}</button><button class="button ${profile.isCurrent ? "secondary" : "quiet"}" data-profile-select="${escapeHtml(profile.indexId)}">${profile.isCurrent ? "当前" : "选择"}</button></article>`;
}

async function checkNetwork() {
  const label = document.querySelector<HTMLElement>("#network-state"); if (!label) return;
  try { const response = isNative() ? await invoke<AppConfig>("fetch_site_config", { baseUrl: API_HOST }) : await fetch(`${API_HOST}/api/v1/site-config`, { signal: AbortSignal.timeout(5000) }); label.textContent = response ? "可访问" : "后台异常"; }
  catch { label.textContent = "不可访问"; }
}

async function createCodexDesktopShortcut() {
  if (!isNative() || state.platform !== "windows") return;
  try {
    await invoke("create_codex_desktop_shortcut");
  } catch (error) {
    toast(errorText(error, "Codex 已安装，但创建 ChatGPT 桌面快捷方式失败"), true);
  }
}

async function waitForInstallation(key: ProductKey, timeoutMs = 180_000) {
  const product = state.products.find(item => item.key === key)!;
  // Download completion is a different phase. The OS installer cannot be paused.
  state.downloadActive = false;
  state.downloadPaused = false;
  state.downloadSuggestionVisible = false;
  if (state.lastDownloadProgress) state.lastDownloadProgress.phase = "install";
  const pause = document.querySelector<HTMLButtonElement>("#download-pause-toggle");
  if (pause) pause.hidden = true;
  const stop = document.querySelector<HTMLButtonElement>("#download-stop");
  if (stop) { stop.hidden = false; stop.textContent = "取消等待"; }
  document.querySelector<HTMLElement>("#download-proxy-suggestion")?.setAttribute("hidden", "");
  const title = document.querySelector<HTMLElement>("#progress-title");
  if (title) title.textContent = `${product.name} 安装检测`;
  const percent = document.querySelector<HTMLElement>("#progress-percent");
  if (percent) percent.textContent = "下载完成";
  const detail = document.querySelector<HTMLElement>("#progress-detail");
  if (detail) detail.textContent = state.platform === "macos"
    ? `请将安装镜像中的 ${product.name} 拖入“应用程序”文件夹。安装后会自动继续；已安装时无需再次打开镜像。`
    : `正在检测 ${product.name} 安装结果。如有安装窗口，请完成安装步骤。`;
  rememberTaskProgress();
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (state.downloadCancelled) throw new Error("DOWNLOAD_CANCELLED");
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([checkInstallations(), new Promise<never>((_, reject) => {
        timer = setTimeout(() => reject(new Error("安装检测超时，请稍后点击重新检测；已下载的安装包会保留。")), 35_000);
      })]);
    } finally { clearTimeout(timer); }
    if (state.downloadCancelled) throw new Error("DOWNLOAD_CANCELLED");
    if (state.products.find(item => item.key === key)?.ready) {
      if (key === "codex") await createCodexDesktopShortcut();
      return true;
    }
    await delay(1000);
  }
  return false;
}

async function installProduct(key: ProductKey, force = false, source: "default" | "store" = "default") {
  if (state.busy) return false;
  const product = state.products.find(item => item.key === key)!;
  if (!force && product.checked && product.ready && !(key === "codex" && state.platform === "windows")) {
    toast(`${product.name} 已安装，已跳过`);
    return true;
  }
  return runTask(`${product.name} 安装`, async () => {
    if (key === "codex" && source === "default" && state.platform === "windows") {
      await waitForInstallLocationSave();
      await invoke<string>("install_codex_windows_fallback", { arch: state.arch, proxyUrl: installerProxyUrl() });
      const installed = await waitForInstallation(key, 30_000);
      renderPage();
      if (!installed) throw new Error("Codex 安装程序已完成，但暂未检测到程序，请稍后点击重新检测。");
      toast("Codex 桌面版已安装；若开始菜单暂未显示，请注销并重新登录 Windows。");
      return;
    }
    const result = await launchProductInstall(key);
    if (result === "launched") {
      const installed = await waitForInstallation(key);
      renderPage();
      if (!installed) throw new Error(`${product.name} 安装程序已启动，但暂未检测到安装完成，请确认安装窗口已完成。`);
      toast(`${product.name} 已安装`);
      return;
    }
    if (result !== "installed") {
      const installed = await waitForInstallation(key);
      renderPage();
      if (!installed) throw new Error(result === "manual" ? "安装镜像已打开，但暂未检测到安装完成，请完成安装后重试。" : "安装程序已启动，但暂未检测到安装完成。");
      toast(`${product.name} 已安装`);
      return;
    }
    const installed = await waitForInstallation(key, 30_000);
    renderPage();
    if (!installed) throw new Error(`尚未检测到 ${product.name}，请完成安装后重试。`);
    toast(`${product.name} 安装完成`);
  });
}

async function launchProductInstall(key: ProductKey) {
  if (key === "codex") await waitForInstallLocationSave();
  const product = state.products.find(item => item.key === key)!;
  const os = state.platform;
  const arch = state.arch;
  if (!isNative()) throw new Error("请在桌面应用中执行安装；网页预览仅展示界面。");
  const keys: Record<ProductKey, string[]> = { codex: [`codex_${os}_${arch}_url`, "codex_download_url"], codexpp: [`codexplusplus_${os}_${arch}_url`, "codexplusplus_download_url"] };
  const url = key === "codexpp"
    ? "https://api.github.com/repos/BigPizzaV3/CodexPlusPlus/releases/latest"
    : keys[key].map(k => state.config[k]).find(Boolean) || "https://portable.invalid/";
  const silentArgs = key === "codexpp" ? state.config.codexplusplus_silent_args || "" : "";
  const configPrefix = key === "codexpp" ? "codexplusplus" : key;
  const sha256 = key === "codexpp" ? "" : state.config[`${configPrefix}_${os}_${arch}_sha256`] || state.config[`${configPrefix}_sha256`] || "";
  if (sha256 && !/^[a-f0-9]{64}$/i.test(sha256)) throw new Error(`${product.name} 的 SHA-256 格式无效，请检查管理后台配置`);
  if (url !== "https://portable.invalid/" && !/^https?:\/\//i.test(url)) throw new Error(`${product.name} 下载地址无效，请在管理后台配置有效的下载链接`);
  return invoke<string>("install_product", { product: key, platform: os, arch, url, mode: key === "codexpp" ? state.installMode : "interactive", silentArgs, sha256, proxyUrl: installerProxyUrl() });
}

function installerProxyUrl(): string | null {
  if (!state.proxy.systemProxyEnabled) return null;
  const address = state.proxy.httpAddress;
  if (!address || address === "未监听" || !/^(?:127\.0\.0\.1|localhost|\[::1\]):\d+$/.test(address)) {
    throw new Error("网络代理已开启，但未检测到本机 HTTP 代理监听端口；请刷新代理状态后重试。");
  }
  return `http://${address}`;
}

function openCodexResetDialog() {
  if (state.busy) return;
  if (!isNative()) { toast("请在桌面应用中恢复 Codex", true); return; }
  showCodexResetDialog(async () => {
    if (state.busy) throw new Error("请等待当前任务完成后重试");
    state.busy = true;
    renderPage();
    try {
      const report = await invoke<CodexResetReport>("reset_codex_initial_state");
      for (const provider of state.providers) { provider.isActive = false; provider.isCurrent = false; }
      const codex = state.products.find(product => product.key === "codex");
      if (codex) codex.running = false;
      return report;
    } finally { state.busy = false; renderPage(); }
  });
}

async function uninstallProduct(key: ProductKey) {
  if (state.busy) return;
  const product = state.products.find(item => item.key === key)!;
  if (!product.ready) return;
  if (!window.confirm(`确定删除 ${product.name}？将卸载程序，并保留 Codex 会话、技能、插件和个人设置。`)) return;
  if (!isNative()) { toast("删除操作仅在绿色桌面版中可用", true); return; }
  await runTask(`${product.name} 删除`, async () => {
    const result = await invoke<string>("uninstall_product", { product: key });
    await checkInstallations();
    renderPage();
    toast(result === "removed" ? `${product.name} 已删除` : `${product.name} 的卸载程序已启动`);
  });
}

async function beginSetup() {
  if (state.busy || state.setupAfterAgreement) return;
  if ((state.setupEnableProxy || state.fullLaunch) && !state.proxyAgreementAcceptedThisRun) {
    state.setupAfterAgreement = true;
    renderPage();
    openProxyAgreement();
    return;
  }
  if (!state.setupConfigureNut) {
    await installAll();
    return;
  }
  const existingNut = state.providers.find(provider => !provider.isGateway && provider.baseUrl.replace(/\/+$/, "") === "https://api.jianguo.ltd/v1");
  if (state.loginMode === "password") {
    try { await ensureAccountApiKey(); }
    catch (error) { toast(errorText(error, "无法获取账户 API Key，请稍后重试"), true); return; }
  }
  const apiKey = state.loginMode === "password" ? state.nutSubscriptionKami : existingNut?.apiKey || state.nutSubscriptionKami;
  if (apiKey) {
    await installAll({
      name: existingNut?.name || "坚果",
      baseUrl: "https://api.jianguo.ltd/v1",
      model: existingNut?.model || "",
      models: existingNut?.models || [],
      apiKey,
      providerId: existingNut?.id || null,
      protocol: "responses",
      relayMode: "pureApi",
      ...setupProviderOptions(),
    }, true);
    return;
  }
  state.setupEditor = true;
  state.page = "config";
  renderDashboard();
  openProviderEditor(existingNut?.id);
  toast("缺少坚果 API Key，请填写密钥；使用卡密登录可自动带入。", true);
}

function setupProviderOptions() {
  const { configureCodexAccess, configureCodexChinese, configureCodexppChinese, configureCodexppStartCodex } = state;
  return { configureCodexAccess, configureCodexChinese, configureCodexppChinese, configureCodexppStartCodex };
}

async function installAll(configuration?: ProviderConfiguration, fetchModels = false) {
  if (state.busy) return;
  if (!isNative()) { toast("请在桌面应用中执行安装与配置", true); return false; }
  const includeCodexpp = state.includeCodexpp;
  const includeCodex = state.includeCodex;
  const enableProxy = state.setupEnableProxy;
  state.setupProxyOpenedForTask = false;
  state.setupProxyCoreWasRunning = false;
  state.setupMessage = "正在检测已安装的软件…";
  state.setupFailed = false;
  const success = await runTask("一键安装并配置", async () => {
    await waitForInstallLocationSave();
    await checkInstallations();
    if (!enableProxy) {
      const proxyStatus = await invoke<{ enabled: boolean; assistantManaged: boolean }>("system_proxy_state");
      if (proxyStatus.enabled && proxyStatus.assistantManaged) {
        if (!await toggleSystemProxy(false)) {
          throw new Error("无法清理助手设置的系统代理；请检查权限后，在网络代理中重试关闭浏览器代理。");
        }
      }
    }
    if (configuration && !includeCodex && !state.products.find(item => item.key === "codex")?.ready) {
      throw new Error("配置坚果中转服务需要先安装 Codex，请勾选“安装 Codex”后重试。");
    }
    if (enableProxy) await enableSetupProxy();
    const apiKey = configuration?.apiKey || state.providers.find(item => item.id === configuration?.providerId)?.apiKey || "";
    if (configuration && fetchModels) {
      const detail = document.querySelector<HTMLElement>("#progress-detail");
      if (detail) detail.textContent = "正在自动获取坚果供应商模型…";
      const result = await invoke<UpstreamModels>("fetch_codexpp_models", { baseUrl: configuration.baseUrl, apiKey });
      configuration.models = [...new Set(result.models.map(model => model.trim()).filter(Boolean))];
      if (!configuration.models.length) throw new Error("坚果供应商未返回可用模型，未执行安装或配置，请稍后重试。");
      configuration.model = result.defaultModel && configuration.models.includes(result.defaultModel)
        ? result.defaultModel
        : configuration.models.includes(configuration.model) ? configuration.model : "";
      if (!configuration.model) throw new Error("上游未返回默认模型，且当前配置的模型不在可用列表中。请先在供应商配置中选择默认模型后重试。");
    } else if (configuration) {
      await invoke("check_codexpp_provider", { baseUrl: configuration.baseUrl, apiKey });
    }
    if (state.downloadCancelled) throw new Error("DOWNLOAD_CANCELLED");
    // Windows checks the saved disk and unfinished data restoration in the
    // backend even when Codex is detected. Matching installs return immediately.
    const pending = state.products.filter(item => (item.key === "codex" ? includeCodex : includeCodexpp)
      && (!item.ready || (item.key === "codex" && state.platform === "windows")));
    const failures: string[] = [];
    for (const [index, product] of pending.entries()) {
      if (state.downloadCancelled) throw new Error("DOWNLOAD_CANCELLED");
      const detail = document.querySelector<HTMLElement>("#progress-detail");
      if (detail) detail.textContent = `${index + 1} / ${pending.length}：正在安装 ${product.name}，如出现系统授权或安装窗口，请完成确认。`;
      try {
        if (product.key === "codex" && state.platform === "windows") {
          if (detail) detail.textContent = "正在直接下载 Codex MSIX 安装包…";
          await invoke("install_codex_windows_fallback", { arch: state.arch, proxyUrl: installerProxyUrl() });
        } else {
          await launchProductInstall(product.key);
        }
        const installed = await waitForInstallation(product.key);
        if (!installed) failures.push(`${product.name} 尚未检测到安装，请完成安装窗口后重试`);
      } catch (error) {
        if (errorText(error, "").includes("DOWNLOAD_CANCELLED")) throw error;
        failures.push(`${product.name}：${errorText(error, "安装失败，请重试")}`);
      }
    }
    if (failures.length) throw new Error(failures.join("；"));
    // Re-running setup also repairs a missing desktop shortcut without
    // downloading or reinstalling an already installed Codex desktop app.
    if (includeCodex && !pending.some(product => product.key === "codex") && state.products.find(product => product.key === "codex")?.ready) {
      await createCodexDesktopShortcut();
    }
    if (state.downloadCancelled) throw new Error("DOWNLOAD_CANCELLED");
    if (state.fullLaunch && !state.products.find(item => item.key === "codex")?.ready) {
      throw new Error("满配启动需要先安装 Codex，请勾选“安装 Codex”后重试。");
    }
    if (state.setupProxyOpenedForTask && state.proxy.systemProxyEnabled) {
      const closed = await toggleSystemProxy(false, { stopCore: !state.setupProxyCoreWasRunning });
      if (!closed) throw new Error(state.proxy.message || "无法关闭安装期间使用的网络代理");
      state.setupProxyOpenedForTask = false;
    }
    const detail = document.querySelector<HTMLElement>("#progress-detail");
    if (detail) detail.textContent = configuration ? "软件已就绪，正在配置供应商…" : "所选操作已完成";
    let message = configuration ? "" : "所选操作已完成，未修改供应商配置。";
    if (configuration) {
      if (detail) detail.textContent = "正在关闭 Codex/ChatGPT 及后台进程，准备写入配置…";
      await invoke("stop_codex_processes");
    }
    if (configuration) {
      message = await invoke<string>("configure_codexpp_provider", configuration);
    }
    state.setupMessage = `${includeCodex || includeCodexpp ? "软件已就绪。" : ""}${message || "未修改供应商配置。"}`;
    if (configuration) {
      state.providers = await invoke<CodexProvider[]>("list_codexpp_providers");
      state.providersLoaded = true;
    }
    if (configuration || state.fullLaunch) {
      let fullLaunchNode = "";
      if (state.fullLaunch) {
        if (detail) detail.textContent = "正在选择美国节点并开启浏览器代理…";
        fullLaunchNode = await enableFullLaunchProxy();
      }
      if (detail) detail.textContent = state.fullLaunch
        ? `美国节点「${fullLaunchNode}」已就绪，正在启动 Codex…`
        : configuration ? "供应商配置已写入，正在启动 Codex…" : "正在启动 Codex…";
      await invoke("launch_installed_product", { product: "codex", restart: false });
      if (configuration?.configureCodexAccess) {
        await finishCodexSetup({
          readState: () => invoke<CodexSetupState>("codex_setup_state"),
          stop: () => invoke("stop_codex_processes"),
          configure: () => invoke<string>("configure_codexpp_provider", configuration),
          launch: () => invoke("launch_installed_product", { product: "codex", restart: false }),
          selectFullAccess: () => invoke("apply_codex_full_access"),
          sleep: ms => new Promise(resolve => window.setTimeout(resolve, ms)),
          now: Date.now,
          cancelled: () => state.downloadCancelled,
          report: message => { if (detail) detail.textContent = message; },
        });
      }
      const codex = state.products.find(item => item.key === "codex");
      if (codex) codex.running = true;
      message = `${message}${state.fullLaunch ? ` 已开启满配启动：美国节点「${fullLaunchNode}」和浏览器代理已启用。` : ""} Codex 已启动。`;
      state.setupMessage = message;
    }
  });
  if (!success && !state.taskWasCancelled) {
    state.setupFailed = true;
    state.setupMessage = document.querySelector("#progress-detail")?.textContent || "安装或配置未完成，请检查后重试。";
  }
  if (state.setupProxyOpenedForTask && state.proxy.systemProxyEnabled) {
    try {
      const closed = await toggleSystemProxy(false, { stopCore: !state.setupProxyCoreWasRunning });
      if (!closed) throw new Error(state.proxy.message || "网络代理关闭失败");
      state.setupMessage = `${state.setupMessage} 一键准备期间使用的网络代理已关闭，原系统代理设置已恢复。`;
    } catch (error) {
      state.setupFailed = true;
      state.setupMessage = `${state.setupMessage} 自动关闭网络代理失败：${errorText(error, "请到网络代理页面手动关闭")}`;
    }
    state.setupProxyOpenedForTask = false;
  }
  if (success) {
    state.setupEditor = false;
    state.providerEditorOpen = false;
    state.editingProviderId = "";
    state.providerPreset = null;
    state.page = "install";
    renderDashboard();
  } else if (state.page === "config") {
    toast(state.setupMessage || "操作已停止，可重试；已安装的软件不会重复安装。", true);
  }
  if (state.page === "install") renderPage();
  return success;
}

async function enableSetupProxy() {
  const detail = document.querySelector<HTMLElement>("#progress-detail");
  if (detail) detail.textContent = "正在加载订阅并开启网络代理…";
  if (!state.proxyAgreementAcceptedThisRun) throw new Error("请先阅读并同意网络代理合规须知。");
  await prepareProxySubscriptionForStart();
  if (!state.proxy.ready) throw new Error(state.proxy.message || "代理服务未就绪，请到网络代理页面检查后重试。");
  if (state.proxy.systemProxyEnabled) {
    state.setupProxyCoreWasRunning = state.proxy.running;
    state.setupProxyOpenedForTask = true;
    stopProxyBillingMonitor();
    return;
  }
  state.setupProxyCoreWasRunning = state.proxy.running;
  if (state.downloadCancelled) throw new Error("DOWNLOAD_CANCELLED");
  const candidates = visibleProxyProfiles();
  const usNodes = candidates.filter(isUsProxyProfile);
  const otherInternationalNodes = candidates.filter(profile => /\b(?:jp|japan|sg|singapore|ca|canada|uk|gb|germany|de|fr|netherlands|nl|hk|tw|kr)\b|日本|新加坡|加拿大|英国|德国|法国|荷兰|香港|台湾|韩国/i.test(profile.remarks));
  const pool = usNodes.length ? usNodes : otherInternationalNodes.length ? otherInternationalNodes : candidates;
  if (!pool.length) throw new Error("订阅中没有可用节点，请在网络代理页面更新订阅后重试。");
  const random = new Uint32Array(1);
  crypto.getRandomValues(random);
  const node = pool[random[0] % pool.length];
  await invoke("select_proxy_profile", { profileId: node.indexId });
  await loadProxyState();
  if (!state.proxy.profiles.some(profile => profile.indexId === node.indexId && profile.isCurrent)) {
    throw new Error("代理服务未确认节点选择，请在网络代理页面选择节点后重试。");
  }
  if (state.downloadCancelled) throw new Error("DOWNLOAD_CANCELLED");
  state.proxy.systemProxyMode = "auto";
  if (!await toggleSystemProxy(true, { billNutUsage: false })) throw new Error(state.proxy.message || "开启网络代理失败，请检查节点后重试。");
  state.setupProxyOpenedForTask = true;
}

async function configureCustomProvider() {
  if (state.busy) return;
  const name = document.querySelector<HTMLInputElement>("#provider-name")?.value.trim() || "";
  const baseUrl = document.querySelector<HTMLInputElement>("#provider-url")?.value.trim() || "";
  const model = document.querySelector<HTMLInputElement>("#provider-model")?.value.trim() || "";
  const apiKey = document.querySelector<HTMLInputElement>("#provider-key")?.value ?? state.providerApiKey;
  const protocol = document.querySelector<HTMLSelectElement>("#provider-protocol")?.value || "responses";
  const relayMode = state.providerAccessMode;
  state.providerApiKey = apiKey;
  if (!name || !baseUrl || !model || (!apiKey && !state.editingProviderId)) { toast("请填写供应商名称、接口、模型和 API Key", true); return; }
  if (!baseUrl.startsWith("https://")) { toast("供应商接口必须使用 HTTPS", true); return; }
  if (!isNative()) { toast("请在桌面应用中写入 Codex++ 配置", true); return; }
  const button = document.querySelector<HTMLButtonElement>("#configure-provider");
  if (button) { button.disabled = true; button.textContent = "配置中…"; }
  try {
    const models = [...new Set(state.providerModels.filter(value => value.trim()))];
    const { configureCodexAccess, configureCodexChinese, configureCodexppChinese, configureCodexppStartCodex } = state;
    if (state.setupEditor) {
      if (protocol !== "responses" && relayMode !== "gateway") {
        toast("当前协议需要选择助手网关，或改用支持 Responses API 的模型。", true);
        return;
      }
      await installAll({ name, baseUrl, model, models, apiKey, providerId: state.editingProviderId || null, protocol, relayMode, configureCodexAccess, configureCodexChinese, configureCodexppChinese, configureCodexppStartCodex });
      return;
    }
    const wasRunning = await invoke<boolean>("product_process_running", { product: "codex" });
    if (wasRunning) {
      if (!await confirmDialog("应用供应商配置", "Codex 正在运行。应用配置需要关闭并重新启动 Codex，请先保存工作。", "保存配置并重启")) return;
      await invoke("stop_codex_processes");
    }
    const message = await invoke<string>("configure_codexpp_provider", { name, baseUrl, model, models, apiKey, providerId: state.editingProviderId || null, protocol, relayMode, configureCodexAccess, configureCodexChinese, configureCodexppChinese, configureCodexppStartCodex });
    if (wasRunning) await invoke("launch_installed_product", { product: "codex", restart: false });
    state.providers = await invoke<CodexProvider[]>("list_codexpp_providers");
    state.providersLoaded = true;
    state.providerEditorOpen = false;
    state.editingProviderId = "";
    state.providerPreset = null;
    renderPage();
    toast(wasRunning ? `${message || "供应商配置完成"}。Codex 已重启，请新建会话使用所选模型。` : message || "供应商配置完成");
  } catch (error) {
    toast(errorText(error, "一键配置失败"), true);
  } finally {
    if (button?.isConnected) { button.disabled = false; button.textContent = state.setupEditor ? "一键安装并配置" : "一键配置"; }
  }
}

async function proxyApi(method: "GET" | "POST" | "PUT" | "DELETE", path: string, body?: unknown) {
  if (!isNative()) throw new Error("v2rayN 管理需要运行绿色桌面版");
  return invoke<Record<string, unknown>>("v2rayn_api", { method, path, body: body ?? null });
}

function proxyData<T>(response: Record<string, unknown>): T {
  const envelope = response.data;
  if (envelope && typeof envelope === "object" && "data" in envelope) {
    return (envelope as { data: T }).data;
  }
  return envelope as T;
}

let proxyStateLoad: Promise<void> | null = null;

async function loadProxyState(): Promise<void> {
  if (proxyStateLoad) return proxyStateLoad;
  proxyStateLoad = refreshProxyState();
  try {
    await proxyStateLoad;
  } finally {
    proxyStateLoad = null;
  }
}

function updateProxyLoading(message: string) {
  state.proxyLoadingMessage = message;
  const label = document.querySelector<HTMLElement>("#proxy-loading-message");
  if (label) label.textContent = message;
}

async function refreshProxyState() {
  if (!isNative()) {
    state.proxy.message = "代理管理仅在绿色桌面版中可用。";
    if (state.page === "proxy") renderPage();
    return;
  }
  state.proxyLoading = true;
  updateProxyLoading("正在准备本机服务并读取订阅…");
  setProxyBusy(true);
  if (state.page === "proxy") renderPage();
  try {
    await new Promise<void>(resolve => requestAnimationFrame(() => setTimeout(resolve, 0)));
    const [statusResponse, subscriptionResponse, systemProxyResponse] = await Promise.all([
      proxyApi("GET", "/api/status"),
      proxyApi("GET", "/api/subscriptions"),
      invoke<{ supported: boolean; enabled: boolean }>("system_proxy_state"),
    ]);
    const status = proxyData<{ coreRunning: boolean; listeners?: ProxyListener[] }>(statusResponse);
    state.proxy.ready = true;
    state.proxy.running = Boolean(status.coreRunning);
    state.proxy.systemProxySupported = systemProxyResponse.supported;
    state.proxy.systemProxyEnabled = systemProxyResponse.enabled;
    state.proxy.routingProfiles = proxyData<RoutingProfile[]>(await proxyApi("GET", "/api/settings/routing-profiles")) || [];
    const activeRoute = state.proxy.routingProfiles.find(route => route.isActive);
    state.proxy.selectedRoute = activeRoute?.id || "";
    if (activeRoute) localStorage.setItem("proxy-routing-profile", activeRoute.id);
    const listeners = status.listeners || [];
    const listenerAddress = (protocol: string) => {
      const listener = listeners.find(item => item.protocols.some(value => value.toLowerCase() === protocol));
      return listener ? `${listener.listenAddress}:${listener.port}` : "未监听";
    };
    state.proxy.socksAddress = listenerAddress("socks");
    state.proxy.httpAddress = listenerAddress("http");
    state.proxy.subscriptions = proxyData<ProxySubscription[]>(subscriptionResponse) || [];
    updateProxyLoading("正在同步订阅信息…");
    await ensureConfiguredSubscription();
    state.proxy.selectedSubscription ||= state.proxy.subscriptions[0]?.id || "";
    const query = state.proxy.selectedSubscription ? `?subscriptionId=${encodeURIComponent(state.proxy.selectedSubscription)}` : "";
    updateProxyLoading("正在加载节点列表…");
    const profileResponse = await proxyApi("GET", `/api/profiles${query}`);
    state.proxy.profiles = proxyData<ProxyProfile[]>(profileResponse) || [];
    if (!state.proxy.message.startsWith("已载入后台订阅")) state.proxy.message = "";
  } catch (error) {
    state.proxy.ready = false;
    state.proxy.message = errorText(error, "无法启动 v2rayN 本机服务");
  } finally {
    state.proxyLoading = false;
    state.proxyLoadingMessage = "";
    setProxyBusy(false);
    if (state.page === "proxy") renderPage();
  }
}

async function activateRoutingProfile(routeId: string) {
  if (!routeId) return;
  const requestedRoute = state.proxy.routingProfiles.find(route => route.id === routeId);
  setProxyBusy(true);
  try {
    await proxyApi("POST", `/api/settings/routing-profiles/${encodeURIComponent(routeId)}/activate`);
    await loadProxyState();
    const activeRoute = state.proxy.routingProfiles.find(route => route.id === routeId && route.isActive);
    if (!activeRoute) throw new Error("v2rayN 未确认路由模式已切换，请刷新状态后重试。");
    state.proxy.selectedRoute = activeRoute.id;
    localStorage.setItem("proxy-routing-profile", activeRoute.id);
    showProxyMessage(`已切换路由模式：${state.proxy.routingProfiles.find(route => route.id === routeId)?.remarks || "已选择"}`);
  } catch (error) {
    showProxyMessage(errorText(error, "切换路由模式失败"), true);
    if (requestedRoute && state.page === "proxy") renderPage();
  } finally {
    setProxyBusy(false);
  }
}

async function ensureConfiguredSubscription() {
  if (state.proxySubscriptionInitialized) return;
  const url = state.config.v2rayn_subscription_url?.trim();
  if (!url || state.proxy.subscriptions.some(item => item.url === url)) {
    state.proxySubscriptionInitialized = true;
    return;
  }
  if (!/^https?:\/\//i.test(url)) {
    state.proxy.message = "后台配置的 v2rayN 订阅地址无效。";
    return;
  }
  const response = await proxyApi("POST", "/api/subscriptions", { remarks: "坚果订阅", url, enabled: true });
  const created = proxyData<ProxySubscription>(response);
  if (!created?.id) throw new Error("v2rayN 未能创建后台订阅");
  state.proxy.selectedSubscription = created.id;
  state.proxy.subscriptions.push(created);
  state.proxy.message = "已载入后台坚果订阅，开启代理时将自动更新节点。";
  state.proxySubscriptionInitialized = true;
}

function isNutProxySubscription(subscription: ProxySubscription) {
  const configuredUrl = state.config.v2rayn_subscription_url?.trim();
  return configuredUrl
    ? subscription.url === configuredUrl
    : subscription.remarks === "坚果订阅";
}

async function prepareProxySubscriptionForStart() {
  await loadProxyState();
  const selected = state.proxy.subscriptions.find(item => item.id === state.proxy.selectedSubscription);
  const customSubscriptions = state.proxy.subscriptions.filter(item => !isNutProxySubscription(item));
  const subscription = (selected && !isNutProxySubscription(selected) ? selected : null)
    || customSubscriptions[0]
    || state.proxy.subscriptions.find(isNutProxySubscription)
    || selected
    || state.proxy.subscriptions[0];
  if (!subscription) throw new Error("没有可用订阅。请检查后台坚果订阅配置，或添加自己的订阅地址。");

  state.proxy.selectedSubscription = subscription.id;
  const loadProfiles = async () => proxyData<ProxyProfile[]>(
    await proxyApi("GET", `/api/profiles?subscriptionId=${encodeURIComponent(subscription.id)}`)
  ) || [];
  let profiles = await loadProfiles();
  if (!visibleProxyProfilesFrom(profiles).length) {
    state.proxy.message = `正在自动更新${isNutProxySubscription(subscription) ? "坚果" : "自定义"}订阅并加载节点…`;
    await proxyApi("POST", `/api/subscriptions/${encodeURIComponent(subscription.id)}/update?useProxy=false`);
    const deadline = Date.now() + 45_000;
    while (Date.now() < deadline && !visibleProxyProfilesFrom(profiles).length) {
      await delay(1500);
      profiles = await loadProfiles();
    }
  }
  state.proxy.profiles = profiles;
  if (!visibleProxyProfilesFrom(profiles).length) {
    throw new Error(`${isNutProxySubscription(subscription) ? "坚果" : "自定义"}订阅已自动更新，但仍没有加载到可用节点。请检查订阅状态或稍后重试。`);
  }
}

async function saveProxySubscription() {
  const input = document.querySelector<HTMLInputElement>("#subscription-url");
  const url = input?.value.trim() || "";
  if (!/^https?:\/\//i.test(url)) { showProxyMessage("请输入有效的 HTTP 或 HTTPS 订阅地址", true); return; }
  const current = state.proxy.subscriptions.find(item => item.id === state.proxy.selectedSubscription);
  setProxyBusy(true);
  try {
    const body = { remarks: "坚果订阅", url, enabled: true };
    const result = current
      ? await proxyApi("PUT", `/api/subscriptions/${encodeURIComponent(current.id)}`, body)
      : await proxyApi("POST", "/api/subscriptions", body);
    const created = proxyData<ProxySubscription>(result);
    if (created?.id) state.proxy.selectedSubscription = created.id;
    state.proxy.addingSubscription = false;
    state.proxy.message = "订阅已保存，正在拉取节点…";
    await proxyApi("POST", "/api/subscriptions/update", { subscriptionId: state.proxy.selectedSubscription, useProxy: false });
    await delay(2500);
    await loadProxyState();
    showProxyMessage("订阅更新任务已提交；节点列表会在服务完成拉取后更新。");
  } catch (error) {
    showProxyMessage(error instanceof Error ? error.message : "保存订阅失败", true);
  } finally {
    setProxyBusy(false);
  }
}

async function deleteProxySubscription() {
  const current = state.proxy.subscriptions.find(item => item.id === state.proxy.selectedSubscription);
  if (!current || !window.confirm(`删除订阅“${current.remarks}”？`)) return;
  setProxyBusy(true);
  try {
    await proxyApi("DELETE", `/api/subscriptions/${encodeURIComponent(current.id)}`);
    state.proxy.subscriptions = state.proxy.subscriptions.filter(item => item.id !== current.id);
    state.proxy.selectedSubscription = state.proxy.subscriptions[0]?.id || "";
    state.proxy.addingSubscription = false;
    await loadProxyState();
    showProxyMessage("订阅已删除。");
  } catch (error) {
    showProxyMessage(error instanceof Error ? error.message : "删除订阅失败", true);
  } finally {
    setProxyBusy(false);
  }
}

async function updateProxySubscription() {
  if (!state.proxy.selectedSubscription) return;
  setProxyBusy(true);
  try {
    await proxyApi("POST", `/api/subscriptions/${encodeURIComponent(state.proxy.selectedSubscription)}/update?useProxy=false`);
    showProxyMessage("订阅更新已提交。");
    await delay(1800);
    await loadProxyState();
  } catch (error) {
    showProxyMessage(error instanceof Error ? error.message : "订阅更新失败", true);
  } finally {
    setProxyBusy(false);
  }
}

async function selectProxyProfile(id: string) {
  setProxyBusy(true);
  try {
    if (!isNative()) throw new Error("节点切换需要运行绿色桌面版");
    await invoke("select_proxy_profile", { profileId: id });
    await loadProxyState();
    if (!state.proxy.profiles.some(profile => profile.indexId === id && profile.isCurrent)) {
      throw new Error("代理服务未确认节点切换，请刷新服务状态后重试");
    }
    showProxyMessage("节点已选中。");
  } catch (error) {
    showProxyMessage(`选择节点失败：${errorText(error, "请查看代理服务状态")}`, true);
  } finally {
    setProxyBusy(false);
  }
}

async function testProxyProfile(id: string) {
  if (state.busy && !state.installTaskActive) return;
  const profile = state.proxy.profiles.find(item => item.indexId === id);
  if (!profile) return;
  state.testingProxyProfileId = id;
  setProxyBusy(true);
  showProxyMessage(`正在测试 ${profile.remarks} 的连接延迟…`);
  try {
    await proxyApi("POST", `/api/profiles/${encodeURIComponent(id)}/latency`);
    // The latency endpoint only queues work; read results after the service finishes.
    const deadline = Date.now() + 30_000;
    while (true) {
      await delay(1000);
      const operations = proxyData<string[]>(await proxyApi("GET", "/api/operations")) || [];
      if (!operations.includes("speedtest")) break;
      if (Date.now() >= deadline) throw new Error("测速超时，请稍后重试或更换节点");
    }
    const query = state.proxy.selectedSubscription ? `?subscriptionId=${encodeURIComponent(state.proxy.selectedSubscription)}` : "";
    state.proxy.profiles = proxyData<ProxyProfile[]>(await proxyApi("GET", `/api/profiles${query}`)) || [];
    const result = state.proxy.profiles.find(item => item.indexId === id);
    if (!result) throw new Error("该节点已不存在，请刷新节点列表");
    if (result.delay <= 0) {
      result.delay = -1;
      throw new Error(`${result.remarks} 连接失败，请检查网络或更换节点`);
    }
    showProxyMessage(`${result.remarks} 测速完成：${result.delay} ms`);
  } catch (error) {
    showProxyMessage(errorText(error, "测速失败，请稍后重试"), true);
  } finally {
    state.testingProxyProfileId = "";
    setProxyBusy(false);
    if (state.page === "proxy") renderPage();
  }
}

let proxyActionInProgress = false;

async function applySystemProxyMode(mode: SystemProxyMode) {
  if (state.busy || proxyActionInProgress) return;
  const previousMode = state.proxy.systemProxyMode;
  state.proxy.systemProxyMode = mode;
  if (mode === "pac") {
    state.proxy.systemProxyMode = previousMode;
    toast("PAC 模式当前尚未接入", true);
    return;
  }
  if (mode === "auto") {
    const applied = await toggleSystemProxy(true);
    if (!applied) state.proxy.systemProxyMode = previousMode;
  } else if (mode === "clear") {
    proxyActionInProgress = true;
    setProxyBusy(true);
    try {
      await invoke("clear_system_proxy");
      stopProxyBillingMonitor();
      window.clearInterval(state.nutProxyCheckTimer);
      state.nutProxyCheckTimer = 0;
      state.proxy.systemProxyEnabled = false;
      await loadProxyState();
      showProxyMessage("系统代理已清除，浏览器不再使用系统代理。");
    } catch (error) {
      state.proxy.systemProxyMode = previousMode;
      showProxyMessage(errorText(error, "清除系统代理失败"), true);
    } finally {
      proxyActionInProgress = false;
      setProxyBusy(false);
    }
  } else {
    showProxyMessage("已选择不改变系统代理，当前系统代理设置保持不变。");
  }
  localStorage.setItem("system-proxy-mode", state.proxy.systemProxyMode);
  if (state.page === "proxy") renderPage();
}

async function handleTrayProxyAction(enabled: boolean) {
  if (state.busy || proxyActionInProgress) { toast("当前任务进行中，请稍后重试", true); return; }
  try {
    if (!enabled && !state.account) {
      window.clearInterval(state.nutProxyCheckTimer);
      state.nutProxyCheckTimer = 0;
      stopProxyBillingMonitor();
      await invoke("set_system_proxy", { enabled: false, port: 1 });
      if (state.proxy.running) await proxyApi("POST", "/api/core/stop");
      state.proxy.systemProxyEnabled = false;
      state.proxy.running = false;
      toast("浏览器代理已关闭，原系统代理设置已恢复");
      return;
    }
    if (enabled && (!state.account || !state.proxyAgreementAcceptedThisRun)) {
      await getCurrentWindow().show();
      await getCurrentWindow().unminimize();
      await getCurrentWindow().setFocus();
      if (!state.account) { toast("请先登录后开启网络代理", true); return; }
      state.proxyStartAfterAgreement = true;
      openProxyAgreement();
      return;
    }
    if (enabled) {
      await loadProxyState();
      if (!state.proxy.ready) throw new Error(state.proxy.message || "代理服务尚未就绪");
      if (!state.proxy.profiles.some(profile => profile.isCurrent)) {
        await getCurrentWindow().show();
        await getCurrentWindow().unminimize();
        await getCurrentWindow().setFocus();
        state.page = "proxy";
        state.proxy.message = "请先在节点列表中选择一个节点，再开启网络代理。";
        renderDashboard();
        toast(state.proxy.message, true);
        return;
      }
      state.proxy.systemProxyMode = "auto";
      localStorage.setItem("system-proxy-mode", "auto");
    }
    await toggleSystemProxy(enabled);
  } catch (error) {
    toast(errorText(error, "托盘代理操作失败"), true);
  }
}

async function toggleSystemProxy(
  enabled = !state.proxy.systemProxyEnabled,
  options: { billNutUsage?: boolean; stopCore?: boolean } = {},
): Promise<boolean> {
  if ((state.busy && !state.installTaskActive) || proxyActionInProgress) return false;
  proxyActionInProgress = true;
  setProxyBusy(true);
  let coreWasRunning = state.proxy.running;
  try {
    if (!enabled) {
      window.clearInterval(state.nutProxyCheckTimer);
      state.nutProxyCheckTimer = 0;
      stopProxyBillingMonitor();
      await invoke("set_system_proxy", { enabled: false, port: 1 });
      state.proxy.systemProxyEnabled = false;
      if (state.proxy.running && options.stopCore !== false) await proxyApi("POST", "/api/core/stop");
      await delay(500);
      await loadProxyState();
      showProxyMessage(options.stopCore === false
        ? "浏览器代理已关闭，原系统代理设置已恢复。"
        : "浏览器代理已关闭，原系统代理设置已恢复，代理核心已停止。");
      return true;
    }
    if (!isNative()) throw new Error("请在绿色桌面版中开启浏览器代理");
    if (state.proxy.systemProxyMode === "pac") throw new Error("PAC 模式当前尚未接入");
    if (state.proxy.systemProxyMode === "unchanged") throw new Error("不改变系统代理模式不会自动配置浏览器；请使用自动配置模式");
    await prepareProxySubscriptionForStart();
    coreWasRunning = state.proxy.running;
    const nutSubscriptionId = await requireNutSubscriptionAccess();
    const shouldBillNutUsage = options.billNutUsage !== false && Boolean(nutSubscriptionId && state.nutSubscriptionKami);
    const configuredDailyPrice = Number(state.config.proxy_nut_daily_points);
    const dailyPrice = Math.max(0.01, Number.isFinite(configuredDailyPrice) && configuredDailyPrice > 0 ? configuredDailyPrice : 5);
    if (shouldBillNutUsage) {
      const balance = await invoke<number>("account_balance", { baseUrl: API_HOST, kami: state.nutSubscriptionKami });
      if (balance <= 0) throw new Error("账户无余额，无法代理。请先充值后再开启网络代理。");
      if (balance < dailyPrice) throw new Error(`账户余额不足，开启坚果订阅代理至少需要 ${dailyPrice} 点，请先充值。`);
    }
    if (!state.proxy.profiles.length) throw new Error("请先添加代理订阅并加载节点");
    if (!state.proxy.profiles.some(profile => profile.isCurrent)) throw new Error("请先在节点列表中选择一个节点，再开启网络代理");
    if (state.proxy.selectedRoute) {
      const selectedRoute = state.proxy.routingProfiles.find(route => route.id === state.proxy.selectedRoute);
      if (selectedRoute && !selectedRoute.isActive) {
        await proxyApi("POST", `/api/settings/routing-profiles/${encodeURIComponent(selectedRoute.id)}/activate`);
      }
    }
    await delay(600);
    await loadProxyState();
    if (!state.proxy.running) {
      if (!state.proxy.profiles.some(profile => profile.isCurrent)) throw new Error("没有可用节点，请先选择节点");
      await proxyApi("POST", "/api/core/start");
      await delay(800);
      await loadProxyState();
    }
    const port = Number(state.proxy.httpAddress.split(":").pop());
    if (!Number.isInteger(port) || port <= 0 || state.proxy.httpAddress === "未监听") {
      throw new Error("代理核心已启动，但没有可用的 HTTP 监听端口");
    }
    await invoke("set_system_proxy", { enabled: true, port });
    state.proxy.systemProxyMode = "auto";
    localStorage.setItem("system-proxy-mode", "auto");
    state.proxy.systemProxyEnabled = true;
    await loadProxyState();
    if (shouldBillNutUsage && nutSubscriptionId) {
      const localBaselineBytes = await readNutSubscriptionTraffic(nutSubscriptionId);
      const billing = await invoke<ProxyBillingResult>("installer_proxy_billing", {
        baseUrl: API_HOST,
        kami: state.nutSubscriptionKami,
        action: "start",
        cumulativeBytes: 0,
      });
      if (!billing.allowed) throw new Error(billing.message || "账户余额不足，无法开启坚果订阅代理。");
      startProxyBillingMonitor(nutSubscriptionId, localBaselineBytes, billing.bytesUsed, billing.usageDate);
    }
    if (state.nutSubscriptionKami) {
      window.clearInterval(state.nutProxyCheckTimer);
      state.nutProxyCheckTimer = window.setInterval(() => {
        void recheckNutSubscriptionAccess();
      }, 30_000);
    }
    showProxyMessage(`浏览器代理已开启，当前节点：${state.proxy.profiles.find(profile => profile.isCurrent)?.remarks || "已选择节点"}`);
    return true;
  } catch (error) {
    stopProxyBillingMonitor();
    try { await invoke("set_system_proxy", { enabled: false, port: 1 }); } catch { /* Keep the original failure visible. */ }
    if (state.proxy.running && !coreWasRunning) try { await proxyApi("POST", "/api/core/stop"); } catch { /* Keep the original failure visible. */ }
    state.proxy.systemProxyEnabled = false;
    showProxyMessage(errorText(error, "系统代理设置失败"), true);
    return false;
  } finally {
    proxyActionInProgress = false;
    setProxyBusy(false);
  }
}

async function requireNutSubscriptionAccess(): Promise<string | null> {
  if (!state.nutSubscriptionKami) return null;
  const configuredUrl = state.config.v2rayn_subscription_url || "";
  const nutSubscription = state.proxy.subscriptions.find(subscription =>
    subscription.remarks === "坚果订阅" || (configuredUrl && subscription.url === configuredUrl)
  );
  const usingNutSubscription = Boolean(nutSubscription && (
    state.proxy.selectedSubscription === nutSubscription.id
    || state.proxy.profiles.some(profile => profile.isCurrent && profile.subscriptionId === nutSubscription.id)
  ));
  if (!usingNutSubscription || !nutSubscription) return null;
  const allowed = await invoke<boolean>("check_nut_subscription_access", {
    baseUrl: API_HOST,
    kami: state.nutSubscriptionKami,
  });
  if (!allowed) {
    await revokeLocalNutSubscription();
    throw new Error("该卡密的坚果订阅使用权限已关闭，无法开启浏览器代理");
  }
  return nutSubscription.id;
}

function stopProxyBillingMonitor() {
  if (state.proxyBilling.timer) window.clearInterval(state.proxyBilling.timer);
  state.proxyBilling.timer = 0;
  state.proxyBilling.active = false;
  state.proxyBilling.subscriptionId = "";
  state.proxyBilling.localBaselineBytes = 0;
  state.proxyBilling.serverBaselineBytes = 0;
  state.proxyBilling.usageDate = "";
}

function startProxyBillingMonitor(subscriptionId: string, localBaselineBytes: number, serverBaselineBytes: number, usageDate: string) {
  stopProxyBillingMonitor();
  state.proxyBilling.active = true;
  state.proxyBilling.subscriptionId = subscriptionId;
  state.proxyBilling.localBaselineBytes = Math.max(0, localBaselineBytes);
  state.proxyBilling.serverBaselineBytes = Math.max(0, serverBaselineBytes);
  state.proxyBilling.usageDate = usageDate;
  state.proxyBilling.timer = window.setInterval(() => { void reportNutProxyTraffic(); }, 5_000);
}

async function readNutSubscriptionTraffic(subscriptionId: string): Promise<number> {
  const query = `?subscriptionId=${encodeURIComponent(subscriptionId)}`;
  const profiles = proxyData<Array<{ todayUp?: number; todayDown?: number }>>(
    await proxyApi("GET", `/api/profiles${query}`),
  ) || [];
  return profiles.reduce((total, profile) =>
    total + Math.max(0, Number(profile.todayUp) || 0) + Math.max(0, Number(profile.todayDown) || 0), 0);
}

async function reportNutProxyTraffic() {
  if (!state.proxyBilling.active || !state.proxy.systemProxyEnabled || state.proxyOperationBusy) return;
  try {
    const cumulativeBytes = await readNutSubscriptionTraffic(state.proxyBilling.subscriptionId);
    const usageDate = new Intl.DateTimeFormat("en-CA", { timeZone: "Asia/Shanghai" }).format(new Date());
    const newDay = usageDate !== state.proxyBilling.usageDate;
    if (newDay) {
      state.proxyBilling.usageDate = usageDate;
      state.proxyBilling.localBaselineBytes = cumulativeBytes;
      state.proxyBilling.serverBaselineBytes = 0;
    } else if (cumulativeBytes < state.proxyBilling.localBaselineBytes) {
      state.proxyBilling.localBaselineBytes = cumulativeBytes;
    }
    const sessionBytes = Math.max(0, cumulativeBytes - state.proxyBilling.localBaselineBytes);
    const billableBytes = state.proxyBilling.serverBaselineBytes + sessionBytes;
    if (!newDay && sessionBytes === 0) return;
    const billing = await invoke<ProxyBillingResult>("installer_proxy_billing", {
      baseUrl: API_HOST,
      kami: state.nutSubscriptionKami,
      action: "report",
      cumulativeBytes: billableBytes,
    });
    if (!billing.allowed) {
      stopProxyBillingMonitor();
      await toggleSystemProxy(false);
      showProxyMessage(billing.message || "坚果订阅代理余额不足，已自动关闭代理。", true);
    } else {
      state.proxyBilling.localBaselineBytes = cumulativeBytes;
      state.proxyBilling.serverBaselineBytes = Math.max(0, billing.bytesUsed);
      state.proxyBilling.usageDate = billing.usageDate || usageDate;
    }
  } catch (error) {
    stopProxyBillingMonitor();
    await toggleSystemProxy(false);
    showProxyMessage(`无法确认坚果订阅流量扣费状态，已自动关闭代理：${errorText(error, "请检查网络后重试")}`, true);
  }
}

async function recheckNutSubscriptionAccess() {
  if (!state.proxy.systemProxyEnabled || !state.nutSubscriptionKami) return;
  try {
    await requireNutSubscriptionAccess();
  } catch (error) {
    window.clearInterval(state.nutProxyCheckTimer);
    state.nutProxyCheckTimer = 0;
    if (state.proxy.systemProxyEnabled) {
      try { await invoke("set_system_proxy", { enabled: false, port: 1 }); } catch { /* Keep revocation handling moving. */ }
      state.proxy.systemProxyEnabled = false;
    }
    if (state.proxy.running) {
      try { await proxyApi("POST", "/api/core/stop"); } catch { /* The system proxy is already disabled. */ }
      state.proxy.running = false;
    }
    await loadProxyState();
    showProxyMessage(errorText(error, "坚果订阅使用权限已关闭"), true);
    renderPage();
  }
}

async function revokeLocalNutSubscription() {
  const configuredUrl = state.config.v2rayn_subscription_url || "";
  state.config.v2rayn_subscription_url = "";
  const nutSubscriptions = state.proxy.subscriptions.filter(subscription =>
    subscription.remarks === "坚果订阅" || (configuredUrl && subscription.url === configuredUrl)
  );
  for (const subscription of nutSubscriptions) {
    try { await proxyApi("DELETE", `/api/subscriptions/${encodeURIComponent(subscription.id)}`); }
    catch { /* Remove the local reference even if v2rayN is temporarily unavailable. */ }
  }
  state.proxy.subscriptions = state.proxy.subscriptions.filter(subscription => !nutSubscriptions.includes(subscription));
  state.proxy.profiles = state.proxy.profiles.filter(profile =>
    !nutSubscriptions.some(subscription => profile.subscriptionId === subscription.id)
  );
}

async function logoutAssistant() {
  accountSessionGeneration++;
  pendingAccountKey = null;
  stopWeChatView();
  if (isNative()) { try { await invoke("wechat_action", { action: "stop", payload: {} }); } catch { /* Continue logging out when the bridge is already stopped. */ } }
  resetWallet();
  resetPluginMarket(); resetPublicPluginMarket();
  stopProxyBillingMonitor();
  window.clearInterval(state.nutProxyCheckTimer);
  state.nutProxyCheckTimer = 0;
  if (state.proxy.systemProxyEnabled) {
    try { await invoke("set_system_proxy", { enabled: false, port: 1 }); } catch { /* Continue clearing the session. */ }
    if (state.proxy.running) try { await proxyApi("POST", "/api/core/stop"); } catch { /* Continue clearing the session. */ }
  }
  state.proxy.systemProxyEnabled = false;
  state.proxy.running = false;
  if (isNative()) { try { await accountRequest("logout"); } catch { /* Clear local credentials even when offline. */ } }
  state.account = null;
  state.loginMode = "kami";
  state.nutSubscriptionKami = "";
  state.config = {};
  state.providerApiKey = "";
  state.providers = []; state.providersLoaded = false; state.personal.revealLoginKami = false;
  state.personal.balance = null;
  state.personal.loaded = false;
  state.personal.section = "info";
  if (isNative()) {
    try { await invoke("clear_auto_login_kami"); } catch (error) { toast(errorText(error, "清除自动登录凭据失败"), true); }
  }
  renderLogin();
}

function setProxyBusy(busy: boolean) {
  state.proxyOperationBusy = busy;
  if (!state.installTaskActive) state.busy = busy;
  document.querySelectorAll<HTMLButtonElement>(".proxy-manager-layout button, .proxy-advanced button, #proxy-refresh, #proxy-service-start, #subscription-add, #subscription-update, #system-proxy-toggle, #clear-system-proxy").forEach(button => { button.disabled = busy; });
}

function showProxyMessage(message: string, isError = false) {
  state.proxy.message = message;
  renderPage();
  if (isError) toast(message, true);
}

function errorText(error: unknown, fallback: string) {
  if (typeof error === "string" && error.trim()) return error;
  if (error instanceof Error && error.message) return error.message;
  if (error && typeof error === "object" && "message" in error && typeof error.message === "string") return error.message;
  return fallback;
}

const delay = (milliseconds: number) => new Promise(resolve => window.setTimeout(resolve, milliseconds));

function renderTaskProgress() {
  const panel = document.querySelector<HTMLElement>("#progress-panel");
  if (!panel) return;
  panel.hidden = !state.taskProgress;
  panel.classList.toggle("progress-inline", state.taskProgressDocked);
  const progress = state.taskProgress;
  if (!progress) return;
  const close = document.querySelector<HTMLButtonElement>("#progress-close")!;
  close.hidden = false;
  close.textContent = state.taskProgressDocked ? "⤢" : "×";
  close.title = state.taskProgressDocked ? "悬浮显示进度" : "收起悬浮窗，在页面下方显示进度";
  close.setAttribute("aria-label", close.title);
  document.querySelector<HTMLElement>("#progress-title")!.textContent = progress.title;
  document.querySelector<HTMLElement>("#progress-percent")!.textContent = progress.percent;
  const longError = progress.failed && (progress.detail.length > 320 || progress.detail.includes("\n"));
  document.querySelector<HTMLElement>("#progress-detail")!.textContent = longError
    ? progress.detail.split(/\r?\n/)[0].slice(0, 320) : progress.detail;
  const details = document.querySelector<HTMLDetailsElement>("#progress-error-details")!;
  details.hidden = !longError;
  details.querySelector("pre")!.textContent = longError ? progress.detail : "";
  if (!longError) details.open = false;
  const bar = document.querySelector<HTMLElement>("#progress-bar")!;
  bar.style.width = progress.width;
  bar.classList.toggle("failed", progress.failed);
}

function rememberTaskProgress() {
  if (!state.taskProgress) return;
  const title = document.querySelector<HTMLElement>("#progress-title");
  const percent = document.querySelector<HTMLElement>("#progress-percent");
  const detail = document.querySelector<HTMLElement>("#progress-detail");
  const bar = document.querySelector<HTMLElement>("#progress-bar");
  if (title && percent && detail && bar) {
    state.taskProgress = { title: title.textContent || "", percent: percent.textContent || "", detail: detail.textContent || "", width: bar.style.width, failed: bar.classList.contains("failed") };
  }
}

async function runTask(title: string, action: () => Promise<void>): Promise<boolean> {
  if (state.busy) return false;
  state.busy = true;
  state.installTaskActive = title.includes("安装");
  if (state.installTaskActive) {
    state.setupFailed = false;
    state.setupMessage = "";
    const previousResult = document.querySelector<HTMLElement>(".setup-result");
    if (previousResult) {
      previousResult.hidden = true;
      previousResult.classList.remove("error");
      previousResult.textContent = "";
    }
  }
  state.downloadSlowSince = 0;
  state.downloadSuggestionVisible = false;
  state.downloadActive = false;
  state.downloadPaused = false;
  state.downloadCancelled = false;
  state.lastDownloadProgress = null;
  state.taskWasCancelled = false;
  const controls = [...document.querySelectorAll<HTMLButtonElement | HTMLInputElement | HTMLSelectElement>("#page-content button, #page-content input, #page-content select")];
  const disabledControls = controls.map(control => control.disabled);
  controls.forEach(control => { control.disabled = true; });
  state.taskProgress = { title, percent: "进行中", width: "28%", failed: false, detail: "正在准备任务…" };
  renderTaskProgress();
  let succeeded = false;
  let cancelled = false;
  try {
    await action();
    if (state.installTaskActive) {
      state.setupFailed = false;
      state.setupMessage ||= state.products.every(product => product.ready)
        ? "Codex 和 Codex++ 已安装。可配置供应商或启动 Codex。"
        : `${title} 已完成。`;
    }
    succeeded = true;
    return true;
  }
  catch (error) {
    cancelled = errorText(error, "").includes("DOWNLOAD_CANCELLED");
    state.taskWasCancelled = cancelled;
    if (cancelled) {
      state.setupMessage = "";
      state.setupFailed = false;
      toast("下载已停止，可以重新安装");
    } else {
      state.taskProgress = { title, percent: "失败", width: "100%", failed: true, detail: errorText(error, "任务失败") };
      renderTaskProgress();
    }
    return false;
  }
  finally {
    controls.forEach((control, index) => { if (control.isConnected) control.disabled = disabledControls[index]; });
    if (isNative()) void invoke("set_download_paused", { paused: false }).catch(() => {});
    state.busy = false;
    state.installTaskActive = false;
    state.downloadActive = false;
    state.downloadPaused = false;
    state.downloadCancelled = false;
    state.lastDownloadProgress = null;
    state.downloadSuggestionVisible = false;
    const pauseButton = document.querySelector<HTMLButtonElement>("#download-pause-toggle");
    if (pauseButton) {
      pauseButton.hidden = true;
      pauseButton.disabled = false;
    }
    const stopButton = document.querySelector<HTMLButtonElement>("#download-stop");
    if (stopButton) {
      stopButton.hidden = true;
      stopButton.disabled = false;
      stopButton.textContent = "停止下载";
    }
    const proxySuggestion = document.querySelector<HTMLButtonElement>("#download-proxy-suggestion");
    if (proxySuggestion) proxySuggestion.hidden = true;
    if (succeeded || cancelled) {
      state.taskProgress = null;
    }
    renderTaskProgress();
    if (state.page === "install") renderPage();
  }
}

function formatBytes(bytes: number) {
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${Math.max(0, Math.round(bytes / 1024))} KB`;
}

function formatDuration(seconds: number) {
  const value = Math.max(0, Math.ceil(seconds));
  if (value < 60) return `${value} 秒`;
  const minutes = Math.floor(value / 60);
  const rest = value % 60;
  return rest ? `${minutes} 分 ${rest} 秒` : `${minutes} 分`;
}

function updateDownloadProgress(progress: DownloadProgress) {
  if (!state.busy || !state.installTaskActive) return;
  if (state.downloadCancelled) return;
  state.lastDownloadProgress = progress;
  state.downloadActive = true;
  const stop = document.querySelector<HTMLButtonElement>("#download-stop");
  if (stop) { stop.hidden = false; stop.title = ""; }
  const pauseButton = document.querySelector<HTMLButtonElement>("#download-pause-toggle");
  if (pauseButton) {
    pauseButton.hidden = !progress.resumable;
    pauseButton.textContent = state.downloadPaused ? "继续下载" : "暂停下载";
  }
  const percent = document.querySelector<HTMLElement>("#progress-percent");
  const bar = document.querySelector<HTMLElement>("#progress-bar");
  const detail = document.querySelector<HTMLElement>("#progress-detail");
  const suggestion = document.querySelector<HTMLButtonElement>("#download-proxy-suggestion");
  if (!percent || !bar || !detail) return;
  if (progress.phase === "install" || progress.phase === "install-wait" || progress.phase === "cached") {
    state.downloadActive = false;
    state.downloadSlowSince = 0;
    state.downloadSuggestionVisible = false;
    if (stop) {
      stop.hidden = progress.phase !== "install-wait";
      stop.textContent = "取消等待";
      stop.title = "取消助手等待，Windows 安装任务可能继续；安装包会保留。";
    }
    if (pauseButton) pauseButton.hidden = true;
    if (suggestion) suggestion.hidden = true;
    percent.textContent = progress.phase !== "cached" ? "安装中" : "已下载";
    bar.style.width = "100%";
    detail.textContent = progress.status || "正在使用已下载的安装包";
    const title = document.querySelector<HTMLElement>("#progress-title");
    if (title) title.textContent = progress.phase !== "cached" ? `${progress.product} 正在安装` : `${progress.product} 使用已下载的安装包`;
    rememberTaskProgress();
    return;
  }
  const speed = progress.bytes_per_second;
  const speedText = speed >= 1024 * 1024
    ? `${(speed / 1024 / 1024).toFixed(1)} MB/s`
    : `${Math.round(speed / 1024)} KB/s`;
  let suffix = "";
  if (progress.total && progress.total > 0) {
    const ratio = Math.min(1, progress.downloaded / progress.total);
    percent.textContent = `${Math.floor(ratio * 100)}%`;
    bar.style.width = `${Math.max(2, ratio * 100)}%`;
    const remaining = speed > 0 ? `，预计剩余 ${formatDuration((progress.total - progress.downloaded) / speed)}` : "，正在计算剩余时间";
    suffix = ` / ${formatBytes(progress.total)}${remaining}`;
  } else {
    percent.textContent = "下载中";
    bar.style.width = "28%";
  }
  detail.textContent = `${progress.product}：已下载 ${formatBytes(progress.downloaded)}，速度 ${speedText}${suffix}${state.downloadPaused ? " · 已暂停，已下载内容会保留" : progress.status ? ` · ${progress.status}` : ""}`;
  const title = document.querySelector<HTMLElement>("#progress-title");
  if (title) title.textContent = state.downloadPaused ? `${progress.product} 下载已暂停` : progress.status ? `${progress.product} 正在重新连接` : `${progress.product} 下载中`;
  rememberTaskProgress();

  if (!state.proxy.systemProxyEnabled && speed < 256 * 1024) {
    if (!state.downloadSlowSince) state.downloadSlowSince = Date.now();
    if (Date.now() - state.downloadSlowSince >= 8_000) {
      state.downloadSuggestionVisible = true;
      if (suggestion) suggestion.hidden = false;
      document.querySelectorAll<HTMLButtonElement>(".proxy-manager-layout button, .proxy-advanced button, #proxy-refresh, #proxy-service-start, #subscription-add, #subscription-update, #system-proxy-toggle, #clear-system-proxy")
        .forEach(button => { button.disabled = state.proxyOperationBusy; });
    }
  } else {
    state.downloadSlowSince = 0;
    state.downloadSuggestionVisible = false;
    if (suggestion) suggestion.hidden = true;
    if (state.page === "proxy") {
      document.querySelectorAll<HTMLButtonElement>(".proxy-manager-layout button, .proxy-advanced button, #proxy-refresh, #proxy-service-start, #subscription-add, #subscription-update, #system-proxy-toggle, #clear-system-proxy")
        .forEach(button => { button.disabled = state.proxyOperationBusy || (state.busy && !state.installTaskActive); });
    }
  }
}

async function stopDownload() {
  if (!state.busy || !state.installTaskActive || state.downloadCancelled || !isNative()) return;
  state.downloadCancelled = true;
  const button = document.querySelector<HTMLButtonElement>("#download-stop");
  if (button) {
    button.disabled = true;
    button.textContent = "正在停止…";
  }
  try {
    await invoke("set_download_cancelled", { cancelled: true });
  } catch (error) {
    state.downloadCancelled = false;
    if (button?.isConnected) {
      button.disabled = false;
      button.textContent = "停止下载";
    }
    toast(errorText(error, "停止下载失败"), true);
  }
}

async function toggleDownloadPause() {
  if (!state.downloadActive || !isNative()) return;
  state.downloadPaused = !state.downloadPaused;
  const button = document.querySelector<HTMLButtonElement>("#download-pause-toggle");
  if (button) {
    button.disabled = true;
    button.textContent = state.downloadPaused ? "正在暂停…" : "正在继续…";
  }
  try {
    await invoke("set_download_paused", { paused: state.downloadPaused });
    if (button?.isConnected) {
      button.disabled = false;
      button.textContent = state.downloadPaused ? "继续下载" : "暂停下载";
    }
    const detail = document.querySelector<HTMLElement>("#progress-detail");
    if (detail && state.downloadPaused) detail.textContent = `${detail.textContent} · 已暂停，已下载内容会保留`;
  } catch (error) {
    state.downloadPaused = !state.downloadPaused;
    if (button?.isConnected) {
      button.disabled = false;
      button.textContent = state.downloadPaused ? "继续下载" : "暂停下载";
    }
    toast(errorText(error, "无法更改下载状态"), true);
  }
}

function toast(message: string, isError = false) {
  let element = document.querySelector<HTMLElement>("#toast");
  if (!element) { element = document.createElement("div"); element.id = "toast"; element.className = "toast"; document.body.append(element); }
  element.textContent = message; element.classList.toggle("error", isError); element.classList.add("visible");
  window.setTimeout(() => element?.classList.remove("visible"), 3200);
}

renderLogin();
if (isNative()) {
  void restoreAutoLogin();
  void listen<DownloadProgress>("installer-download-progress", event => updateDownloadProgress(event.payload))
    .catch(error => toast(errorText(error, "下载进度监听初始化失败"), true));
  void listen<boolean>("tray-proxy-action", event => {
    void handleTrayProxyAction(event.payload);
  }).catch(error => toast(errorText(error, "托盘代理操作初始化失败"), true));
}

async function restoreAutoLogin() {
  if (state.autoLoginAttempted) return;
  state.autoLoginAttempted = true;
  const submit = document.querySelector<HTMLButtonElement>("#login-form button[type=submit]");
  if (submit) submit.disabled = true;
  try {
    const kami = await invoke<string | null>("load_auto_login_kami");
    if (!kami?.trim() || !document.querySelector("#login-form")) {
      if (submit?.isConnected) submit.disabled = false;
      return;
    }
    state.loginMode = "kami";
    state.rememberLogin = true;
    const input = document.querySelector<HTMLInputElement>("#login-kami");
    if (input) input.value = kami;
    document.querySelector<HTMLInputElement>("#remember-login")!.checked = true;
    if (submit?.isConnected) submit.disabled = false;
    document.querySelector<HTMLFormElement>("#login-form")!.requestSubmit();
  } catch (error) {
    const loginError = document.querySelector<HTMLElement>("#login-error");
    if (loginError) loginError.textContent = `自动登录凭据无法读取：${errorText(error, "请手动输入卡密登录")}`;
    if (submit?.isConnected) submit.disabled = false;
  }
}

