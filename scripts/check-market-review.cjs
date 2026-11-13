// Isolated review-page checks. Does not contact or modify the production server.
const fs = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const file = process.argv[2] || 'F:/development/taobao/nut-server/src/main/resources/static/jg_panel_9527.html';
const html = fs.readFileSync(file, 'utf8');
for (const match of html.matchAll(/<script\b[^>]*>([\s\S]*?)<\/script>/g)) new vm.Script(match[1]);
const markup = html.replace(/<script\b[^>]*>[\s\S]*?<\/script>/g, '');
const ids = [...markup.matchAll(/\bid="([^"]+)"/g)].map(m => m[1]);
const reviewIds = ids.filter(id => /^(pluginReview|pluginAutoReview|sharedSkill|codexSkill)/.test(id));
assert.equal(new Set(reviewIds).size, reviewIds.length, 'Review panels must not duplicate IDs');
const nodes = new Map();
function node() {
  let content = '';
  return { value: 'pending', textContent: '', style: {}, children: [],
    classList: { hidden: true, add() { this.hidden = true; }, remove() { this.hidden = false; } },
    append(...items) { this.children.push(...items); }, appendChild(item) { this.children.push(item); }, scrollIntoView() {},
    set innerHTML(value) { content = value; for (const m of value.matchAll(/\bid="([^"]+)"/g)) nodes.set(m[1], node()); },
    get innerHTML() { return content; }
  };
}
ids.forEach(id => nodes.set(id, node()));
const calls = [], notices = [];
const rows = [{ id: 1, kind: 'plugin', name: 'plugin-only', status: 'pending' }, { id: 2, kind: 'skill', name: 'skill-only', status: 'pending' }];
const context = vm.createContext({ console, document: { createElement: node },
  $: id => { assert.ok(nodes.has(id), 'Missing DOM target: ' + id); return nodes.get(id); },
  escapeAdmin: value => String(value ?? ''), toast: (...args) => notices.push(args), prompt: () => '',
  api: async (method, route, body) => {
    calls.push({ method, route, body });
    if (route === '/codex-skills/pending') return { ok: true, data: [{ id: 9, name: 'legacy-skill' }] };
    const url = new URL(route, 'https://test.invalid');
    if (url.pathname === '/codex-plugins') return { ok: true, data: rows.filter(r => r.kind === url.searchParams.get('kind') && r.status === url.searchParams.get('status')) };
    const item = rows.find(r => route.startsWith('/codex-plugins/' + r.id + '/'));
    assert.ok(item, route);
    if (route.endsWith('/content')) return { ok: true, data: { ...item, files: [{ path: 'SKILL.md', bytes: 10, preview: 'Read only instructions' }], screenshots: [] } };
    if (route.endsWith('/auto-review')) return { ok: true, data: { verdict: 'review', riskLevel: 'low', summary: 'Review this content', spec: [], findings: [] } };
    if (route.endsWith('/review')) { item.status = body.action === 'approve' ? 'published' : 'rejected'; return { ok: true }; }
    throw Error(route);
  }
});
vm.runInContext(html.slice(html.indexOf('var pluginReviewBadges='), html.indexOf('// ── 注册用户管理')), context);
(async () => {
  await context.loadPluginReviews(); await context.loadCodexSkillReviews();
  assert.match(nodes.get('pluginReviewTable').innerHTML, /plugin-only/);
  assert.doesNotMatch(nodes.get('pluginReviewTable').innerHTML, /skill-only/);
  assert.match(nodes.get('sharedSkillReviewTable').innerHTML, /skill-only/);
  assert.doesNotMatch(nodes.get('sharedSkillReviewTable').innerHTML, /plugin-only/);
  assert.match(nodes.get('sharedSkillReviewTable').innerHTML, /viewPluginReview\(2,'skill'\)/);
  assert.match(nodes.get('sharedSkillReviewTable').innerHTML, /autoReviewPlugin\(2,'skill'\)/);
  await context.viewPluginReview(2, 'skill');
  assert.equal(nodes.get('sharedSkillReviewViewer').classList.hidden, false);
  assert.equal(nodes.get('pluginReviewViewer').classList.hidden, true);
  assert.equal(nodes.get('sharedSkillReviewFiles').children.length, 1);
  await context.autoReviewPlugin(2, 'skill');
  assert.equal(nodes.get('sharedSkillAutoReviewBox').classList.hidden, false);
  assert.equal(nodes.get('pluginAutoReviewBox').classList.hidden, true);
  await context.reviewPlugin(2, 'approve', 'skill');
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(rows[1].status, 'published');
  assert.doesNotMatch(nodes.get('sharedSkillReviewTable').innerHTML, /skill-only/);
  assert.match(nodes.get('sharedSkillReviewTable').innerHTML, /暂无技能/);
  nodes.get('sharedSkillReviewStatus').value = 'published';
  await context.loadCodexSkillReviews();
  assert.match(nodes.get('sharedSkillReviewTable').innerHTML, /skill-only/);
  await context.loadLegacyCodexSkillReviews();
  assert.match(nodes.get('codexSkillReviewTable').innerHTML, /legacy-skill/);
  assert.match(nodes.get('sharedSkillReviewTable').innerHTML, /skill-only/);
  assert.equal(notices.some(n => n[1] === false), false, JSON.stringify(notices));
  console.log('PASS: separate plugin/skill review lists, detail viewers, auto-review results, approval refresh, status filtering and legacy submissions; JavaScript parses and DOM IDs are unique');
})().catch(error => { console.error(error); process.exitCode = 1; });
