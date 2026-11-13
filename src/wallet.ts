import { invoke } from "@tauri-apps/api/core";

type Package = { points: number; amount: number; label?: string; price?: string };
type Order = { orderNo: string; amount: number; points: number; status: string; paymentInfo: string; paymentType: string; balance?: number | null };
type Credentials = { accountId: number; kami: string };
const wallet = {
  credentials: null as Credentials | null,
  packages: [] as Package[],
  loaded: false,
  loading: false,
  ready: false,
  selected: 1,
  custom: "",
  creating: false,
  checking: false,
  order: null as Order | null,
  message: "",
  error: false,
  epoch: 0,
  timer: 0,
  deadline: 0,
};
let host: HTMLElement | null = null;
let onBalance: (balance: number | null) => void = () => {};
const escape = (text: unknown) => String(text ?? "").replace(/[&<>"']/g, char => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[char]!);
const number = (value: number) => value.toLocaleString("zh-CN", { maximumFractionDigits: 2 });
const pending = () => Boolean(wallet.order && !["paid", "closed", "cancelled", "failed", "expired"].includes(wallet.order.status.toLowerCase()));

export function resetWallet() {
  window.clearTimeout(wallet.timer);
  wallet.epoch++;
  wallet.credentials = null;
  wallet.order = null;
  wallet.custom = "";
  wallet.creating = false;
  wallet.checking = false;
  wallet.message = "";
  wallet.error = false;
  wallet.loaded = false;
  wallet.loading = false;
  wallet.packages = [];
  wallet.ready = false;
  wallet.selected = 1;
  host = null;
}

export function renderWallet(element: HTMLElement, credentials: Credentials | null, balanceChanged: typeof onBalance) {
  if (wallet.credentials?.accountId !== credentials?.accountId || wallet.credentials?.kami !== credentials?.kami) resetWallet();
  wallet.credentials = credentials;
  host = element;
  onBalance = balanceChanged;
  paint();
  if (credentials && !wallet.loaded && !wallet.loading) void loadCatalog();
}

function hint() {
  if (wallet.custom.trim()) {
    const amount = Number(wallet.custom);
    return validAmount() ? `自定义充值 ¥${number(amount)}，到账点数以服务器订单为准。` : "自定义金额无效，请输入不低于 0.01 元、最多两位小数的金额。";
  }
  if (wallet.selected >= 0 && wallet.packages[wallet.selected]) {
    const item = wallet.packages[wallet.selected];
    return `已选择 ¥${number(item.amount)}，到账 ${number(item.points)} 点。`;
  }
  return "请选择套餐或输入有效的充值金额。";
}

function validAmount() {
  return /^\d+(?:\.\d{1,2})?$/.test(wallet.custom) && Number(wallet.custom) >= 0.01 && Number.isFinite(Number(wallet.custom));
}

function trustedPaymentUrl(value: string) {
  try {
    const url = new URL(value);
    return url.protocol === "https:" && !url.username && !url.password && (!url.port || url.port === "443")
      && (url.hostname === "alipay.com" || url.hostname.endsWith(".alipay.com") || url.hostname === "api.jianguo.ltd");
  } catch { return false; }
}

function paymentSurface() {
  const order = wallet.order;
  if (!order) return `<div class="wallet-placeholder"><span>付款二维码</span></div>`;
  if (order.status.toLowerCase() === "paid") return `<div class="wallet-paid"><b>支付成功</b><span>${escape(number(order.points))} 点已到账</span></div>`;
  if (!pending()) return `<div class="wallet-placeholder"><span>订单已结束，请重新选择金额</span></div>`;
  if (!trustedPaymentUrl(order.paymentInfo)) return `<div class="wallet-placeholder"><span>付款地址无效，已阻止加载</span></div>`;
  if ((order.paymentType || "").toLowerCase() === "cashier") {
    return `<iframe src="${escape(order.paymentInfo)}" title="支付宝官方付款码" referrerpolicy="no-referrer" sandbox="allow-scripts allow-forms allow-same-origin" scrolling="no"></iframe>`;
  }
  return `<img src="https://api.jianguo.ltd/api/v1/wallet/qr?data=${escape(encodeURIComponent(order.paymentInfo))}" alt="支付宝付款二维码" referrerpolicy="no-referrer">`;
}

function paint() {
  if (!host?.isConnected) return;
  const locked = wallet.creating || pending();
  const disabled = !wallet.credentials || locked;
  host.innerHTML = `<div class="wallet-layout"><section class="wallet-amounts" aria-labelledby="wallet-amount-title">
    <div class="panel-title"><div><h3 id="wallet-amount-title">选择充值金额</h3><p>选择套餐或输入自定义金额，支付成功后点数自动到账。</p></div></div>
    <p class="wallet-rate"><b>1 元 = 10 点</b><span>充值金额按此比例换算为可用点数。</span></p>
    <fieldset class="wallet-packages" ${disabled ? "disabled" : ""}><legend class="sr-only">充值套餐</legend>${wallet.packages.map((item, index) =>
      `<label class="wallet-package"><input type="radio" name="wallet-package" value="${index}" ${wallet.selected === index ? "checked" : ""}><span><b>¥${escape(number(item.amount))}</b><small>${escape(number(item.points))} 点</small></span></label>`).join("")}</fieldset>
    ${wallet.loading ? `<p class="account-status" role="status">正在读取充值金额…</p>` : ""}
    <label class="wallet-custom-label" for="wallet-custom">自定义金额（元）</label><input id="wallet-custom" type="text" inputmode="decimal" autocomplete="off" placeholder="最低 0.01 元" value="${escape(wallet.custom)}" ${disabled ? "disabled" : ""}>
    <button id="wallet-pay" class="button primary" type="button" ${!wallet.credentials || !wallet.ready || locked ? "disabled" : ""}>${wallet.creating ? "正在创建订单…" : pending() ? "等待付款" : wallet.order?.status === "paid" ? "继续充值" : "支付宝扫码支付"}</button>
    <p id="wallet-catalog-status" class="account-status">${!wallet.credentials ? "请使用卡密登录后充值。" : wallet.loaded ? wallet.ready ? "支付宝支付已就绪，付款后自动检测到账。" : "支付宝服务暂不可用，请稍后重试或使用卡密充值。" : "正在连接充值服务…"}</p>
    <button id="wallet-catalog-retry" type="button" class="button quiet" ${wallet.loading || locked ? "disabled" : ""}>重新读取充值配置</button>
    </section><section class="wallet-payment" aria-labelledby="wallet-payment-title"><h3 id="wallet-payment-title">支付宝付款</h3>
    <p id="wallet-order-summary" class="account-status">${wallet.order ? `订单 ${escape(wallet.order.orderNo)}<br>¥${escape(number(wallet.order.amount))} · ${escape(number(wallet.order.points))} 点` : "创建订单后显示付款二维码"}</p>
    <div class="wallet-qr">${paymentSurface()}</div><p id="wallet-payment-status" class="account-status ${wallet.error ? "account-error" : ""}" role="status" aria-live="polite">${escape(wallet.message || hint())}</p>
    <div class="wallet-order-actions">${pending() ? `<button id="wallet-check" class="button quiet" type="button" ${wallet.checking ? "disabled" : ""}>检查到账</button><button id="wallet-reset" class="button quiet" type="button">重新选择金额</button>` : ""}</div>
    </section></div><div class="wallet-notice"><b>在线充值说明</b><p>请使用手机支付宝扫描官方付款码。支付成功后，点数自动进入当前登录账户；不要重复付款。若付款码未显示，可检查网络连接后重试。充值金额与到账点数以实际订单为准。</p></div>`;
  host.querySelectorAll<HTMLInputElement>('input[name="wallet-package"]').forEach(input => input.addEventListener("change", () => {
    wallet.selected = Number(input.value);
    wallet.custom = "";
    wallet.message = "";
    paint();
  }));
  host.querySelector<HTMLInputElement>("#wallet-custom")!.addEventListener("input", event => {
    wallet.custom = (event.currentTarget as HTMLInputElement).value.trim();
    wallet.selected = -1;
    wallet.message = "";
    host?.querySelectorAll<HTMLInputElement>('input[name="wallet-package"]').forEach(input => { input.checked = false; });
    status(hint(), false);
  });
  host.querySelector("#wallet-pay")!.addEventListener("click", () => void createOrder());
  host.querySelector("#wallet-catalog-retry")!.addEventListener("click", () => void loadCatalog());
  host.querySelector("#wallet-check")?.addEventListener("click", () => void checkOrder());
  host.querySelector(".wallet-qr img")?.addEventListener("error", () => status("付款码加载失败，请检查网络连接；已付款请点击“检查到账”，不要重复支付。", true));
  host.querySelector("#wallet-reset")?.addEventListener("click", () => {
    if (!window.confirm("当前订单尚未确认付款。重新选择金额不会撤销原订单；如果已经付款，请先检查到账，避免重复支付。确定继续吗？")) return;
    window.clearTimeout(wallet.timer);
    wallet.epoch++;
    wallet.checking = false;
    wallet.order = null;
    wallet.message = "";
    wallet.error = false;
    paint();
  });
}

function status(message: string, error: boolean) {
  wallet.message = message;
  wallet.error = error;
  const element = host?.isConnected ? host.querySelector<HTMLElement>("#wallet-payment-status") : null;
  if (element) {
    element.textContent = message;
    element.classList.toggle("account-error", error);
  }
}

async function loadCatalog() {
  if (wallet.loading || !wallet.credentials) return;
  const epoch = wallet.epoch;
  wallet.loading = true;
  paint();
  try {
    const result = await invoke<{ packages: Package[]; paymentReady: boolean }>("wallet_catalog");
    if (epoch !== wallet.epoch) return;
    wallet.packages = (result.packages || []).filter(item => Number.isFinite(item.points) && item.points > 0 && Number.isFinite(item.amount) && item.amount > 0);
    wallet.ready = result.paymentReady === true;
    wallet.loaded = true;
    const hundredYuanPackage = wallet.packages.findIndex(item => Math.abs(item.amount - 100) < 0.001);
    wallet.selected = hundredYuanPackage >= 0 ? hundredYuanPackage : wallet.packages.length > 1 ? 1 : wallet.packages.length ? 0 : -1;
    wallet.message = "";
    wallet.error = false;
  } catch (error) {
    if (epoch !== wallet.epoch) return;
    wallet.ready = false;
    wallet.loaded = true;
    status(`读取充值配置失败：${String(error instanceof Error ? error.message : error)}`, true);
  } finally {
    if (epoch === wallet.epoch) { wallet.loading = false; paint(); }
  }
}

async function createOrder() {
  if (!wallet.credentials || wallet.creating || pending() || !wallet.ready) return;
  if (!Number.isSafeInteger(wallet.credentials.accountId) || wallet.credentials.accountId < 0 || !wallet.credentials.kami.trim()) {
    status("当前登录信息不完整，请重新登录后再进行充值。", true);
    return;
  }
  const inputAmount = host?.querySelector<HTMLInputElement>("#wallet-custom")?.value.trim() ?? wallet.custom.trim();
  if (inputAmount) wallet.custom = inputAmount;
  const useCustomAmount = Boolean(inputAmount);
  if (useCustomAmount && !validAmount()) { status("自定义金额无效，请输入不低于 0.01 元、最多两位小数的金额。", true); return; }
  const selected = useCustomAmount ? undefined : wallet.packages[wallet.selected];
  if (!selected && !useCustomAmount) { status("请选择套餐或输入有效的充值金额。", true); return; }
  const epoch = wallet.epoch;
  const credentials = { ...wallet.credentials };
  wallet.creating = true;
  wallet.message = "正在连接支付宝支付服务，请勿重复操作。";
  wallet.error = false;
  paint();
  try {
    const order = await invoke<Order>("wallet_create_order", { ...credentials, value: useCustomAmount ? Number(inputAmount) : selected!.points, custom: useCustomAmount });
    if (epoch !== wallet.epoch) return;
    if (!order?.orderNo || typeof order.status !== "string" || !Number.isFinite(order.amount) || order.amount <= 0 || !Number.isFinite(order.points) || order.points <= 0 || !trustedPaymentUrl(order.paymentInfo)) throw new Error("服务器未返回有效的付款信息，请稍后重试。");
    wallet.order = order;
    wallet.deadline = Date.now() + 10 * 60 * 1000;
    wallet.message = order.status === "paid" ? "支付成功，点数已到账。" : "请使用支付宝扫码付款，正在自动检测到账。";
    if (order.status === "paid") onBalance(order.balance ?? null);
    else schedule();
  } catch (error) {
    if (epoch !== wallet.epoch) return;
    status(`创建订单失败：${String(error instanceof Error ? error.message : error)}。若已付款，请先刷新余额，勿重复支付。`, true);
  } finally {
    if (epoch === wallet.epoch) { wallet.creating = false; paint(); }
  }
}

function schedule() {
  window.clearTimeout(wallet.timer);
  if (!pending()) return;
  if (Date.now() >= wallet.deadline) { status("自动检测已暂停。已付款可点击“检查到账”，请勿重复支付。", false); return; }
  wallet.timer = window.setTimeout(() => void checkOrder(), 3000);
}

async function checkOrder() {
  if (!wallet.credentials || !wallet.order || wallet.checking || !pending()) return;
  if (!Number.isSafeInteger(wallet.credentials.accountId) || wallet.credentials.accountId < 0 || !wallet.credentials.kami.trim()) {
    status("当前登录信息不完整，无法查询订单。请重新登录后重试。", true);
    return;
  }
  window.clearTimeout(wallet.timer);
  const epoch = wallet.epoch;
  const orderNo = wallet.order.orderNo;
  wallet.checking = true;
  const button = host?.querySelector<HTMLButtonElement>("#wallet-check");
  if (button) button.disabled = true;
  try {
    const result = await invoke<Order>("wallet_order_status", { ...wallet.credentials, orderNo });
    if (epoch !== wallet.epoch || wallet.order?.orderNo !== orderNo) return;
    if (result.orderNo !== orderNo || !result.status) throw new Error("订单状态返回异常");
    if (result.status.toLowerCase() === "paid") {
      wallet.order = { ...wallet.order, ...result };
      status(`支付成功，${number(result.points)} 点已到账。`, false);
      onBalance(result.balance ?? null);
      paint();
    } else if (["closed", "cancelled", "failed", "expired"].includes(result.status.toLowerCase())) {
      wallet.order = { ...wallet.order, ...result };
      status("订单已关闭或失效，请确认未付款后重新选择金额。", false);
      paint();
    } else status("等待支付宝确认付款结果，支付成功后点数自动到账。", false);
  } catch (error) {
    if (epoch === wallet.epoch) status(`暂时无法确认到账，将继续重试：${String(error instanceof Error ? error.message : error)}。请勿重复支付。`, true);
  } finally {
    if (epoch === wallet.epoch) {
      wallet.checking = false;
      const current = host?.querySelector<HTMLButtonElement>("#wallet-check");
      if (current) current.disabled = false;
      schedule();
    }
  }
}
