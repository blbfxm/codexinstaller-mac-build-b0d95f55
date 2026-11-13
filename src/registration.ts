import { invoke } from "@tauri-apps/api/core";

export type AccountResult = { ok: boolean; message?: string; balance?: number; apiKey?: string; created?: boolean; account?: { id: number; username?: string; email?: string; mainKami?: string; mainKamiMasked?: string; hasApiKey?: boolean }; installerConfig?: Record<string, string> };

export async function accountRequest(action: string, payload: Record<string, unknown> = {}) {
  return invoke<AccountResult>("account_portal", { action, payload });
}

export function renderRegistration(host: HTMLElement, back: () => void, complete: (result: AccountResult) => Promise<void>) {
  host.innerHTML = `<section class="registration-panel"><button id="registration-back" class="button quiet" type="button">← 返回登录</button><h2>注册坚果账户</h2><p class="muted">邮箱验证后即可登录助手，账户与网页中转平台通用。</p><form id="registration-form"><label>电子邮箱<input id="register-email" type="email" autocomplete="email" required placeholder="输入常用邮箱"></label><label>图形验证码<div class="registration-code"><input id="register-captcha" autocomplete="off" placeholder="图片中的字符"><button id="register-captcha-refresh" type="button" title="点击刷新验证码"><img id="register-captcha-image" alt="图形验证码"></button></div></label><label>邮箱验证码<div class="registration-code"><input id="register-email-code" required inputmode="numeric" autocomplete="one-time-code" placeholder="输入邮件验证码"><button id="register-send-code" class="button outline" type="button">发送验证码</button></div></label><label>登录密码<input id="register-password" type="password" minlength="6" autocomplete="new-password" required placeholder="至少 6 位"></label><label>确认密码<input id="register-password-confirm" type="password" minlength="6" autocomplete="new-password" required placeholder="再次输入密码"></label><p id="registration-status" role="status" class="error-line"></p><button id="register-submit" class="button primary full" type="submit">注册并登录</button></form></section>`;
  let challenge = "", expires = 0, interval = 0;
  const status = host.querySelector<HTMLElement>("#registration-status")!;
  const send = host.querySelector<HTMLButtonElement>("#register-send-code")!;
  const value = (id: string) => host.querySelector<HTMLInputElement>(`#${id}`)!.value.trim();
  const errorText = (e: unknown) => typeof e === "string" ? e : e instanceof Error ? e.message : "请求失败，请检查网络后重试";
  const refresh = async () => {
    try {
      const qr = await invoke<{ challenge: string; image: string }>("account_portal", { action: "captcha", payload: {} });
      if (!status.isConnected) return;
      challenge = qr.challenge; host.querySelector<HTMLImageElement>("#register-captcha-image")!.src = qr.image;
      host.querySelector<HTMLInputElement>("#register-captcha")!.value = "";
    } catch (e) { if (status.isConnected) status.textContent = errorText(e); }
  };
  host.querySelector("#registration-back")!.addEventListener("click", () => { window.clearInterval(interval); back(); });
  host.querySelector("#register-captcha-refresh")!.addEventListener("click", () => void refresh());
  send.addEventListener("click", async () => {
    if (!value("register-email") || !value("register-captcha") || !challenge) { status.textContent = "请填写邮箱和图形验证码"; return; }
    send.disabled = true; send.textContent = "发送中…"; status.textContent = "";
    try {
      const result = await accountRequest("sendCode", { email: value("register-email"), captcha: value("register-captcha"), captchaChallenge: challenge });
      if (!result.ok) throw new Error(result.message || "验证码发送失败");
      status.textContent = result.message || "验证码已发送，请查收邮箱";
      expires = Date.now() + 60_000;
      interval = window.setInterval(() => {
        if (!send.isConnected) { window.clearInterval(interval); return; }
        const seconds = Math.ceil((expires - Date.now()) / 1000);
        send.textContent = seconds > 0 ? `${seconds} 秒后重发` : "发送验证码";
        send.disabled = seconds > 0;
        if (seconds <= 0) window.clearInterval(interval);
      }, 500);
    } catch (e) { status.textContent = errorText(e); send.disabled = false; send.textContent = "发送验证码"; }
    finally { void refresh(); }
  });
  host.querySelector("#registration-form")!.addEventListener("submit", async event => {
    event.preventDefault();
    const password = host.querySelector<HTMLInputElement>("#register-password")!.value;
    if (password !== host.querySelector<HTMLInputElement>("#register-password-confirm")!.value) { status.textContent = "两次输入的密码不一致"; return; }
    const submit = host.querySelector<HTMLButtonElement>("#register-submit")!;
    submit.disabled = true; status.textContent = "正在注册…";
    try {
      const bytes = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(password));
      const passwordHash = [...new Uint8Array(bytes)].map(b => b.toString(16).padStart(2, "0")).join("");
      const result = await accountRequest("register", { email: value("register-email"), emailCode: value("register-email-code"), passwordHash });
      if (!result.ok || !result.account) throw new Error(result.message || "注册失败");
      window.clearInterval(interval);
      await complete(result);
    } catch (e) { if (status.isConnected) { status.textContent = errorText(e); submit.disabled = false; } }
  });
  void refresh();
}
