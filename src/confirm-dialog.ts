// Native confirm dialogs are not reliably available in desktop WebViews.
export function confirmDialog(title: string, message: string, confirmLabel: string): Promise<boolean> {
  if (document.querySelector("#app-confirm-dialog")) return Promise.resolve(false);
  return new Promise(resolve => {
    const dialog = document.createElement("dialog");
    dialog.id = "app-confirm-dialog";
    dialog.className = "builtin-plugin-dialog";
    dialog.setAttribute("aria-labelledby", "app-confirm-title");
    dialog.setAttribute("aria-describedby", "app-confirm-message");
    dialog.innerHTML = `<header><h2 id="app-confirm-title"></h2></header><div class="plugin-help-body"><p id="app-confirm-message"></p></div><footer><button type="button" class="button outline" data-confirm-cancel>取消</button><button type="button" class="button primary" data-confirm-accept></button></footer>`;
    dialog.querySelector("h2")!.textContent = title;
    const copy = dialog.querySelector<HTMLElement>("#app-confirm-message")!;
    copy.textContent = message;
    copy.style.whiteSpace = "pre-line";
    const cancel = dialog.querySelector<HTMLButtonElement>("[data-confirm-cancel]")!;
    const accept = dialog.querySelector<HTMLButtonElement>("[data-confirm-accept]")!;
    accept.textContent = confirmLabel;
    cancel.addEventListener("click", () => dialog.close());
    accept.addEventListener("click", () => dialog.close("confirmed"));
    dialog.addEventListener("close", () => {
      const confirmed = dialog.returnValue === "confirmed";
      dialog.remove();
      resolve(confirmed);
    }, { once: true });
    document.body.append(dialog);
    dialog.showModal();
    cancel.focus();
  });
}
