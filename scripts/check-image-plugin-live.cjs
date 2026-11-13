// Explicit live check. --generate submits ONE paid request, only when requested.
// Credentials remain inside the installed MCP host's encrypted credential store.
const { parseArgs } = require('node:util');
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const readline = require('node:readline');
const assert = require('node:assert/strict');
const { values } = parseArgs({ options: {
  host: {type:'string'}, home: {type:'string'}, job: {type:'string'},
  generate: {type:'boolean',default:false}, model: {type:'string'}, output: {type:'string'},
} });
const home = values.home || process.env.CODEX_HOME || path.join(os.homedir(),'.codex');
const host = values.host || path.resolve(__dirname,'../dist-portable/codexinstaller-windows-x64/Codex使用助手.exe');
assert.ok(values.job || (values.generate && values.model && values.output), 'Use --job ID, or explicitly --generate --model ID --output FILE');
assert.ok(!(values.job && values.generate), 'Recover OR generate, never both in one run');
const child = spawn(host,['--jianguo-image-mcp','--home',home],{windowsHide:true,stdio:['pipe','pipe','pipe']});
const pending = new Map(); let sequence = 0, stderr = '';
const sleep = ms => new Promise(resolve=>setTimeout(resolve,ms));
readline.createInterface({input:child.stdout}).on('line',line=>{
  try { const response = JSON.parse(line); const p=pending.get(response.id); if(p){pending.delete(response.id);clearTimeout(p.timer);response.error?p.reject(Error(JSON.stringify(response.error))):p.resolve(response.result);} }
  catch(error){console.error('Invalid MCP response:',error.message);}
});
child.stderr.on('data',data=>stderr+=data.toString());
child.on('error',error=>{for(const p of pending.values()){clearTimeout(p.timer);p.reject(error);}pending.clear();});
child.on('exit',code=>{for(const p of pending.values()){clearTimeout(p.timer);p.reject(Error('MCP exited '+code+' '+stderr));}pending.clear();});
function call(method,params){return new Promise((resolve,reject)=>{
  const id=++sequence;const timer=setTimeout(()=>{pending.delete(id);reject(Error('MCP request timed out; do not repeat paid submission'));},210000);
  pending.set(id,{resolve,reject,timer});child.stdin.write(JSON.stringify({jsonrpc:'2.0',id,method,params})+'\n');
});}
async function tool(name,args){
  const response=await call('tools/call',{name,arguments:args});
  const text=response.content?.find(item=>item.type==='text')?.text;
  if(response.isError)throw Error(text||'Tool error');return JSON.parse(text);
}
(async()=>{
  const initialized=await call('initialize',{protocolVersion:'2024-11-05',capabilities:{},clientInfo:{name:'jianguo-live-check',version:'1'}});
  assert.equal(initialized.serverInfo.name,'jianguo-image');
  child.stdin.write(JSON.stringify({jsonrpc:'2.0',method:'notifications/initialized'})+'\n');
  let job=values.job;
  if(values.generate){
    const output=path.resolve(values.output);assert.ok(!fs.existsSync(output),'Output already exists');
    const submitted=await tool('generate_image',{model:values.model,prompt:'A minimal square test illustration: one fresh green leaf centered on a soft white background, clean flat vector style, no words, no watermark.',aspectRatio:'1:1',imageSize:'1K',outputFile:output});
    job=submitted.jobId;assert.ok(job);console.log('ONE paid request submitted:',JSON.stringify({jobId:job,model:values.model}));
  }
  console.log('Querying original job:',job);
  let last;
  const deadline=Date.now()+10*60*1000;
  while(Date.now()<deadline){
    const result=await tool('get_image_result',{jobId:job});
    if(result.status!==last){console.log('Job status:',result.status);last=result.status;}
    if(result.status==='completed'){
      const data=fs.readFileSync(result.filePath);
      const valid=data.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10]))||data.subarray(0,3).equals(Buffer.from([255,216,255]))||(data.subarray(0,4).toString()==='RIFF'&&data.subarray(8,12).toString()==='WEBP');
      assert.ok(valid,'Downloaded result is not a recognized image');assert.ok(data.length>100,'Empty/truncated result');
      assert.deepEqual(await tool('get_image_result',{jobId:job}),result,'Repeat query must reuse saved result');
      console.log('PASS:',JSON.stringify({...result,bytes:data.length}));return;
    }
    if(['failed','submission-unconfirmed'].includes(result.status))throw Error(JSON.stringify(result));
    await sleep(3000);
  }
  throw Error('Timed out waiting for original job '+job+'; no repeat paid submission was made');
})().catch(error=>{console.error(error.message);process.exitCode=1;}).finally(async()=>{
  child.stdin.end();const exited=new Promise(resolve=>child.once('exit',resolve));
  await Promise.race([exited,sleep(3000)]);if(child.exitCode===null)child.kill();
});
