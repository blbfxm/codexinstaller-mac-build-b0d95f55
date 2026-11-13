import { invoke } from "@tauri-apps/api/core";

type Tokens = { input: number; cached: number; output: number; reasoning: number; total: number };
type Group = { name: string; tokens: Tokens; requests: number };
type Usage = { period: string; tokens: Tokens; requests: number; sessions: number; models: Group[]; days: Group[]; complete: boolean; files: number; updatedAt: string; notice: string };
type Entry = { time: string; model: string; sessionId: string; requestId: string | null; tokens: Tokens; note: string };
type LogPage = { page: number; pageSize: number; total: number; totalPages: number; entries: Entry[]; complete: boolean; notice: string; updatedAt: string };
const escape = (s: unknown) => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
const format = (n: number) => Number(n || 0).toLocaleString("zh-CN");
const date = (time: string) => new Date(time).toLocaleString("zh-CN", { hour12: false });
let period = "today", section: "summary" | "logs" = "summary", page = 1, requestId = 0;

function drawSummary(result: Usage) {
  const t = result.tokens;
  const recent = result.days.slice(-30), maximum = Math.max(1, ...recent.map(d => d.tokens.total));
  return `<div class="usage-metrics">${[["总 Token", t.total], ["输入 Token", t.input], ["输出 Token", t.output], ["缓存输入", t.cached]].map(([label, n]) => `<article><span>${label}</span><strong>${format(Number(n))}</strong></article>`).join("")}</div><p class="muted">${format(result.requests)} 次用量记录 · ${format(result.sessions)} 个会话 · ${format(result.files)} 份会话文件</p><p class="usage-notice ${result.complete ? "" : "partial"}">${escape(result.notice)}</p>${result.days.length ? `<h3>每日用量</h3><div class="usage-days">${recent.map(day => `<div class="usage-day"><span>${escape(day.name.slice(5))}</span><div><i style="width:${Math.max(1, day.tokens.total / maximum * 100)}%"></i></div><b>${format(day.tokens.total)}</b></div>`).join("")}</div>${result.days.length > 30 ? `<p class="muted">趋势展示最近 30 个有记录的日期，上方汇总包含全部所选时段。</p>` : ""}<h3>按模型统计</h3><div class="usage-table-wrap"><table class="usage-table"><thead><tr><th>模型</th><th>输入</th><th>输出</th><th>总 Token</th></tr></thead><tbody>${result.models.map(row => `<tr><td>${escape(row.name)}</td><td>${format(row.tokens.input)}</td><td>${format(row.tokens.output)}</td><td>${format(row.tokens.total)}</td></tr>`).join("")}</tbody></table></div>` : `<div class="empty-state">该时段暂无本机用量记录。使用 Codex 产生会话后，在此刷新查看。</div>`}<p class="microcopy">更新于 ${escape(date(result.updatedAt))}。本页为本机使用统计，不代表当前坚果账户的结算账单。</p>`;
}

function drawLogs(result: LogPage) {
  return `<div class="usage-log-heading"><h3>请求使用日志</h3><span>共 ${format(result.total)} 条 · 每页 20 条</span></div><p class="microcopy">按本机 Codex 已记录的请求用量显示，最新记录在前；未写入本机会话的请求不包含在内。</p>${!result.complete ? `<p class="usage-notice partial">${escape(result.notice)}</p>` : ""}<div class="usage-table-wrap"><table class="usage-table usage-log-table"><thead><tr><th>时间 / 模型</th><th>输入</th><th>缓存</th><th>输出</th><th>总 Token</th><th>详情</th></tr></thead><tbody>${result.entries.length ? result.entries.map(row => `<tr data-usage-log-row><td><time datetime="${escape(row.time)}">${escape(date(row.time))}</time><strong>${escape(row.model)}</strong></td><td>${format(row.tokens.input)}</td><td>${format(row.tokens.cached)}</td><td>${format(row.tokens.output)}</td><td>${format(row.tokens.total)}</td><td><details><summary>查看</summary><dl><dt>会话 ID</dt><dd>${escape(row.sessionId)}</dd><dt>请求 ID</dt><dd>${escape(row.requestId || "本机未记录")}</dd><dt>推理 Token</dt><dd>${format(row.tokens.reasoning)}</dd>${row.note ? `<dt>记录说明</dt><dd>${escape(row.note)}</dd>` : ""}</dl></details></td></tr>`).join("") : `<tr><td colspan="6" class="usage-empty">该时段暂无请求使用记录。</td></tr>`}</tbody></table></div><nav class="usage-pagination" aria-label="使用日志分页"><span>第 ${result.page} / ${result.totalPages} 页</span><div><button type="button" class="button outline" id="usage-log-prev" ${result.page <= 1 ? "disabled" : ""}>上一页</button><button type="button" class="button outline" id="usage-log-next" ${result.page >= result.totalPages ? "disabled" : ""}>下一页</button></div></nav><p class="microcopy">更新于 ${escape(date(result.updatedAt))}。缓存输入已包含在输入中，推理已包含在输出中。</p>`;
}

export function renderUsage(host: HTMLElement) {
  const version = ++requestId;
  host.innerHTML = `<section class="usage-workspace"><div class="section-heading"><div><h2>本机 Codex 用量</h2><p class="muted">查看用量统计与逐次请求的使用日志。</p></div><button type="button" class="button quiet" id="usage-refresh">刷新用量</button></div><div class="usage-view-tabs" role="tablist" aria-label="用量页面"><button type="button" role="tab" aria-selected="${section === "summary"}" data-usage-section="summary">用量统计</button><button type="button" role="tab" aria-selected="${section === "logs"}" data-usage-section="logs">使用日志</button></div><div class="usage-periods" role="tablist" aria-label="用量时段">${[["today", "当天"], ["7d", "7天"], ["30d", "30天"], ["all", "全部"]].map(([key, label]) => `<button type="button" role="tab" aria-selected="${key === period}" data-usage-period="${key}">${label}</button>`).join("")}</div><div id="usage-result" aria-live="polite"></div></section>`;
  const target = host.querySelector<HTMLElement>("#usage-result")!;
  const refresh = host.querySelector<HTMLButtonElement>("#usage-refresh")!;
  let loadId = 0;
  const alive = () => version === requestId && host.isConnected && !!target.isConnected;
  const load = async () => {
    const current = ++loadId, requestedPeriod = period, requestedSection = section;
    refresh.disabled = true; refresh.textContent = "读取中…"; target.setAttribute("aria-busy", "true");
    target.innerHTML = `<p class="muted">${section === "logs" ? "正在读取使用日志…" : "正在读取本机会话用量…"}</p>`;
    try {
      if (requestedSection === "logs") {
        const data = await invoke<LogPage>("codex_usage_logs", { period: requestedPeriod, page });
        if (!alive() || current !== loadId) return;
        page = data.page; target.innerHTML = drawLogs(data);
        host.querySelector("#usage-log-prev")?.addEventListener("click", () => { page = Math.max(1, page - 1); void load(); });
        host.querySelector("#usage-log-next")?.addEventListener("click", () => { page = Math.min(data.totalPages, page + 1); void load(); });
      } else {
        const data = await invoke<Usage>("codex_usage", { period: requestedPeriod });
        if (!alive() || current !== loadId) return;
        target.innerHTML = drawSummary(data);
      }
    } catch (e) { if (alive() && current === loadId) target.textContent = typeof e === "string" ? e : "用量读取失败，请稍后重试"; }
    finally { if (alive() && current === loadId) { refresh.disabled = false; refresh.textContent = "刷新用量"; target.setAttribute("aria-busy", "false"); } }
  };
  host.querySelectorAll<HTMLButtonElement>("[data-usage-section]").forEach(button => button.addEventListener("click", () => { section = button.dataset.usageSection as "summary" | "logs"; page = 1; renderUsage(host); }));
  host.querySelectorAll<HTMLButtonElement>("[data-usage-period]").forEach(button => button.addEventListener("click", () => { period = button.dataset.usagePeriod!; page = 1; renderUsage(host); }));
  refresh.addEventListener("click", () => void load());
  void load();
}