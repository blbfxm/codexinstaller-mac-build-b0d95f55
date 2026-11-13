const assert = require('node:assert/strict');
const fs = require('node:fs');
const ts = require('typescript');
const source = fs.readFileSync(require('node:path').join(__dirname, '../src/finish-codex-setup.ts'), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS } }).outputText;
const entry = { exports: {} };
new Function('exports', 'module', compiled)(entry.exports, entry);
const { finishCodexSetup } = entry.exports;

async function check(completedAt, initialAccess, cancelAt = Infinity) {
  let time = 0;
  let access = initialAccess;
  let reset = false;
  const calls = [];
  const actions = {
    readState: async () => {
      if (!reset && time >= completedAt && completedAt > 0) {
        reset = true;
        access = false; // Actual non_coding onboarding handler overwrites it.
      }
      return { onboardingComplete: time >= completedAt, fullAccessEnabled: access };
    },
    stop: async () => { assert.ok(time >= completedAt, 'must not interrupt onboarding'); calls.push('stop'); },
    configure: async () => { calls.push('configure'); access = true; },
    launch: async () => { calls.push('launch'); },
    selectFullAccess: async () => { calls.push('selectFullAccess'); },
    sleep: async ms => { time += ms; },
    now: () => time,
    cancelled: () => time >= cancelAt,
    report: () => {},
  };
  return { run: () => finishCodexSetup(actions), calls, access: () => access };
}

(async () => {
  // User finishes onboarding well after both old fixed delays (2 s / 15 s).
  const fresh = await check(60000, true);
  await fresh.run();
  assert.deepEqual(fresh.calls, ['stop', 'configure', 'launch', 'selectFullAccess']);
  assert.equal(fresh.access(), true);
  const existing = await check(0, true);
  await existing.run();
  assert.deepEqual(existing.calls, ['selectFullAccess']);
  const disabled = await check(0, false);
  await disabled.run();
  assert.deepEqual(disabled.calls, ['stop', 'configure', 'launch', 'selectFullAccess']);
  const cancelled = await check(60000, true, 5000);
  await assert.rejects(cancelled.run(), /DOWNLOAD_CANCELLED/);
  assert.deepEqual(cancelled.calls, []);
  const unfinished = await check(Infinity, true);
  await assert.rejects(unfinished.run(), /尚未完成/);
  assert.deepEqual(unfinished.calls, []);
  // A failed best-effort UI check must not surface an error to the user.
  await finishCodexSetup({
    readState: async () => ({ onboardingComplete: true, fullAccessEnabled: true }),
    stop: async () => {}, configure: async () => {}, launch: async () => {},
    selectFullAccess: async () => { throw new Error('live selection failed'); },
    sleep: async () => {}, now: () => 0, cancelled: () => false, report: () => {},
  });
  console.log('6 onboarding/permission scenarios passed');
})().catch(error => { console.error(error); process.exitCode = 1; });
