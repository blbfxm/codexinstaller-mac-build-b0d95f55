export type CodexResetReport = { backupDirectories: string[] };

export function showCodexResetDialog(reset: () => Promise<CodexResetReport>) {
  if (document.querySelector("#codex-reset-dialog")) return;
  const dialog = document.createElement("dialog");
  dialog.id = "codex-reset-dialog";
  dialog.className = "builtin-plugin-dialog";
  dialog.setAttribute("aria-labelledby", "codex-reset-title");
  dialog.innerHTML = `<header><div><h2 id="codex-reset-title">恢复 Codex 初始状态</h2><p class="microcopy">保留以前的会话记录</p></div></header>
    <div class="plugin-help-body"><p>适用于卸载重装后仍跳过登录、切换供应商后旧会话提示不匹配的情况。</p>
    <ul><li>关闭 Codex，停止微信连接，重置本地登录凭证和模型连接，恢复 OpenAI 官方登录方式。</li>
    <li>以前的会话记录、附件、已安装的技能和插件保持原样。助手中的供应商列表和账户余额不受影响。</li>
    <li>恢复前会备份被修改的配置。请先完全退出 Codex++，避免它重新写入配置。</li></ul>
    <p>恢复后，请启动 Codex 并登录原来的 OpenAI 官方账号，再打开原有的官方会话。重新启用第三方供应商后，官方旧会话仍可能提示供应商不匹配。</p>
    <p id="codex-reset-feedback" role="status"></p><div id="codex-reset-backups"></div></div>
    <footer><button type="button" class="button outline" id="codex-reset-cancel">取消</button><button type="button" class="button primary" id="codex-reset-confirm">备份并恢复</button></footer>`;
  const confirm = dialog.querySelector<HTMLButtonElement>("#codex-reset-confirm")!;
  const cancel = dialog.querySelector<HTMLButtonElement>("#codex-reset-cancel")!;
  const feedback = dialog.querySelector<HTMLElement>("#codex-reset-feedback")!;
  let busy = false, done = false;
  cancel.addEventListener("click", () => dialog.close());
  dialog.addEventListener("cancel", event => { if (busy) event.preventDefault(); });
  confirm.addEventListener("click", async () => {
    if (busy) return;
    if (done) { dialog.close(); return; }
    busy = true; confirm.disabled = true; cancel.disabled = true;
    confirm.textContent = "正在恢复…"; feedback.textContent = "正在备份配置并恢复官方登录，请稍候…";
    try {
      const report = await reset();
      done = true;
      feedback.textContent = "恢复完成，原有会话记录已保留。现在可以启动 Codex，登录原来的 OpenAI 官方账号。";
      const backups = dialog.querySelector<HTMLElement>("#codex-reset-backups")!;
      if (report.backupDirectories.length) {
        const label = document.createElement("p"); label.textContent = "原配置备份位置（包含登录凭证，请妥善保管）："; backups.append(label);
        for (const path of report.backupDirectories) { const line = document.createElement("p"); line.className = "codex-reset-path"; line.textContent = path; backups.append(line); }
      }
      confirm.textContent = "完成"; cancel.hidden = true;
    } catch (error) {
      feedback.textContent = typeof error === "string" ? error : error instanceof Error ? error.message : "恢复失败，请稍后重试。";
      confirm.textContent = "重试";
    } finally { busy = false; confirm.disabled = false; cancel.disabled = false; }
  });
  dialog.addEventListener("close", () => dialog.remove(), { once: true });
  document.body.append(dialog); dialog.showModal(); cancel.focus();
}
