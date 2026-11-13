// Real Codex discovery and release stdio smoke test; no live Key or paid requests.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const { spawn, spawnSync } = require('node:child_process');
const readline = require('node:readline');
const assert = require('node:assert/strict');
const root = path.resolve(__dirname, '..');
const temp = process.env.TEMP || os.tmpdir();
const directory = fs.mkdtempSync(path.join(temp, 'jianguo-plugins-integration-'));
const home = path.join(directory, 'home');
const market = path.join(directory, 'market');
const cli = process.env.CODEX_TEST_PLUGIN_CLI || path.join(os.homedir(), '.codex/plugins/.plugin-appserver/codex.exe');
const binary = process.env.CODEX_TEST_PLUGIN_HOST || path.join(root, 'dist-portable/codexinstaller-windows-x64/Codex使用助手.exe');
const plugins = [
  { name:'jianguo-image', server:'jianguo_image', flag:'--jianguo-image-mcp', display:'文生图', tools:['list_image_models','get_image_pricing','generate_image','get_image_result'], submit:'generate_image' },
  { name:'jianguo-vectorize', server:'jianguo_vectorize', flag:'--jianguo-vectorize-mcp', display:'位图转矢量图', tools:['get_vectorize_pricing','vectorize_image','get_vectorize_result'], submit:'vectorize_image' },
  { name:'jianguo-video', server:'jianguo_video', flag:'--jianguo-video-mcp', display:'文生视频', tools:['list_video_models','generate_video','get_video_result'], submit:'generate_video' },
];
let child;
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
function run(args, profile=home) {
  const result = spawnSync(cli, args, { cwd: directory, env: { ...process.env, CODEX_HOME: profile }, encoding: 'utf8', timeout: 30_000, windowsHide: true });
  assert.equal(result.status, 0, result.stderr || result.error?.message);
  return JSON.parse(result.stdout);
}
function mcp(p, copiedHost, profile) { return JSON.stringify({mcpServers:{[p.server]:{command:copiedHost,args:[p.flag,'--home',profile]}}}); }
(async () => {
  fs.mkdirSync(home,{recursive:true});
  const copiedHost=path.join(directory,'plugin-host'+(process.platform==='win32'?'.exe':''));
  fs.copyFileSync(binary,copiedHost);if(process.platform!=='win32')fs.chmodSync(copiedHost,0o700);
  for(const p of plugins){
    const source=path.join(market,'plugins',p.name);
    fs.cpSync(path.join(root,'src-tauri/plugins',p.name),source,{recursive:true});
    fs.writeFileSync(path.join(source,'.mcp.json'),mcp(p,copiedHost,home));
  }
  fs.mkdirSync(path.join(market,'.agents/plugins'),{recursive:true});
  fs.writeFileSync(path.join(market,'.agents/plugins/marketplace.json'),JSON.stringify({name:'jianguo-assistant',plugins:plugins.map(p=>({name:p.name,source:{source:'local',path:'./plugins/'+p.name},policy:{installation:'AVAILABLE',authentication:'ON_INSTALL'},category:'Creativity'}))}));
  run(['plugin','marketplace','add',market,'--json']);
  for(const p of plugins){
    const installed=run(['plugin','add',p.name+'@jianguo-assistant','--json']);
    const manifest=JSON.parse(fs.readFileSync(path.join(installed.installedPath,'.codex-plugin/plugin.json'),'utf8'));
    assert.equal(manifest.interface.displayName,p.display);
  }
  const listed=run(['plugin','list','--marketplace','jianguo-assistant','--json']);
  for(const p of plugins)assert.ok(listed.installed.find(item=>item.pluginId===p.name+'@jianguo-assistant')?.enabled);
  console.log('PASS: actual Codex CLI installs and discovers all three built-in plugins');
  // Mirror native installation to verify Codex accepts the exact local config/cache shape.
  const directHome=path.join(directory,'native-home');fs.mkdirSync(directHome,{recursive:true});
  const directMarket=path.join(directHome,'.usage-assistant/image-plugin/marketplace');
  fs.cpSync(market,directMarket,{recursive:true});
  for(const p of plugins){
    const source=path.join(directMarket,'plugins',p.name);
    fs.writeFileSync(path.join(source,'.mcp.json'),mcp(p,copiedHost,directHome));
    const cache=path.join(directHome,'plugins/cache/jianguo-assistant',p.name,'1.0.0');
    fs.mkdirSync(path.dirname(cache),{recursive:true});fs.cpSync(source,cache,{recursive:true});
  }
  fs.writeFileSync(path.join(directHome,'config.toml'),`[marketplaces.jianguo-assistant]\nsource_type = "local"\nsource = '${directMarket}'\n`+plugins.map(p=>`\n[plugins."${p.name}@jianguo-assistant"]\nenabled = true\n`).join(''));
  const direct=run(['plugin','list','--marketplace','jianguo-assistant','--json'],directHome);
  for(const p of plugins)assert.ok(direct.installed.find(item=>item.pluginId===p.name+'@jianguo-assistant')?.enabled);
  console.log('PASS: all assistant-style plugin registrations are recognized together');
  const communityHome=path.join(directory,'community-home');
  const communityMarket=path.join(communityHome,'.usage-assistant/community/42');
  const communitySource=path.join(communityMarket,'plugins/shared-example');
  fs.mkdirSync(path.join(communitySource,'.codex-plugin'),{recursive:true});
  fs.mkdirSync(path.join(communitySource,'skills/shared-example'),{recursive:true});
  fs.writeFileSync(path.join(communitySource,'.codex-plugin/plugin.json'),JSON.stringify({name:'shared-example',version:'1.0.0',description:'Community example',skills:'./skills/',interface:{displayName:'用户分享示例'}}));
  fs.writeFileSync(path.join(communitySource,'skills/shared-example/SKILL.md'),'---\nname: shared-example\ndescription: A fixture skill for plugin discovery.\n---\nAn isolated discovery fixture.\n');
  fs.mkdirSync(path.join(communityMarket,'.agents/plugins'),{recursive:true});
  fs.writeFileSync(path.join(communityMarket,'.agents/plugins/marketplace.json'),JSON.stringify({name:'jg-community-42',interface:{displayName:'用户分享'},plugins:[{name:'shared-example',source:{source:'local',path:'./plugins/shared-example'},policy:{installation:'AVAILABLE',authentication:'ON_INSTALL'},category:'Productivity'}]}));
  const communityCache=path.join(communityHome,'plugins/cache/jg-community-42/shared-example/1.0.0');
  fs.mkdirSync(path.dirname(communityCache),{recursive:true});fs.cpSync(communitySource,communityCache,{recursive:true});
  fs.writeFileSync(path.join(communityHome,'config.toml'),`[marketplaces.jg-community-42]\nsource_type = "local"\nsource = '${communityMarket}'\n[plugins."shared-example@jg-community-42"]\nenabled = true\n`);
  const shared=run(['plugin','list','--marketplace','jg-community-42','--json'],communityHome);
  assert.ok(shared.installed.find(item=>item.pluginId==='shared-example@jg-community-42')?.enabled);
  console.log('PASS: actual Codex CLI recognizes the isolated community marketplace/cache/config layout');
  for(const p of plugins){
    child=spawn(copiedHost,[p.flag,'--home',home],{cwd:directory,windowsHide:true,stdio:['pipe','pipe','pipe']});
    const replies=new Map();let stderr='';
    child.stderr.on('data',data=>stderr+=data.toString());
    readline.createInterface({input:child.stdout}).on('line',line=>{const message=JSON.parse(line);replies.set(message.id,message);});
    const send=value=>child.stdin.write(JSON.stringify(value)+'\n');
    const response=async id=>{for(let i=0;i<100;i++){if(replies.has(id))return replies.get(id);if(child.exitCode!==null)throw Error('Host exited: '+stderr);await sleep(100);}throw Error('MCP response timeout: '+id+' '+stderr);};
    send({jsonrpc:'2.0',id:1,method:'initialize',params:{protocolVersion:'2024-11-05',capabilities:{},clientInfo:{name:'fixture',version:'1'}}});
    assert.equal((await response(1)).result.serverInfo.name,p.name);
    send({jsonrpc:'2.0',method:'notifications/initialized'});
    send({jsonrpc:'2.0',id:2,method:'tools/list'});
    const tools=(await response(2)).result.tools;assert.deepEqual(tools.map(tool=>tool.name),p.tools);
    for(const tool of tools)assert.equal(tool.inputSchema.properties.kami,undefined,'credentials must not come from conversation');
    send({jsonrpc:'2.0',id:3,method:'tools/call',params:{name:p.submit,arguments:{}}});
    const missingCredential=(await response(3)).result;assert.equal(missingCredential.isError,true);
    assert.match(missingCredential.content[0].text,/凭证|安装插件/);
    send({jsonrpc:'2.0',id:4,method:'ping'});assert.deepEqual((await response(4)).result,{});
    child.stdin.end();
    await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Host did not exit')),5000);child.on('exit',code=>{clearTimeout(timer);code===0?resolve():reject(Error('Host exit code '+code));});});
    console.log('PASS: release EXE '+p.display+' stdio MCP handshake, tools, credential protection and clean exit');
  }
})().catch(error=>{console.error(error);process.exitCode=1;}).finally(async()=>{
  if(child&&child.exitCode===null)child.kill();await sleep(300);
  if(path.resolve(directory).startsWith(path.resolve(temp)+path.sep))fs.rmSync(directory,{recursive:true,force:true,maxRetries:4,retryDelay:200});
});
