// Runs an isolated Edge window with mocked native APIs; never modifies real accounts or Codex.
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { spawn } = require('node:child_process');
const assert = require('node:assert/strict');
const root = path.resolve(__dirname, '..');
const publicPlugins = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/catalog/public-plugins.json'), 'utf8')).plugins;
const temp = process.env.TEMP || require('node:os').tmpdir();
const profile = fs.mkdtempSync(path.join(temp, 'assistant-account-ui-'));
const sleep = ms => new Promise(r => setTimeout(r, ms));
let browser, ws, server, sequence = 0;
const pending = new Map(), errors = [];
function call(method, params = {}) { return new Promise((resolve, reject) => { const id = ++sequence; pending.set(id, { resolve, reject }); ws.send(JSON.stringify({ id, method, params })); }); }
async function evaluate(expression) { const r = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }); if (r.exceptionDetails) throw Error(JSON.stringify(r.exceptionDetails)); return r.result.value; }
async function until(expression) { for (let i = 0; i < 150; i++) { if (await evaluate(expression)) return; await sleep(100); } throw Error('Timeout: ' + expression + ' ' + JSON.stringify(await evaluate("({feedback:document.querySelector('#codex-reset-feedback')?.textContent,button:document.querySelector('#codex-reset-confirm')?.textContent,calls:window.calls?.slice(-6)})"))); }
async function snapshot(name) {
  const directory=process.env.ASSISTANT_UI_SNAPSHOTS;if(!directory)return;
  fs.mkdirSync(directory,{recursive:true});const result=await call('Page.captureScreenshot',{format:'png'});
  fs.writeFileSync(path.join(directory,name+'.png'),Buffer.from(result.data,'base64'));
}
const injection = `
  window.publicPlugins=${JSON.stringify(publicPlugins)};window.publicInstalled=[];
  window.fixtureCallbacks={};window.fixtureListeners={};
  window.installLocationPath='';window.installLocationsUsed=[];
  window.testHasKey=false;
  window.imagePlugin={installed:false,enabled:false,credentialReady:false,version:'1.0.0',baseUrl:'https://api.jianguo.ltd'};
  window.vectorPlugin={installed:false,enabled:false,credentialReady:false,version:'1.0.0',baseUrl:'https://api.jianguo.ltd'};
  window.marketInstalled=[];window.marketApproved=false;window.marketSubmitted=false;
  window.marketItem={id:9,name:'社区示例插件',slug:'community-example',version:'1.0.0',author:'社区作者',description:'用户分享的实用插件 <img src=x onerror=alert(1)>',instructions:'输入 @，选择插件，然后描述任务。',price:10,downloads:5,status:'published',screenshots:[],requiresAuthorization:true,authorizationDetails:'申请示例 API Key <script>只是说明</script>',billingDetails:'服务商按请求计费，查询官网价目'};
  { const c=document.createElement('canvas');c.width=640;c.height=360;const g=c.getContext('2d');const fill=g.createLinearGradient(0,0,640,360);fill.addColorStop(0,'#17473e');fill.addColorStop(1,'#4aa796');g.fillStyle=fill;g.fillRect(0,0,640,360);g.fillStyle='#fff';g.font='bold 44px sans-serif';g.fillText('社区插件 · 介绍图',50,170);g.font='24px sans-serif';g.fillText('工作更轻松，创作更自由',50,220);window.marketItem.coverImage=c.toDataURL('image/png'); }
  window.qrBehavior='success';window.folderBehavior='success';window.wxPollMode='confirmed';
  window.wx={bound:false,running:false,workspace:'',writable:false,phase:'未开启',received:0,replied:0,error:'',activity:''};
  if(command==='load_auto_login_kami')return null;
  if(command==='account_portal'){
    if(args.action==='captcha')return {challenge:'test-challenge',image:'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aDakAAAAASUVORK5CYII='};
    if(args.action==='sendCode')return {ok:true};
    if(args.action==='ensureKey'){
      if(window.failKey)throw '测试：账户服务暂不可用';
      window.testHasKey=true;return {ok:true,apiKey:'fixture-account-key',created:true,balance:10,account:{id:2,username:'tester',mainKami:'fixture-account-key',hasApiKey:true}};
    }
    return {ok:true,balance:10,account:{id:window.testHasKey?2:1,username:'tester',mainKami:window.testHasKey?'fixture-account-key':'',hasApiKey:window.testHasKey}};
  }
  if(command==='image_plugin_status')return {...window.imagePlugin};
  if(command==='public_plugin_market'){
    if(args.action==='list'){
      if(window.failPublicList)throw '测试：插件目录暂时无法读取';
      return {items:window.publicPlugins,installed:[...window.publicInstalled],origin:args.payload.refresh?'online':'bundled',updatedAt:'2026-10-08',warning:window.publicWarning||''};
    }
    if(args.action==='install'){
      if(window.holdPublicInstall)await new Promise(resolve=>window.resolvePublicInstall=resolve);
      if(window.failPublicInstall)throw '测试：插件下载中断';
      const p=window.publicPlugins.find(p=>p.id===args.payload.id);const result={id:p.id,tree:p.tree,version:p.version,enabled:true};
      window.publicInstalled=[...window.publicInstalled.filter(i=>i.id!==p.id),result];return result;
    }
    if(args.action==='uninstall'){window.publicInstalled=window.publicInstalled.filter(i=>i.id!==args.payload.id);return {id:args.payload.id};}
  }
  if(command==='plugin_market'){
    if(args.action==='installed')return [...window.marketInstalled];
    if(args.payload?.kind==='skill'){
      if(args.action==='list')return {items:window.sharedSkill?.status==='published'?[{...window.sharedSkill}]:[],total:window.sharedSkill?.status==='published'?1:0,page:0};
      if(args.action==='mine')return {items:window.sharedSkill?[{...window.sharedSkill}]:[],revenue:0};
      if(args.action==='upload'){window.sharedSkill={...window.marketItem,...args.payload,id:29,kind:'skill',status:'pending',downloads:0};return {id:29,status:'pending'};}
      if(args.action==='detail'||args.action==='edit')return {...window.sharedSkill};
      if(args.action==='install'){window.marketInstalled.push({id:29,slug:'shared-skill',version:'1.0.0',kind:'skill'});return {id:29};}
      if(args.action==='uninstall'){window.marketInstalled=window.marketInstalled.filter(i=>i.id!==29);return {id:29};}
      if(args.action==='end'){window.sharedSkill.status='withdrawn';return {id:29};}
    }
    if(args.action==='list'){if(window.failMarketList)throw '插件市场服务尚未开放，请稍后再试；内置插件和技能仍可使用';const rows=[window.marketItem,...(window.marketApproved&&window.marketMineItem.status!=='withdrawn'?[{...window.marketMineItem,status:'published'}]:[])];return {items:rows.filter(x=>!args.payload.q||x.name.includes(args.payload.q)),total:rows.length,page:args.payload.page||0};}
    if(args.action==='detail')return {...window.marketItem};
    if(args.action==='mine')return {items:window.marketSubmitted?[{...window.marketMineItem,status:window.marketMineItem.status==='withdrawn'?'withdrawn':window.marketApproved?'published':'pending'}]:[],revenue:30};
    if(args.action==='edit'){if(window.failMarketEdit)throw '测试：无法加载分享';return {...window.marketMineItem};}
    if(args.action==='end'){if(window.failMarketEnd)throw '测试：结束分享失败';window.marketApproved=false;window.marketMineItem.status='withdrawn';return {id:args.payload.id,status:'withdrawn'};}
    if(args.action==='upload'){if(window.failMarketUpload)throw '测试：投稿保存失败，请重试';window.marketSubmitted=true;window.marketApproved=false;window.marketMineItem={...window.marketItem,...window.marketMineItem,...args.payload,id:10,status:'pending',...('screenshots' in args.payload?{coverImage:args.payload.screenshots[0]||''}:{})};return {id:10,status:'pending'};}
    if(args.action==='install'){if(window.failMarketInstall)throw '测试：账户余额不足，请先充值';window.marketInstalled=[{id:9,slug:'community-example',version:'1.0.0'}];return window.marketInstalled[0];}
    if(args.action==='uninstall'){window.marketInstalled=[];return {id:9};}
  }
  if(command==='vector_plugin_status')return {...window.vectorPlugin};
  if(command==='sync_builtin_plugin_credentials')return;
  if(command==='codex_install_options')return {path:window.installLocationPath,volumes:[{path:'D:/WindowsApps'.replace('/','\\\\'),label:'D: · 可用 100 GB'},{path:'E:/WindowsApps'.replace('/','\\\\'),label:'E: · 可用 200 GB'}]};
  if(command==='save_codex_install_location'){
    if(window.holdLocationSave)await new Promise(resolve=>window.resolveLocationSave=resolve);
    if(window.failLocationSave)throw '所选磁盘不可用';
    window.installLocationPath=args.path;return args.path;
  }
  if(command==='plugin:event|listen'){window.fixtureListeners[args.event]=window.fixtureCallbacks[args.handler];return 1;}
  if(command==='install_product'||command==='install_codex_windows_fallback'){
    window.installLocationsUsed.push(window.installLocationPath);
    if(window.holdInstall){
      window.downloadFixture={product:'Codex',downloaded:1024,total:10240,bytes_per_second:1024,resumable:true};
      window.fixtureListeners['installer-download-progress']?.({payload:window.downloadFixture});
      await new Promise(resolve=>window.resolveInstall=resolve);
    }
    if(window.failInstall)throw '当前 Windows 版本过低，请升级系统后重试。\\n'+('PowerShell 详细错误 <not-html>\\n'.repeat(1000));
    if(command==='install_codex_windows_fallback')window.testInstalled.codex=true;
    return 'installed';
  }
  if(command==='uninstall_builtin_plugin'){
    if(window.failUninstall)throw '测试：插件正在使用，暂时无法卸载';
    const key=args.plugin==='image'?'imagePlugin':'vectorPlugin';
    window[key]={...window[key],installed:false,enabled:false};return {...window[key]};
  }
  if(command==='install_vector_plugin'){
    if(window.failVectorPlugin)throw '测试：转矢量配置目录无法写入';
    window.vectorPlugin={...window.vectorPlugin,installed:true,enabled:true,credentialReady:true};return {...window.vectorPlugin};
  }
  if(command==='install_image_plugin'){
    if(window.failPlugin)throw '测试：插件配置目录无法写入';
    if(window.holdPlugin)await new Promise(resolve=>window.resolvePluginInstall=resolve);
    window.imagePlugin={...window.imagePlugin,installed:true,enabled:true,credentialReady:true};return {...window.imagePlugin};
  }
  if(command==='codex_usage')return {period:args.period,tokens:{total:150,input:100,output:50,cached:20,reasoning:10},requests:2,sessions:1,files:1,days:[{name:'2026-11-10',tokens:{total:150,input:100,output:50,cached:20},requests:2}],models:[{name:'fixture-model',tokens:{total:150,input:100,output:50},requests:2}],complete:true,notice:'本机用量，不等同平台扣费。',updatedAt:'2026-11-10T12:00:00+08:00'};
  if(command==='codex_usage_logs'){
    const total=args.period==='today'?3:45,pageSize=20,totalPages=Math.max(1,Math.ceil(total/pageSize)),page=Math.min(totalPages,Math.max(1,args.page));
    return {page,pageSize,total,totalPages,entries:Array.from({length:Math.min(20,total-(page-1)*20)},(_,i)=>({time:new Date(Date.UTC(2026,10,10,4,0,0)-((page-1)*20+i)*60000).toISOString(),model:'fixture-model',sessionId:'session-'+((page-1)*20+i),requestId:i===0?'fixture-request-'+page:null,tokens:{total:150,input:100,output:50,cached:20,reasoning:10},note:''})),complete:true,notice:'本机请求用量。',updatedAt:'2026-11-10T12:00:00+08:00'};
  }
  if(command==='wechat_action'){
    if(args.action==='preflight')return {ready:true,message:'Codex 会话组件可用'};
    if(args.action==='qr'){
      if(window.qrBehavior==='hold')return new Promise(resolve=>window.resolveQr=resolve);
      if(window.qrBehavior==='fail')throw '获取微信二维码超时，请检查网络后重试。';
      return {content:'https://example.test/qr-fixture'};
    }
    if(args.action==='pollQr'){if(window.wxPollMode==='wait')return {status:'wait'};window.wx.bound=true;return {status:'confirmed'};}
    if(args.action==='cancelQr')return {...window.wx};
    if(args.action==='chooseFolder'){
      if(window.folderBehavior==='hold')return new Promise(resolve=>window.resolveFolder=resolve);
      if(window.folderBehavior==='fail')throw '无法打开文件夹选择窗口';
      return {path:'D:\\\\Projects\\\\test'};
    }
    if(args.action==='start'){window.wx={...window.wx,...args.payload,running:true,phase:'等待微信消息'};}
    if(args.action==='stop')window.wx.running=false;
    if(args.action==='unbind'){window.wx.bound=false;window.wx.running=false;}
    return {...window.wx};
  }
  if(command==='fetch_codexpp_models')return {models:['z-model','default-model','a-model'],defaultModel:'default-model'};
  if(command==='configure_codexpp_provider')return '配置已完成';
  if(command==='reset_codex_initial_state'){
    if(window.failReset)throw '请先从托盘完全退出 Codex++';
    if(window.holdReset)await new Promise(resolve=>window.resolveReset=resolve);
    return {backupDirectories:['F:/fixture/backup/provider-test','F:/fixture/<not-html>']};
  }
  if(command==='codex_setup_state')return {onboardingComplete:true,fullAccessEnabled:true};
`;
(async () => {
  const sample = fs.readFileSync(path.join(__dirname, 'check-desktop-layout.cjs'), 'utf8');
  const begin = sample.indexOf('await page.addInitScript(() => {') + 'await page.addInitScript(() => {'.length;
  let bootstrap = sample.slice(begin, sample.indexOf('\n    });', begin));
  // Persistent fixture state belongs outside the per-command invoker.
  const split = injection.indexOf("  if(command===");
  const state = injection.slice(0, split);
  bootstrap = state + bootstrap.replace('window.calls.push({ command, args });', 'window.calls.push({ command, args });' + injection.slice(split));
  bootstrap = bootstrap.replace('transformCallback: () => 1,','transformCallback: callback => { const id=Object.keys(window.fixtureCallbacks).length+1; window.fixtureCallbacks[id]=callback; return id; },');
  server = http.createServer((req, res) => {
    const pathname = new URL(req.url, 'http://localhost').pathname;
    const file = path.resolve(root, 'dist', pathname === '/' ? 'index.html' : '.' + pathname);
    if (!file.startsWith(path.resolve(root, 'dist') + path.sep) || !fs.existsSync(file)) return res.writeHead(404).end();
    res.setHeader('Content-Type', ({ '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css' })[path.extname(file)] || 'application/octet-stream'); res.end(fs.readFileSync(file));
  });
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  const edge = [process.env['PROGRAMFILES(X86)'], process.env.PROGRAMFILES].map(p => p && path.join(p, 'Microsoft/Edge/Application/msedge.exe')).find(p => p && fs.existsSync(p));
  if (!edge) throw Error('Microsoft Edge is required for this UI check');
  browser = spawn(edge, ['--headless=new', '--disable-gpu', '--no-first-run', '--remote-debugging-port=0', '--user-data-dir=' + profile, 'about:blank'], { windowsHide: true, stdio: 'ignore' });
  const active = path.join(profile, 'DevToolsActivePort'); for (let i = 0; !fs.existsSync(active) && i < 150; i++) await sleep(100);
  let port;
  for(let attempt=0;attempt<100;attempt++){
    try { port=fs.readFileSync(active,'utf8').split('\n')[0];if(/^\d+$/.test(port))break; } catch {}
    await sleep(100);
  }
  if(!port)throw Error('Test browser debugging port did not become ready');
  const target = await fetch('http://127.0.0.1:' + port + '/json/new?about:blank', { method: 'PUT' }).then(r => r.json());
  ws = new WebSocket(target.webSocketDebuggerUrl); await new Promise(r => ws.addEventListener('open', r));
  ws.addEventListener('message', e => { const m = JSON.parse(e.data); if (m.method === 'Runtime.exceptionThrown') errors.push(m.params.exceptionDetails.text); if (pending.has(m.id)) { const p = pending.get(m.id); pending.delete(m.id); m.error ? p.reject(Error(JSON.stringify(m.error))) : p.resolve(m.result); } });
  await call('Page.enable'); await call('Runtime.enable'); await call('Page.addScriptToEvaluateOnNewDocument', { source: bootstrap });
  await call('Page.navigate', { url: 'http://127.0.0.1:' + server.address().port + '/' });
  await until("!!document.querySelector('.register-entry')");
  await evaluate("document.querySelector('.register-entry').click()");
  await until("document.querySelector('#register-captcha-image')?.src.startsWith('data:')");
  await snapshot('registration');
  await evaluate("document.querySelector('#register-email').value='test@example.test';document.querySelector('#register-captcha').value='ABCD';document.querySelector('#register-send-code').click()");
  await until("window.calls.some(c=>c.command==='account_portal'&&c.args.action==='sendCode')");
  await evaluate("document.querySelector('#register-email-code').value='123456';document.querySelector('#register-password').value='fixture-password';document.querySelector('#register-password-confirm').value='fixture-password';document.querySelector('#registration-form').requestSubmit()");
  await until("!!document.querySelector('#install-all')");
  assert.equal(await evaluate("window.calls.find(c=>c.command==='account_portal'&&c.args.action==='register').args.payload.passwordHash.length"),64);
  console.log('PASS: registration captcha, email code, password hash and automatic login');
  // Creation failure must leave the setup unstarted; retry must create the key and use it.
  await evaluate("window.failKey=true;document.querySelector('#install-all').click()");
  await until("document.querySelector('#toast').textContent.includes('账户服务暂不可用')");
  assert.equal(await evaluate("window.calls.some(c=>c.command==='configure_codexpp_provider')"),false);
  await evaluate("window.failKey=false;document.querySelector('#install-all').click()");
  await until("window.calls.some(c=>c.command==='configure_codexpp_provider')");
  await until("document.querySelector('#progress-panel').hidden");
  const configured = await evaluate("window.calls.find(c=>c.command==='configure_codexpp_provider').args");
  assert.equal(configured.apiKey,'fixture-account-key'); assert.equal(configured.model,'default-model'); assert.deepEqual(configured.models,['z-model','default-model','a-model']);
  console.log('PASS: one-click stops on key failure, retries creation, uses account key and preserves upstream model order/default');
  await evaluate("document.querySelector('.setup-options-disclosure').open=true");
  await until("document.querySelector('#codex-install-path')?.options.length===3");
  await evaluate("window.holdLocationSave=true;document.querySelector('#codex-install-path').selectedIndex=1;document.querySelector('#codex-install-path').dispatchEvent(new Event('change'))");
  await until("typeof window.resolveLocationSave==='function'");
  const installsBeforeSave=await evaluate("window.installLocationsUsed.length");
  await evaluate("document.querySelector('[data-product-action=reinstall][data-key=codex]').click()");
  await sleep(150);
  assert.equal(await evaluate("window.installLocationsUsed.length"),installsBeforeSave);
  await evaluate("window.resolveLocationSave();window.holdLocationSave=false");
  await until("window.installLocationsUsed.length>0&&document.querySelector('#progress-panel').hidden");
  assert.equal(await evaluate("window.installLocationsUsed.at(-1)"),'D:\\WindowsApps');
  await evaluate("document.querySelector('[data-page=config]').click();document.querySelector('[data-page=install]').click()");
  assert.equal(await evaluate("document.querySelector('#codex-install-path').value"),'D:\\WindowsApps');
  await evaluate("document.querySelector('.setup-options-disclosure').open=true;window.failLocationSave=true;document.querySelector('#codex-install-path').selectedIndex=2;document.querySelector('#codex-install-path').dispatchEvent(new Event('change'))");
  await until("document.querySelector('.install-location-status').textContent.includes('磁盘不可用')");
  assert.equal(await evaluate("document.querySelector('#codex-install-path').value"),'D:\\WindowsApps');
  await evaluate("window.failLocationSave=false;window.testInstalled.codex=false;document.querySelector('#install-all').click()");
  await until("window.installLocationsUsed.length>1&&document.querySelector('#progress-panel').hidden");
  assert.equal(await evaluate("window.installLocationsUsed.at(-1)"),'D:\\WindowsApps');
  await evaluate("document.querySelector('[data-product-action=store-install]').click()");
  await until("window.calls.some(c=>c.command==='install_product'&&c.args.product==='codex')&&document.querySelector('#progress-panel').hidden");
  assert.equal(await evaluate("window.installLocationsUsed.at(-1)"),'D:\\WindowsApps');
  await evaluate("document.querySelector('.setup-options-disclosure').open=true");
  await snapshot('codex-install-location');
  await evaluate("document.querySelector('#codex-install-path').selectedIndex=0;document.querySelector('#codex-install-path').dispatchEvent(new Event('change'))");
  await until("window.installLocationPath===''&&!document.querySelector('#codex-install-path').disabled");
  await evaluate("document.querySelector('.setup-options-disclosure').open=false");
  console.log('PASS: install location auto-save, save/install sequencing, navigation persistence, save failure rollback, batch/individual/Store entry paths and default reset');
  for(const theme of ['dark','light'])for(const width of [760,1120]){
    await call('Emulation.setDeviceMetricsOverride',{width,height:820,deviceScaleFactor:1,mobile:false});
    await evaluate(`document.documentElement.dataset.theme='${theme}';document.querySelector('[data-product-action=reset]').closest('.product-actions').querySelector('.more-button').click()`);
    assert.equal(await evaluate(`(()=>{const m=document.querySelector('[data-product-action=reset]').closest('.product-menu'),r=m.getBoundingClientRect();return !m.hidden&&r.width>=220&&r.left>=0&&r.right<=innerWidth&&[...m.querySelectorAll('button')].every(b=>getComputedStyle(b).whiteSpace==='nowrap'&&b.scrollWidth<=b.clientWidth)})()`),true);
    await snapshot(`product-menu-${theme}-${width}`);
    await evaluate("document.querySelector('[data-product-action=reset]').closest('.product-actions').querySelector('.more-button').click()");
  }
  await evaluate("window.failInstall=true;window.holdInstall=true;document.querySelector('[data-product-action=reinstall][data-key=codex]').click()");
  await until("typeof window.resolveInstall==='function'");
  assert.equal(await evaluate("document.querySelector('#progress-close').hidden"),false);
  await evaluate("document.querySelector('#progress-close').click()");
  assert.equal(await evaluate("getComputedStyle(document.querySelector('#progress-panel')).position"),'static');
  assert.equal(await evaluate("document.querySelector('#progress-percent').textContent"),'10%');
  await evaluate("window.downloadFixture.downloaded=5120;window.fixtureListeners['installer-download-progress']({payload:window.downloadFixture})");
  assert.equal(await evaluate("document.querySelector('#progress-percent').textContent"),'50%');
  await evaluate("document.querySelector('#download-pause-toggle').click()");
  await until("document.querySelector('#download-pause-toggle').textContent==='继续下载'");
  await evaluate("document.querySelector('#download-pause-toggle').click()");
  await until("document.querySelector('#download-pause-toggle').textContent==='暂停下载'");
  await evaluate("document.querySelector('#progress-panel').scrollIntoView({block:'end'})");
  await snapshot('download-progress-inline');
  await evaluate("document.querySelector('[data-page=config]').click()");
  await until("!!document.querySelector('#provider-new')");
  assert.equal(await evaluate("document.querySelector('#progress-panel').classList.contains('progress-inline')"),true);
  assert.equal(await evaluate("window.calls.filter(c=>c.command==='set_download_cancelled'&&c.args.cancelled).length"),0);
  await evaluate("document.querySelector('[data-page=install]').click();document.querySelector('#progress-close').click()");
  await evaluate("window.resolveInstall()");
  await until("document.querySelector('#progress-percent').textContent==='失败'");
  assert.equal(await evaluate("document.querySelector('#progress-detail').textContent.includes('PowerShell')"),false);
  assert.equal(await evaluate("document.querySelector('#progress-error-details').open"),false);
  await evaluate("document.querySelector('#progress-error-details').open=true");
  assert.equal(await evaluate("!!document.querySelector('#progress-error-details not-html')"),false);
  assert.equal(await evaluate("(()=>{const r=document.querySelector('#progress-close').getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight&&document.elementFromPoint(r.x+r.width/2,r.y+r.height/2).closest('#progress-close')!==null})()"),true);
  await snapshot('install-failure-details');
  await evaluate("document.querySelector('[data-page=config]').click()");
  await until("!!document.querySelector('#provider-new')");
  assert.equal(await evaluate("document.querySelector('#provider-new').disabled"),false);
  await evaluate("document.querySelector('#progress-close').click()");
  assert.equal(await evaluate("document.querySelector('#progress-panel').hidden"),false);
  assert.equal(await evaluate("(()=>{const p=document.querySelector('#progress-panel');return getComputedStyle(p).position==='static'&&p.getBoundingClientRect().top>=document.querySelector('#page-content').getBoundingClientRect().bottom})()"),true);
  await snapshot('install-failure-inline');
  await evaluate("window.failInstall=false;window.holdInstall=false;document.querySelector('[data-page=install]').click();document.querySelector('[data-product-action=reinstall][data-key=codex]').click()");
  await until("window.calls.filter(c=>c.command==='install_codex_windows_fallback').length>=2&&document.querySelector('#progress-panel').hidden");
  console.log('PASS: menu fits both themes; active/failed progress docks below content without cancelling, survives navigation, floats again and permits retry');
  // Reset is mocked: real local credentials, history, processes and configurations are never used.
  await evaluate("document.querySelector('[data-product-action=reset]').closest('.product-actions').querySelector('.more-button').click();document.querySelector('[data-product-action=reset]').click()");
  await until("!!document.querySelector('#codex-reset-dialog[open]')");
  assert.equal(await evaluate("document.querySelector('#codex-reset-dialog').textContent.includes('保留以前的会话记录')"),true);
  assert.equal(await evaluate("document.querySelector('[data-product-action=reset]').closest('.product-menu').hidden"),true);
  await evaluate("document.querySelector('#codex-reset-cancel').click()");
  await until("!document.querySelector('#codex-reset-dialog')");
  assert.equal(await evaluate("window.calls.filter(c=>c.command==='reset_codex_initial_state').length"),0);
  await evaluate("window.failReset=true;document.querySelector('[data-product-action=reset]').click()");
  await until("!!document.querySelector('#codex-reset-confirm')");
  await evaluate("document.querySelector('#codex-reset-confirm').click()");
  await until("document.querySelector('#codex-reset-feedback')?.textContent.includes('退出 Codex++')");
  assert.equal(await evaluate("document.querySelector('#codex-reset-confirm').textContent"),'重试');
  await evaluate("window.failReset=false;window.holdReset=true;document.querySelector('#codex-reset-confirm').click();document.querySelector('#codex-reset-confirm').click()");
  await until("typeof window.resolveReset==='function'");
  assert.equal(await evaluate("window.calls.filter(c=>c.command==='reset_codex_initial_state').length"),2);
  assert.equal(await evaluate("document.querySelector('#codex-reset-cancel').disabled"),true);
  await evaluate("window.resolveReset()");
  await until("document.querySelector('#codex-reset-feedback')?.textContent.includes('恢复完成')");
  await until("document.querySelectorAll('.codex-reset-path').length===2");
  assert.deepEqual(await evaluate("[...document.querySelectorAll('.codex-reset-path')].map(p=>p.textContent)"),['F:/fixture/backup/provider-test','F:/fixture/<not-html>']);
  assert.equal(await evaluate("!!document.querySelector('#codex-reset-backups not-html')"),false);
  assert.equal(await evaluate("document.querySelector('#codex-reset-cancel').hidden"),true);
  await snapshot('reset-complete');
  await evaluate("document.querySelector('#codex-reset-confirm').click()");
  await until("!document.querySelector('#codex-reset-dialog')");
  assert.equal(await evaluate("document.querySelector('#install-all').disabled"),false);
  console.log('PASS: reset cancellation, error/retry, duplicate-click protection, preserved-history notice and safe backup-path display');
  await evaluate("document.querySelector('[data-page=account]').click()");
  await until("!!document.querySelector('#account-web-center')&&!document.querySelector('#account-add-key')");
  const keyCalls=await evaluate("window.calls.filter(c=>c.command==='account_portal'&&c.args.action==='ensureKey').length");
  await evaluate("document.querySelector('[data-page=install]').click();document.querySelector('#install-all').click()");
  await until("document.querySelector('#progress-panel').hidden");
  assert.equal(await evaluate("window.calls.filter(c=>c.command==='account_portal'&&c.args.action==='ensureKey').length"),keyCalls);
  await evaluate("document.querySelector('[data-page=account]').click();document.querySelector('#account-usage-tab').click()");
  for (const period of ['today','7d','30d','all']) {
    await evaluate(`document.querySelector('[data-usage-period="${period}"]').click()`);
    await until("!!document.querySelector('.usage-metrics')");
    assert.equal(await evaluate(`window.calls.some(c=>c.command==='codex_usage'&&c.args.period==='${period}')`),true);
  }
  console.log('PASS: existing key reuse, account navigation and four usage periods');
  await snapshot('usage');
  await evaluate("document.querySelector('[data-usage-section=logs]').click()");
  await until("document.querySelectorAll('[data-usage-log-row]').length===20");
  assert.equal(await evaluate("!!document.querySelector('.usage-metrics')"),false);
  assert.equal(await evaluate("document.querySelector('#usage-log-prev').disabled"),true);
  await evaluate("document.querySelector('[data-usage-log-row] details').open=true");
  assert.equal(await evaluate("document.querySelector('[data-usage-log-row]').textContent.includes('fixture-request-1')"),true);
  await snapshot('usage-logs');
  await evaluate("document.querySelector('#usage-log-next').click()");
  await until("document.querySelector('.usage-pagination')?.textContent.includes('第 2 / 3 页')");
  assert.equal(await evaluate("document.querySelectorAll('[data-usage-log-row]').length"),20);
  await evaluate("document.querySelector('#usage-log-next').click()");
  await until("document.querySelectorAll('[data-usage-log-row]').length===5");
  assert.equal(await evaluate("document.querySelector('#usage-log-next').disabled"),true);
  await evaluate("document.querySelector('#usage-log-prev').click()");
  await until("document.querySelector('.usage-pagination')?.textContent.includes('第 2 / 3 页')");
  await evaluate("document.querySelector('[data-usage-period=today]').click()");
  await until("document.querySelectorAll('[data-usage-log-row]').length===3");
  assert.equal(await evaluate("document.querySelector('.usage-pagination').textContent.includes('第 1 / 1 页')"),true);
  console.log('PASS: separate usage views; 45 logs paginate 20/20/5; details, previous page and filter reset');
  assert.equal(await evaluate("document.querySelector('[data-page=skills]')"),null);
  assert.equal(await evaluate("document.querySelector('[data-page=plugins] span').textContent.trim()"),'技能与插件');
  assert.equal(await evaluate("document.querySelector('#library-submenu').hidden"),true);
  await evaluate("document.querySelector('[data-page=plugins]').click()");
  assert.equal(await evaluate("document.querySelector('#library-submenu').hidden"),false);
  assert.deepEqual(await evaluate("[...document.querySelectorAll('#library-submenu button')].map(b=>b.textContent)"),['技能库','插件库']);
  assert.equal(await evaluate("!!document.querySelector('.usage-pagination')"),true,'Expanding navigation preserves current page');
  await evaluate("document.querySelector('[data-library-tab=plugins]').click()");
  assert.equal(await evaluate("document.querySelector('.library-tabs')"),null);
  await until("document.querySelectorAll('.public-plugin-card').length===48");
  assert.equal(await evaluate("document.querySelector('h1').textContent"),'插件市场');
  assert.equal(await evaluate("document.querySelector('[data-public-filter=all]').textContent"),'全部插件 520');
  assert.equal(await evaluate("document.querySelector('[data-public-filter=auth]').textContent"),'需服务授权 104');
  assert.equal(await evaluate("Array.from(document.querySelectorAll('.public-plugin-description')).every(p=>/[\\u4e00-\\u9fff]/.test(p.textContent))"),true);
  assert.equal(await evaluate("document.querySelectorAll('.market-card [data-plugin-source=user]').length"),0,'Public marketplace excludes paid community shares');
  await snapshot('public-plugin-market');
  await evaluate("document.querySelector('[data-public-filter=auth]').click()");
  assert.equal(await evaluate("document.querySelector('#public-pagination').textContent.includes('共 104 个')"),true);
  await evaluate("document.querySelector('[data-public-filter=all]').click();document.querySelector('#public-next').click()");
  assert.equal(await evaluate("document.querySelector('#public-pagination').textContent.includes('第 2 / 11 页')"),true);
  await evaluate("document.querySelector('#public-search').value='文献';document.querySelector('#public-search-form').requestSubmit()");
  assert.equal(await evaluate("document.querySelector('#public-results').textContent.includes('Zotero')"),true);
  await evaluate("document.querySelector('#public-search').value='01 Superdesign';document.querySelector('#public-search-form').requestSubmit();window.failPublicInstall=true;document.querySelector('[data-public-install]').click()");
  await until("document.querySelector('.public-plugin-notice')?.textContent.includes('下载中断')");
  await evaluate("window.failPublicInstall=false;window.holdPublicInstall=true;document.querySelector('[data-public-install]').click()");
  await until("!!window.resolvePublicInstall");
  assert.equal(await evaluate("document.querySelector('[data-public-install]').disabled"),true);
  await evaluate("document.querySelector('[data-plugin-section=community]').click()");
  await until("!!document.querySelector('#market-search')");
  await evaluate("window.resolvePublicInstall();window.holdPublicInstall=false;document.querySelector('[data-plugin-section=market]').click()");
  await until("document.querySelector('[data-public-install]')?.textContent==='已安装'");
  assert.equal(await evaluate("document.querySelector('[data-public-filter=installed]').textContent"),'已安装 1');
  const publicInstallArgs=await evaluate("window.calls.filter(c=>c.command==='public_plugin_market'&&c.args.action==='install').at(-1).args");
  assert.equal(publicInstallArgs.payload.tree,publicPlugins[0].tree);
  assert.equal('apiKey' in publicInstallArgs,false,'Public install never attaches account credentials');
  await evaluate("window.publicPlugins[0].tree='1111111111111111111111111111111111111111';document.querySelector('#public-refresh').click()");
  await until("document.querySelector('[data-public-filter=updates]')?.textContent==='有更新 1'");
  assert.equal(await evaluate("document.querySelector('[data-public-install]').textContent"),'更新');
  await evaluate("document.querySelector('[data-public-install]').click()");
  await until("document.querySelector('[data-public-filter=updates]')?.textContent==='有更新 0'");
  await evaluate("document.querySelector('[data-public-detail]').click()");
  assert.equal(await evaluate("document.querySelector('.public-plugin-dialog').textContent.includes('下载插件不会扣除坚果点数')"),true);
  await evaluate("document.querySelector('#public-remove').click()");
  await until("!!document.querySelector('[data-confirm-accept]')");
  await evaluate("document.querySelector('[data-confirm-accept]').click()");
  await until("document.querySelector('[data-public-filter=installed]')?.textContent==='已安装 0'");
  for (const theme of ['light','dark']) for (const width of [390,760,1120]) {
    await call('Emulation.setDeviceMetricsOverride',{width,height:820,deviceScaleFactor:1,mobile:false});
    await evaluate(`document.documentElement.dataset.theme='${theme}';document.querySelector('#public-search').value='';document.querySelector('#public-search-form').requestSubmit()`);
    assert.equal(await evaluate("document.documentElement.scrollWidth<=innerWidth+1"),true,'Public marketplace does not overflow');
    await snapshot(`public-plugin-market-${theme}-${width}`);
  }
  await call('Emulation.setDeviceMetricsOverride',{width:1120,height:820,deviceScaleFactor:1,mobile:false});
  await evaluate("document.querySelector('[data-plugin-section=community]').click()");
  console.log('PASS: 520 Chinese public plugins, 104 authorization filters, pagination/search, retry/background install, update/uninstall, source isolation, light/dark responsive layouts');
  await until("document.querySelector('#image-plugin-install')?.textContent==='安装到 Codex'");
  assert.equal(await evaluate("document.querySelector('.builtin-plugin-title h2').textContent"),'文生图');
  assert.equal(await evaluate("document.querySelector('.builtin-plugin-dialog')"),null);
  assert.equal(await evaluate("!!document.querySelector('.plugin-usage')"),false,'Usage details start collapsed');
  await evaluate("document.querySelector('.plugin-help').click()");
  assert.equal(await evaluate("document.querySelector('.plugin-usage').textContent.includes('聊天框中输入 @，选择“文生图”')"),true);
  await evaluate("document.querySelector('[data-plugin-help-close]').click()");
  await snapshot('image-plugin-before-install');
  await evaluate("window.failPlugin=true;document.querySelector('#image-plugin-install').click()");
  await until("document.querySelector('#image-plugin-feedback')?.textContent.includes('无法写入')");
  assert.equal(await evaluate("document.querySelector('#image-plugin-install').disabled"),false);
  await evaluate("window.failPlugin=false;window.holdPlugin=true;document.querySelector('#image-plugin-install').click()");
  await until("!!window.resolvePluginInstall");
  assert.equal(await evaluate("document.querySelector('#image-plugin-install').disabled"),true);
  const pluginArgs=await evaluate("window.calls.find(c=>c.command==='install_image_plugin').args");
  assert.equal(pluginArgs.apiKey,'fixture-account-key');assert.equal(pluginArgs.baseUrl,'https://api.jianguo.ltd');
  await evaluate("document.querySelector('[data-library-tab=skills]').click()");
  await until("!!document.querySelector('#skill-search')");
  assert.equal(await evaluate("document.querySelector('h1').textContent"),'技能库');
  await evaluate("window.resolvePluginInstall();window.holdPlugin=false;document.querySelector('[data-library-tab=plugins]').click()");
  await until("document.querySelector('#image-plugin-badge')?.textContent==='已安装'");
  assert.equal(await evaluate("document.querySelector('#image-plugin-install').textContent"),'更新插件');
  assert.equal(await evaluate("document.querySelector('.builtin-plugin').textContent.includes('fixture-account-key')"),false);
  console.log('PASS: unified 技能库 with skill/plugin tabs; image plugin credentials and background install survive tab changes');
  await until("document.querySelector('#vector-plugin-install')?.textContent==='安装到 Codex'");
  assert.equal(await evaluate("document.querySelectorAll('.builtin-plugin').length"),2);
  assert.equal(await evaluate("document.querySelectorAll('.builtin-plugin-title h2')[1].textContent"),'位图转矢量图');
  await evaluate("window.failVectorPlugin=true;document.querySelector('#vector-plugin-install').click()");
  await until("document.querySelector('#vector-plugin-feedback')?.textContent.includes('无法写入')");
  assert.equal(await evaluate("document.querySelector('#vector-plugin-install').disabled"),false);
  await evaluate("window.failVectorPlugin=false;document.querySelector('#vector-plugin-install').click()");
  await until("document.querySelector('#vector-plugin-badge')?.textContent==='已安装'");
  const vectorArgs=await evaluate("window.calls.find(c=>c.command==='install_vector_plugin').args");
  assert.equal(vectorArgs.apiKey,'fixture-account-key');assert.equal(vectorArgs.baseUrl,'https://api.jianguo.ltd');
  assert.equal(await evaluate("document.querySelector('#vector-plugin-install').textContent"),'更新插件');
  assert.equal(await evaluate("document.querySelector('#image-plugin-badge').textContent"),'已安装');
  assert.equal(await evaluate("document.querySelector('#page-content').textContent.includes('fixture-account-key')"),false);
  console.log('PASS: 坚果位图转矢量图 uses current account Key, retries failed installation and coexists with image plugin');
  await evaluate("document.querySelector('#image-plugin-install').click()");
  await until("document.querySelector('#image-plugin-feedback')?.textContent.includes('已更新')");
  assert.equal(await evaluate("window.calls.filter(c=>c.command==='install_image_plugin').length>=3"),true);
  for(const id of ['image','vector']){
    assert.equal(await evaluate(`document.querySelector('#${id}-plugin-uninstall').hidden`),false);
    const before=await evaluate("window.calls.filter(c=>c.command==='uninstall_builtin_plugin').length");
    await evaluate(`window.confirm=()=>false;document.querySelector('#${id}-plugin-uninstall').click()`);
    assert.equal(await evaluate("window.calls.filter(c=>c.command==='uninstall_builtin_plugin').length"),before);
    await evaluate(`window.confirm=()=>true;window.failUninstall=true;document.querySelector('#${id}-plugin-uninstall').click()`);
    await until(`document.querySelector('#${id}-plugin-feedback')?.textContent.includes('无法卸载')`);
    assert.equal(await evaluate(`document.querySelector('#${id}-plugin-uninstall').disabled`),false);
    await evaluate(`window.failUninstall=false;document.querySelector('#${id}-plugin-uninstall').click()`);
    await until(`document.querySelector('#${id}-plugin-badge')?.textContent==='未安装'`);
    assert.equal(await evaluate(`document.querySelector('#${id}-plugin-uninstall').hidden`),true);
    assert.equal(await evaluate(`document.querySelector('#${id==='image'?'vector':'image'}-plugin-badge').textContent`),'已安装');
    await evaluate(`document.querySelector('#${id}-plugin-install').click()`);
    await until(`document.querySelector('#${id}-plugin-badge')?.textContent==='已安装'`);
  }
  console.log('PASS: compact renamed cards, collapsed instructions, plugin update, uninstall cancel/error/retry, independent removal and reinstall');
  assert.equal(await evaluate("document.querySelector('.content').textContent.includes('非分享')"),false);
  assert.equal(await evaluate("document.querySelector('.content').textContent.includes('使用当前登录的坚果 API Key')"),false);
  for(const id of ['image','vector']) assert.equal(await evaluate(`document.querySelector('#${id}-plugin-badge').parentElement.classList.contains('builtin-plugin-title')`),true);
  assert.equal(await evaluate("document.querySelector('[data-library-tab=shares]')"),null);
  assert.equal(await evaluate("document.querySelector('#market-share').textContent"),'我要分享');
  await until("!!document.querySelector('[data-market-detail]')");
  assert.equal(await evaluate("document.querySelector('.market-card [data-plugin-source]').textContent"),'用户分享');
  assert.equal(await evaluate("document.querySelector('.market-card p img')"),null,'Descriptions are escaped, never rendered as HTML');
  const lists=await evaluate("window.calls.filter(c=>c.command==='plugin_market'&&c.args.action==='list').length");
  await evaluate("document.querySelector('#market-search').value='社区';document.querySelector('#market-search').dispatchEvent(new Event('input',{bubbles:true}))");
  assert.equal(await evaluate("window.calls.filter(c=>c.command==='plugin_market'&&c.args.action==='list').length"),lists,'Typing does not trigger a request');
  await evaluate("document.querySelector('#market-search-form').requestSubmit()");
  await until("window.calls.filter(c=>c.command==='plugin_market'&&c.args.action==='list').length>"+lists);
  await evaluate("document.querySelector('[data-market-detail]').click()");
  await until("!!document.querySelector('#market-install')");
  assert.equal(await evaluate("document.querySelector('.market-service-disclosure').textContent.includes('申请示例 API Key <script>只是说明</script>')"),true);
  assert.equal(await evaluate("document.querySelector('.market-service-disclosure script')"),null);
  assert.equal(await evaluate("document.querySelector('.market-service-disclosure').textContent.includes('服务商按请求计费')"),true);
  await snapshot('community-plugin-detail');
  await evaluate("window.confirm=()=>false;document.querySelector('#market-install').click()");
  assert.equal(await evaluate("window.calls.filter(c=>c.command==='plugin_market'&&c.args.action==='install').length"),0);
  await evaluate("window.confirm=()=>true;window.failMarketInstall=true;document.querySelector('#market-install').click()");
  await until("document.querySelector('#market-dialog-feedback')?.textContent.includes('余额不足')");
  assert.equal(await evaluate("document.querySelector('#market-install').disabled"),false);
  await evaluate("window.failMarketInstall=false;document.querySelector('#market-install').click()");
  await until("document.querySelector('#market-dialog-feedback')?.textContent.includes('安装完成')");
  assert.equal(await evaluate("window.calls.find(c=>c.command==='plugin_market'&&c.args.action==='install').args.payload.expectedPrice"),10);
  await evaluate("document.querySelector('#market-remove').click()");
  await until("document.querySelector('#market-dialog-feedback')?.textContent.includes('已卸载')");
  await evaluate("document.querySelector('[data-close]').click();document.querySelector('#market-share').click()");
  await until("document.querySelector('#market-mine-summary')?.textContent.includes('30 点')");
  assert.equal(await evaluate("document.querySelector('[data-library-tab=plugins]').getAttribute('aria-current')"),'page');
  assert.equal(await evaluate("document.querySelector('#market-back').textContent"),'返回插件库');
  assert.equal(await evaluate("document.querySelector('.market-share-form').open"),true);
  await evaluate("document.querySelector('.market-share-form').open=true");
  assert.equal(await evaluate("document.querySelector('#share-requires-authorization').checkValidity()"),false);
  await evaluate("var select=document.querySelector('#share-requires-authorization');select.value='yes';select.dispatchEvent(new Event('change',{bubbles:true}))");
  assert.equal(await evaluate("document.querySelector('[name=authorizationDetails]').required&&!document.querySelector('[name=authorizationDetails]').checkValidity()"),true);
  await evaluate("var select=document.querySelector('#share-requires-authorization');select.value='no';select.dispatchEvent(new Event('change',{bubbles:true}))");
  assert.equal(await evaluate("document.querySelector('#share-authorization-fields').hidden&&document.querySelector('[name=authorizationDetails]').disabled"),true);
  assert.equal(await evaluate("document.querySelector('[name=billingDetails]').checkValidity()"),false);
  await evaluate(`{ document.querySelector('.market-share-form').open=true;const f=document.querySelector('#plugin-share-form');for(const [name,value] of Object.entries({name:'投稿测试',author:'测试作者',description:'测试介绍',instructions:'输入 @ 使用',price:'10',requiresAuthorization:'yes',authorizationDetails:'通过示例平台注册 API Key',billingDetails:'示例服务按次计费'})){f.elements.namedItem(name).value=value;f.elements.namedItem(name).dispatchEvent(new Event('input',{bubbles:true}));}f.elements.namedItem('requiresAuthorization').dispatchEvent(new Event('change',{bubbles:true}));const transfer=new DataTransfer();transfer.items.add(new File(['fixture-package'],'example.zip',{type:'application/zip'}));document.querySelector('#share-archive').files=transfer.files;document.querySelector('#share-archive').dispatchEvent(new Event('change',{bubbles:true}));document.querySelector('#share-ownership').checked=true;window.failMarketUpload=true;f.requestSubmit(); }`);
  await until("document.querySelector('#share-feedback')?.textContent.includes('保存失败')");
  await snapshot('community-plugin-upload');
  const beforeInvalidImage=await evaluate("window.calls.filter(c=>c.command==='plugin_market'&&c.args.action==='upload').length");
  await evaluate("{ const dt=new DataTransfer();dt.items.add(new File(['broken-image'],'bad.png',{type:'image/png'}));document.querySelector('#share-screenshots').files=dt.files;document.querySelector('#share-screenshots').dispatchEvent(new Event('change')); }");
  await until("document.querySelector('#share-image-feedback').textContent.includes('无法读取')");
  await evaluate("document.querySelector('#plugin-share-form').requestSubmit()");
  assert.equal(await evaluate("window.calls.filter(c=>c.command==='plugin_market'&&c.args.action==='upload').length"),beforeInvalidImage,'Invalid images must block submission, not silently disappear');
  await evaluate("(async()=>{ const c=document.createElement('canvas');c.width=1200;c.height=1000;const g=c.getContext('2d'),d=g.createImageData(c.width,c.height),v=new Uint32Array(d.data.buffer);let seed=12345;for(let i=0;i<v.length;i++){seed^=seed<<13;seed^=seed>>>17;seed^=seed<<5;v[i]=(seed&0xffffff)|0xff000000;}g.putImageData(d,0,0);const blob=await new Promise(r=>c.toBlob(r,'image/png'));window.originalScreenshotSize=blob.size;const dt=new DataTransfer();dt.items.add(new File([blob],'large-introduction.png',{type:'image/png'}));document.querySelector('#share-screenshots').files=dt.files;document.querySelector('#share-screenshots').dispatchEvent(new Event('change')); })()");
  await until("document.querySelector('#share-image-preview img')?.complete&&!document.querySelector('#share-submit').disabled");
  assert.ok(await evaluate("window.originalScreenshotSize")>1024*1024,'Fixture exceeds old upload limit');
  assert.equal(await evaluate("document.querySelector('#share-image-feedback').textContent.includes('第一张将用作卡片封面')"),true);
  await snapshot('community-plugin-image-preview');
  assert.equal(await evaluate("document.querySelector('#plugin-share-form').elements.namedItem('name').value"),'投稿测试');
  await evaluate("document.querySelector('#library-toggle').click()");
  assert.equal(await evaluate("document.querySelector('#library-submenu').hidden"),true);
  assert.equal(await evaluate("document.querySelector('#plugin-share-form').elements.namedItem('billingDetails').value"),'示例服务按次计费');
  await evaluate("document.querySelector('#library-toggle').click();document.querySelector('#market-back').click();document.querySelector('#market-share').click()");
  assert.equal(await evaluate("document.querySelector('#plugin-share-form').elements.namedItem('authorizationDetails').value"),'通过示例平台注册 API Key');
  await evaluate("document.querySelector('.market-share-form').open=true;document.querySelector('#share-ownership').checked=true;window.failMarketUpload=false;document.querySelector('#plugin-share-form').requestSubmit()");
  await until("document.querySelector('#share-feedback')?.textContent.includes('提交成功')");
  await until("document.querySelector('.market-card')?.textContent.includes('待审核')");
  const uploaded=await evaluate("window.calls.find(c=>c.command==='plugin_market'&&c.args.action==='upload').args.payload");
  assert.equal(uploaded.requiresAuthorization,true);assert.equal(uploaded.authorizationDetails,'通过示例平台注册 API Key');assert.equal(uploaded.billingDetails,'示例服务按次计费');
  const savedUpload=await evaluate("window.calls.filter(c=>c.command==='plugin_market'&&c.args.action==='upload').at(-1).args.payload");
  assert.equal(savedUpload.screenshots.length,1);assert.match(savedUpload.screenshots[0],/^data:image\/jpeg;base64,/);
  assert.ok(Buffer.from(savedUpload.screenshots[0].split(',')[1],'base64').length<=1024*1024);
  await until("document.querySelector('.market-card-cover img')?.complete&&document.querySelector('.market-card-cover img')?.naturalWidth>0");
  assert.equal(await evaluate("document.querySelector('#share-requires-authorization').value"),'');
  assert.equal(await evaluate("document.querySelector('#share-authorization-fields').hidden"),true);
  assert.equal(await evaluate("document.querySelector('.market-card [data-plugin-source]').textContent"),'用户分享');
  await evaluate("document.querySelector('[data-library-tab=plugins]').click()");
  await until("document.querySelectorAll('.market-card').length===1");
  assert.equal(await evaluate("document.querySelector('#market-shares').textContent.includes('投稿测试')"),false);
  await evaluate("window.marketApproved=true;document.querySelector('#market-search').value='';document.querySelector('#market-search').dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('#market-search-form').requestSubmit()");
  await until("document.querySelectorAll('.market-card').length===2");
  await snapshot('community-plugin-market');
  await evaluate("window.marketMineItem.downloads=17;document.querySelector('#market-share').click()");
  await until("!!document.querySelector('[data-market-edit]')");
  await evaluate("window.failMarketEdit=true;document.querySelector('[data-market-edit]').click()");
  await until("document.querySelector('#market-feedback').textContent.includes('无法加载')");
  assert.equal(await evaluate("document.querySelector('[data-market-edit]').disabled"),false);
  await evaluate("window.failMarketEdit=false;document.querySelector('[data-market-edit]').click()");
  await until("!!document.querySelector('#share-edit-cancel')");
  assert.equal(await evaluate("document.querySelector('[name=name]').value"),'投稿测试');
  assert.equal(await evaluate("document.querySelector('[name=authorizationDetails]').value"),'通过示例平台注册 API Key');
  await evaluate("document.querySelector('[name=description]').value='编辑后的简介';document.querySelector('[name=description]').dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('#share-ownership').checked=true;window.failMarketUpload=true;document.querySelector('#plugin-share-form').requestSubmit()");
  await until("document.querySelector('#share-feedback').textContent.includes('保存失败')");
  assert.equal(await evaluate("document.querySelector('[name=description]').value"),'编辑后的简介');
  await snapshot('community-plugin-edit');
  await evaluate("window.failMarketUpload=false;document.querySelector('#plugin-share-form').requestSubmit()");
  await until("document.querySelector('#share-feedback').textContent.includes('修改已提交')");
  await until("document.querySelector('.market-card')?.textContent.includes('待审核')");
  const edited=await evaluate("window.calls.filter(c=>c.command==='plugin_market'&&c.args.action==='upload').at(-1).args.payload");
  assert.equal(edited.id,10);assert.equal(edited.description,'编辑后的简介');
  assert.equal('archiveBase64' in edited,false);assert.equal('screenshots' in edited,false);
  assert.equal(await evaluate("document.querySelector('.market-download-count').textContent.trim().replace(/\\s+/g,' ')"),'↓ 17 次');
  await evaluate("window.confirm=()=>false;document.querySelector('[data-market-end]').click()");
  assert.equal(await evaluate("window.calls.some(c=>c.command==='plugin_market'&&c.args.action==='end')"),false);
  await evaluate("window.confirm=()=>true;window.failMarketEnd=true;document.querySelector('[data-market-end]').click()");
  await until("document.querySelector('#market-feedback').textContent.includes('结束分享失败')");
  assert.equal(await evaluate("document.querySelector('[data-market-end]').disabled"),false);
  await evaluate("window.failMarketEnd=false;document.querySelector('[data-market-end]').click()");
  await until("document.querySelector('.market-card')?.textContent.includes('已下架')");
  assert.equal(await evaluate("!!document.querySelector('[data-market-end]')"),false);
  assert.equal(await evaluate("document.querySelector('.market-download-count').textContent.includes('17 次')"),true);
  await evaluate("document.querySelector('#market-back').click()");
  await until("document.querySelectorAll('.market-card').length===1");
  await evaluate("document.querySelector('#market-share').click()");
  await until("!!document.querySelector('[data-market-edit]')");
  await evaluate("document.querySelector('[data-market-edit]').click()");
  await until("!!document.querySelector('#share-edit-cancel')");
  await evaluate("document.querySelector('#share-ownership').checked=true;document.querySelector('#plugin-share-form').requestSubmit()");
  await until("document.querySelector('#share-feedback').textContent.includes('修改已提交')");
  await evaluate("window.marketApproved=true;window.marketInstalled=[{id:9,slug:'community-example',version:'1.0.0'}];document.querySelector('#market-back').click()");
  await until("document.querySelectorAll('.market-card').length===2");
  assert.equal(await evaluate("document.querySelector('[data-market-detail]').textContent"),'已安装 · 查看');
  console.log('PASS: edit failures/retry, same-id resubmission without ZIP, download counters, end-share confirmation/failure/retry, and resubmitting ended shares');
  await evaluate("window.failMarketList=true;document.querySelector('#market-refresh').click()");
  await until("document.querySelector('#market-feedback')?.textContent.includes('尚未开放')");
  assert.equal(await evaluate("document.querySelectorAll('.builtin-plugin').length"),2,'Offline market preserves built-in plugins');
  assert.equal(await evaluate("document.querySelector('#image-plugin-install').disabled"),false);
  await evaluate("window.failMarketList=false;document.querySelector('#market-refresh').click()");
  await until("!document.querySelector('#market-feedback').textContent");
  console.log('PASS: source badges, safe descriptions, Enter-only search, paid-install confirmation/failure recovery, uninstall, upload retry and pending/public separation');
  await evaluate("document.querySelector('[data-library-tab=skills]').click()");
  await until("!!document.querySelector('#skills-share')&&!document.querySelector('.skill-loading')");
  assert.equal(await evaluate("document.querySelector('#library-body').textContent.includes('汇集 Codex++')"),false);
  const publicCount=await evaluate("document.querySelectorAll('[data-install-skill]').length");
  assert.ok(publicCount>0,'Original public catalog preserved');
  await evaluate("document.querySelector('[data-skill-source=community]').click()");
  await until("!!document.querySelector('#market-share')");
  assert.equal(await evaluate("document.querySelectorAll('.builtin-plugin,.skill-group,.market-card').length"),0);
  await evaluate("document.querySelector('#skills-share').click()");
  await until("!!document.querySelector('#plugin-share-form')");
  assert.equal(await evaluate("document.querySelector('.market-share-form summary').textContent"),'分享我的技能');
  assert.equal(await evaluate("document.querySelector('#share-archive-name').textContent.includes('SKILL.md')"),true);
  await evaluate(`{ const f=document.querySelector('#plugin-share-form');for(const [name,value] of Object.entries({name:'分享技能示例',author:'技能作者',description:'用户创建的技能',instructions:'在聊天中选用此技能',price:'15',requiresAuthorization:'no',billingDetails:'无需额外费用'})){f.elements.namedItem(name).value=value;f.elements.namedItem(name).dispatchEvent(new Event('input',{bubbles:true}));}f.elements.namedItem('requiresAuthorization').dispatchEvent(new Event('change',{bubbles:true}));const dt=new DataTransfer();dt.items.add(new File(['fixture-skill'],'my-skill.zip',{type:'application/zip'}));document.querySelector('#share-archive').files=dt.files;document.querySelector('#share-archive').dispatchEvent(new Event('change'));document.querySelector('#share-ownership').checked=true;f.requestSubmit(); }`);
  await until("document.querySelector('#share-feedback')?.textContent.includes('提交成功')");
  assert.equal(await evaluate("window.calls.filter(c=>c.command==='plugin_market'&&c.args.action==='upload').at(-1).args.payload.kind"),'skill');
  await evaluate("document.querySelector('#market-back').click()");
  await until("!!document.querySelector('#market-search')");
  assert.equal(await evaluate("document.querySelectorAll('.market-card').length"),0,'Pending skills stay private');
  await evaluate("window.sharedSkill.status='published';document.querySelector('#market-refresh').click()");
  await until("document.querySelectorAll('.market-card').length===1");
  await snapshot('shared-skills');
  await evaluate("document.querySelector('[data-market-detail]').click()");
  await until("!!document.querySelector('#market-install')");
  await evaluate("window.confirm=()=>true;document.querySelector('#market-install').click()");
  await until("document.querySelector('#market-dialog-feedback')?.textContent.includes('安装完成')");
  assert.equal(await evaluate("window.calls.filter(c=>c.command==='plugin_market'&&c.args.action==='install').at(-1).args.payload.expectedPrice"),15);
  await evaluate("document.querySelector('[data-close]').click();document.querySelector('[data-skill-source=public]').click()");
  await until("!!document.querySelector('[data-install-skill]')");
  assert.equal(await evaluate("document.querySelectorAll('[data-install-skill]').length"),publicCount);
  assert.equal(await evaluate("!!document.querySelector('.market-card')"),false);
  await evaluate("document.querySelector('[data-library-tab=plugins]').click()");
  await until("document.querySelectorAll('.market-card').length===2");
  assert.equal(await evaluate("document.querySelector('#market-shares').textContent.includes('分享技能示例')"),false);
  console.log('PASS: public catalog intact and separate from shared skills; independent share forms, pending/public separation and priced skill installation');
  for (const theme of ['dark','light']) {
    await evaluate(`document.documentElement.dataset.theme='${theme}'`);
    for (const width of [390,760,1120]) {
      await call('Emulation.setDeviceMetricsOverride',{width,height:820,deviceScaleFactor:1,mobile:false});
      for(const page of ['account','plugins','wechat']) {
        await evaluate(page==='plugins'?"if(document.querySelector('#library-submenu').hidden)document.querySelector('#library-toggle').click();document.querySelector('[data-library-tab=plugins]').click()":`document.querySelector('[data-page=${page}]').click()`);
        if(page==='account'){
          await until("!!document.querySelector('#account-usage-tab')");
          await evaluate("document.querySelector('#account-usage-tab').click();document.querySelector('[data-usage-period=all]').click()");
          await until("document.querySelectorAll('[data-usage-log-row]').length===20");
          await evaluate("document.querySelector('[data-usage-log-row] details').open=true");
        }
        await sleep(100);
        assert.equal(await evaluate("document.documentElement.scrollWidth>innerWidth+1 || document.querySelector('.content').scrollWidth>document.querySelector('.content').clientWidth+1"),false,`${theme} ${width} ${page} overflow`);
        if(page==='account')await snapshot(`usage-logs-${theme}-${width}`);
        if(page==='plugins'){
          await until("document.querySelector('#image-plugin-badge')?.textContent==='已安装'");
          await until("!!document.querySelector('[data-market-detail]')");
          const grid=await evaluate("(()=>{const a=document.querySelector('.builtin-plugin').getBoundingClientRect(),b=document.querySelector('.market-card').getBoundingClientRect();return {a:a.top,b:b.top,bottom:b.bottom,display:getComputedStyle(document.querySelector('#builtin-plugins')).display}})()");
          assert.equal(grid.display,'contents');if(width>=760)assert.ok(grid.b<820,'Community plugins appear without scrolling');
          if(width===1120)assert.ok(Math.abs(grid.a-grid.b)<2,'Built-ins and community share the first row');
          const cardLayout=await evaluate("(()=>{const c=document.querySelector('.market-card'),f=c.querySelector('footer'),a=c.querySelector('.market-card-actions').getBoundingClientRect(),r=f.getBoundingClientRect(),n=f.querySelector('.market-download-count').getBoundingClientRect(),p=f.querySelector('b').getBoundingClientRect();return {price:f.querySelector('b').textContent,count:f.querySelector('.market-download-count').textContent.trim().replace(/\\s+/g,' '),above:a.bottom<=r.top+1,right:n.left>p.right,oldCount:c.querySelector('small.microcopy').textContent.includes('次')}})()");
          assert.equal(cardLayout.price,'10 点');assert.equal(cardLayout.count,'↓ 5 次');
          assert.equal(cardLayout.above,true);assert.equal(cardLayout.right,true);assert.equal(cardLayout.oldCount,false);
          await snapshot(`plugin-market-${theme}-${width}`);
          await until("document.querySelector('.market-card-cover img')?.complete&&document.querySelector('.market-card-cover img')?.naturalWidth>0");
          if(width===1120){
            const cover=await evaluate("(()=>{const r=document.querySelector('.market-card-cover').getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()");
            await call('Input.dispatchMouseEvent',{type:'mouseMoved',...cover});await sleep(350);
            assert.notEqual(await evaluate("getComputedStyle(document.querySelector('.market-card')).transform"),'none');
            assert.notEqual(await evaluate("getComputedStyle(document.querySelector('.market-card-cover img')).transform"),'none');
            await snapshot(`plugin-market-hover-${theme}`);
            await call('Emulation.setEmulatedMedia',{features:[{name:'prefers-reduced-motion',value:'reduce'}]});
            assert.equal(await evaluate("getComputedStyle(document.querySelector('.market-card')).transform"),'none');
            await call('Emulation.setEmulatedMedia',{features:[]});
            await call('Input.dispatchMouseEvent',{type:'mouseMoved',x:0,y:0});
          }
          await evaluate("document.querySelector('.plugin-help').click()");
          assert.equal(await evaluate("document.querySelector('.builtin-plugin-dialog').open"),true);
          assert.equal(await evaluate("(()=>{const r=document.querySelector('.builtin-plugin-dialog').getBoundingClientRect();return r.left>=0&&r.right<=innerWidth+1&&r.top>=0&&r.bottom<=innerHeight+1})()"),true,'Help dialog stays within viewport');
          await snapshot(`image-plugin-${theme}-${width}`);
          await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
          await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
          await until("!document.querySelector('.builtin-plugin-dialog')");
        }
      }
    }
  }
  await until("document.querySelector('#wechat-start')&&!document.querySelector('#wechat-start').disabled");
  await evaluate("document.querySelector('#wechat-workspace').value='D:/Projects/test';window.folderBehavior='hold';document.querySelector('#wechat-folder-select').click()");
  await until("!!window.resolveFolder");
  assert.equal(await evaluate("document.querySelector('#wechat-bind').disabled || document.querySelector('#wechat-start').disabled"),false);
  await evaluate("window.qrBehavior='hold';document.querySelector('#wechat-start').click()");
  await until("!!window.resolveQr");
  await evaluate("window.resolveFolder({path:'D:\\\\Projects\\\\test'})");
  await until("!document.querySelector('#wechat-folder-select').disabled");
  assert.equal(await evaluate("document.querySelector('#wechat-start').disabled"),false);
  await snapshot('wechat-qr-pending');
  await evaluate("document.querySelector('#wechat-cancel-scan').click();window.resolveQr({content:'https://example.test/stale-qr'})");
  await sleep(100);
  assert.equal(await evaluate("!!document.querySelector('#wechat-qr svg')"),false,'Cancelled request must not render a stale QR');
  await evaluate("window.qrBehavior='fail';document.querySelector('#wechat-start').click()");
  await until("document.querySelector('#wechat-error')?.textContent.includes('超时')");
  assert.equal(await evaluate("document.querySelector('#wechat-bind').disabled || document.querySelector('#wechat-folder-select').disabled || document.querySelector('#wechat-start').disabled"),false);
  await evaluate("window.folderBehavior='fail';document.querySelector('#wechat-folder-select').click()");
  await until("document.querySelector('#wechat-error')?.textContent.includes('选择窗口')");
  assert.equal(await evaluate("document.querySelector('#wechat-folder-select').disabled"),false);
  await evaluate("window.folderBehavior='success';document.querySelector('#wechat-folder-select').click()");
  await until("!document.querySelector('#wechat-folder-select').disabled");
  await evaluate("window.qrBehavior='success';window.wxPollMode='wait';document.querySelector('#wechat-start').click()");
  await until("!!document.querySelector('#wechat-qr svg')");
  assert.equal(await evaluate("document.querySelector('#wechat-folder-select').disabled || document.querySelector('#wechat-start').disabled"),false);
  await evaluate("document.querySelector('#wechat-cancel-scan').click();window.wxPollMode='confirmed';document.querySelector('#wechat-form').requestSubmit()");
  await until("document.querySelector('#wechat-badge').textContent==='连接已开启'");
  await snapshot('wechat-connected');
  assert.equal(await evaluate("window.calls.find(c=>c.command==='wechat_action'&&c.args.action==='start').args.payload.writable"),false);
  const stopCalls=await evaluate("window.calls.filter(c=>c.command==='wechat_action'&&c.args.action==='stop').length");
  await evaluate("document.querySelector('[data-page=config]').click()"); await sleep(2100);
  assert.equal(await evaluate("window.calls.filter(c=>c.command==='wechat_action'&&c.args.action==='stop').length"),stopCalls);
  await evaluate("document.querySelector('[data-page=wechat]').click()");
  await until("!document.querySelector('#wechat-stop').disabled");
  await evaluate("document.querySelector('#wechat-stop').click()");
  await until("document.querySelector('#wechat-badge').textContent==='已绑定 · 未开启'");
  console.log('PASS: folder/QR requests independent; QR cancel, stale results, error recovery, bind and automatic read-only start; navigation preserves worker; both themes and widths');
  await evaluate("document.querySelector('[data-page=config]').click()");
  await until("!!document.querySelector('#provider-new')");
  await evaluate("document.querySelector('[data-edit-provider=nut]').click()");
  await until("!!document.querySelector('#provider-name')");
  assert.equal(await evaluate("document.querySelector('#provider-name').value"),'坚果中转');
  assert.equal(await evaluate("document.querySelector('#provider-url').value"),'https://example.test/v1');
  await evaluate("document.querySelector('#provider-cancel').click();document.querySelector('#provider-new').click();document.querySelector('#provider-custom').click()");
  await until("!!document.querySelector('#provider-name')");
  for(const id of ['provider-name','provider-url','provider-key','provider-model'])assert.equal(await evaluate(`document.querySelector('#${id}').value`),'',`${id} starts blank for a custom provider`);
  await snapshot('custom-provider');
  await evaluate("document.querySelector('#provider-cancel').click();document.querySelector('#provider-catalog-back').click()");
  await until("!!document.querySelector('#nut-service-configure')");
  await evaluate("document.querySelector('#nut-service-configure').click()");
  await until("!!document.querySelector('#provider-name')");
  assert.equal(await evaluate("document.querySelector('#provider-name').value"),'坚果');
  assert.equal(await evaluate("document.querySelector('#provider-url').value"),'https://api.jianguo.ltd/v1');
  console.log('PASS: custom provider starts blank; existing fields and dedicated Jianguo configuration preserved');
  // Re-login without a key exposes the explicit add button; main account center stays prominent.
  await evaluate("document.querySelector('#logout').click()");
  await until("!!document.querySelector('[data-login-mode=password]')");
  await evaluate("window.testHasKey=false;document.querySelector('[data-login-mode=password]').click();document.querySelector('#username').value='tester';document.querySelector('#password').value='fixture-password';document.querySelector('#login-form').requestSubmit()");
  await until("!!document.querySelector('[data-page=account]')");
  await evaluate("document.querySelector('[data-page=account]').click()");
  await until("!!document.querySelector('#account-add-key')");
  assert.equal(await evaluate("document.querySelector('#account-web-center').classList.contains('primary')"),true);
  await evaluate("document.querySelector('#account-add-key').click()");
  await until("!document.querySelector('#account-add-key')");
  assert.equal(await evaluate("document.querySelector('#toggle-login-kami')!==null"),true);
  assert.deepEqual(errors,[]);
  console.log('PASS: password login, add API Key, masked display, prominent webpage center; no JS exceptions');
})().catch(e=>{console.error(e);process.exitCode=1;}).finally(async()=>{
  if(ws?.readyState===1){await call('Browser.close').catch(()=>{});ws.close();}
  if(browser&&!browser.killed)browser.kill();
  if(server)await new Promise(r=>server.close(r));
  await sleep(500);
  if(path.resolve(profile).startsWith(path.resolve(temp)+path.sep)){
    for(let i=0;i<20;i++){try{fs.rmSync(profile,{recursive:true,force:true});break;}catch{await sleep(200);}}
  }
});

