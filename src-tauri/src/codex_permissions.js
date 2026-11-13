// Use Codex's own permission controls so its live state and persistence agree.
(() => {
  const visible = node => node && node.getClientRects().length > 0;
  const text = node => (node?.textContent || "").replace(/\s+/g, " ").trim();
  const full = value => /完全访问权限|完整访问权限|full access/i.test(value);
  const buttons = selector => Array.from(document.querySelectorAll(selector)).filter(visible);
  const trigger = buttons('[data-composer-navigation-target="permissions"]')[0];
  if (!trigger || trigger.disabled || trigger.getAttribute("aria-busy") === "true") {
    return { selected: false, stage: "waiting-for-composer" };
  }
  const label = text(trigger) || trigger.getAttribute("aria-describedby")?.split(" ")
    .map(id => text(document.getElementById(id))).join(" ") || "";
  const expanded = trigger.getAttribute("aria-expanded") === "true";
  if (full(label) && !expanded) return { selected: true, stage: "confirmed" };
  // Full access may require Codex's own explicit confirmation dialog.
  const dialog = buttons('[role="dialog"],[role="alertdialog"]').find(node => full(text(node)));
  if (dialog) {
    const confirm = Array.from(dialog.querySelectorAll('button')).filter(visible)
      .find(node => !node.disabled && /^(确认|确定|开启|启用|Confirm|Turn on|Enable)$/i.test(text(node)));
    if (confirm) { confirm.click(); return { selected: false, stage: "confirming" }; }
    return { selected: false, stage: "waiting-for-confirmation" };
  }
  if (!expanded) {
    trigger.click();
    return { selected: false, stage: "opening-permissions" };
  }
  const item = buttons('[role="menuitemradio"],[role="menuitem"],[role="option"]')
    .find(node => full(text(node)));
  if (!item) return { selected: false, stage: "waiting-for-permission-option" };
  if (item.getAttribute("aria-disabled") === "true") {
    return { selected: false, stage: "blocked-by-policy" };
  }
  // Clicking even a checked item closes the menu; success is checked on the trigger.
  item.click();
  return { selected: false, stage: "selecting-full-access" };
})();
