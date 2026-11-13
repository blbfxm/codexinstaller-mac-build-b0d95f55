const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync(require('node:path').join(__dirname, '../src-tauri/src/codex_permissions.js'), 'utf8');

function fixture({ selected = false, blocked = false, confirmation = false } = {}) {
  let open = false, dialog = false, chosen = selected, clicks = 0;
  const node = (text, attrs, click) => ({
    textContent: text, getClientRects: () => [1], getAttribute: key => attrs()[key] ?? null,
    click: () => { clicks++; click(); },
  });
  const trigger = node('', () => ({ 'aria-expanded': String(open) }), () => { open = true; });
  Object.defineProperty(trigger, 'textContent', { get: () => chosen ? '完全访问权限' : '请求批准' });
  const item = node('完全访问权限 不受限制地访问互联网和电脑上的任何文件',
    () => ({ 'aria-disabled': String(blocked) }), () => {
      open = false;
      if (confirmation) dialog = true; else chosen = true;
    });
  const confirm = node('确认', () => ({}), () => { chosen = true; dialog = false; });
  const modal = node('启用完全访问权限', () => ({}), () => {});
  modal.querySelectorAll = () => [confirm];
  const document = {
    querySelectorAll(selector) {
      if (selector.includes('data-composer-navigation-target')) return [trigger];
      if (selector.includes('role="dialog"')) return dialog ? [modal] : [];
      if (selector.includes('role="menuitemradio"')) return open ? [item] : [];
      return [];
    },
    getElementById: () => null,
  };
  return { run: () => vm.runInNewContext(source, { document }), clicks: () => clicks, chosen: () => chosen };
}

// A persisted config can say full-access while the real trigger still says Ask.
const first = fixture();
assert.equal(first.run().stage, 'opening-permissions');
assert.equal(first.run().stage, 'selecting-full-access');
assert.equal(first.run().selected, true);
assert.equal(first.chosen(), true);
const existing = fixture({ selected: true });
assert.equal(existing.run().selected, true);
assert.equal(existing.clicks(), 0);
const confirm = fixture({ confirmation: true });
assert.equal(confirm.run().selected, false);
assert.equal(confirm.run().selected, false);
assert.equal(confirm.run().stage, 'confirming');
assert.equal(confirm.run().selected, true);
const blocked = fixture({ blocked: true });
blocked.run();
assert.equal(blocked.run().stage, 'blocked-by-policy');
assert.equal(blocked.chosen(), false);
const absent = vm.runInNewContext(source, { document: { querySelectorAll: () => [] } });
assert.equal(absent.selected, false);
assert.equal(absent.stage, 'waiting-for-composer');
console.log('5 live permission selection scenarios passed');
