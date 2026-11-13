export type CodexSetupState = { onboardingComplete: boolean; fullAccessEnabled: boolean };

type SetupActions = {
  readState(): Promise<CodexSetupState>;
  stop(): Promise<unknown>;
  configure(): Promise<unknown>;
  launch(): Promise<unknown>;
  selectFullAccess(): Promise<unknown>;
  sleep(ms: number): Promise<unknown>;
  now(): number;
  cancelled(): boolean;
  report(message: string): void;
};

// Codex's onboarding completion handler resets agent mode and menu visibility.
// Wait for that handler's persisted completion marker, not a fixed launch delay.
export async function finishCodexSetup(actions: SetupActions): Promise<void> {
  let status = await actions.readState();
  const wasOnboarding = !status.onboardingComplete;
  const deadline = actions.now() + 10 * 60 * 1000;
  while (!status.onboardingComplete) {
    actions.report("请在 Codex 窗口完成首次使用引导；完成后助手会自动应用供应商和完整访问权限。");
    if (actions.cancelled()) throw new Error("DOWNLOAD_CANCELLED");
    if (actions.now() >= deadline) {
      throw new Error("尚未完成 Codex 首次使用引导。完成引导后，请再次点击一键安装并配置；已安装的软件不会重复下载。");
    }
    await actions.sleep(1000);
    status = await actions.readState();
  }
  if (actions.cancelled()) throw new Error("DOWNLOAD_CANCELLED");
  if (wasOnboarding || !status.fullAccessEnabled) {
    actions.report("首次使用引导已完成，正在应用供应商和完整访问权限…");
    await actions.stop();
    await actions.configure();
    await actions.launch();
  }
  // Best-effort UI alignment; failures must not alarm the user.
  actions.report("正在应用 Codex 完整访问权限…");
  try { await actions.selectFullAccess(); } catch { /* keep configuration result */ }
}
