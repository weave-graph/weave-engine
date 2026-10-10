// Actual generic host WASM, SDK sources, worker and IndexedDB. No new browsers.
const fs=require('node:fs'),path=require('node:path'),http=require('node:http'),os=require('node:os');
const assert=require('node:assert/strict'),crypto=require('node:crypto');
const {execFileSync}=require('node:child_process');
const {chromium}=require('playwright');
const sha=bytes=>crypto.createHash('sha256').update(bytes).digest('hex');
const request=operation=>JSON.stringify({format:'weave-host-request/2',operation});
const rawRequest=operation=>`{"format":"weave-host-request/2","operation":${operation}}`;
const executeRaw=program=>rawRequest(`{"kind":"execute","program":${program}}`);
const query=graph=>request({kind:'execute',program:{version:'0.21.0',commands:[{op:'query',query:{graph_id:graph}}]}});
const commit=(graph,expected,exact,extra='')=>executeRaw(`{"version":"0.21.0","commands":[{"op":"commit","graph_id":${JSON.stringify(graph)},"expected_head":${JSON.stringify(expected)},"data":{"nodes":[{"id":"n","entity_id":"entity","space_id":"s","properties":{"value":${exact}${extra}},"readers":["owner"]}]}}]}`);
function rawValue(result) {
  assert.equal(result.ok,true,JSON.stringify(result));
  assert.equal(result.poisoned,false);
  const prefix='{"format":"weave-host-response/1","ok":true,"value":';
  const suffix=',"requires_fence":true,"poisoned":false}';
  assert.ok(result.response_json.startsWith(prefix),result.response_json);
  assert.ok(result.response_json.endsWith(suffix),result.response_json);
  return result.response_json.slice(prefix.length,-suffix.length);
}
// Decode only receipt/head/control envelopes. Graphs/SDK programs/templates stay raw.
const control=result=>JSON.parse(rawValue(result));
async function deadline(promise,label,ms=20000) {
  let timer;try{return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error(`${label} timed out`)),ms);})]);}
  finally{clearTimeout(timer);}
}
async function main() {
  const artifact=path.resolve(process.argv[2]),fixtureDir=path.resolve(process.argv[3]),evidence=path.resolve(process.argv[4]);
  fs.mkdirSync(evidence,{recursive:false});
  const fixtures=JSON.parse(fs.readFileSync(path.join(fixtureDir,'fixtures.json')));
  const files=new Map([['/host.js',artifact],['/browser_host.wasm',artifact.replace(/\.js$/,'.wasm')],['/worker.js',path.resolve('examples/browser-host/worker.js')]]);
  const server=http.createServer((req,res)=>{
    res.setHeader('Cache-Control','no-store');const file=files.get(req.url);
    if(!file){res.end('<!doctype html><title>Actual Weave browser host verification</title>');return;}
    res.setHeader('Content-Type',file.endsWith('.wasm')?'application/wasm':'text/javascript');fs.createReadStream(file).on('error',e=>res.destroy(e)).pipe(res);
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  const origin=`http://127.0.0.1:${server.address().port}`,profile=fs.mkdtempSync(path.join(os.tmpdir(),'weave-browser-host-'));
  const checks=[],trace=[],measurements=[];let browser,browserPid,serial=0;
  const record=text=>{checks.push(text);console.error(text);};
  async function identify() {
    const session=await browser.newBrowserCDPSession();const info=await session.send('SystemInfo.getProcessInfo');
    browserPid=info.processInfo.find(p=>p.type==='browser').id;await session.detach();
  }
  async function rpc(page,message) {
    const n=++serial,raw=JSON.stringify(message);fs.writeFileSync(path.join(evidence,`${n}-request.json`),raw);
    const result=await deadline(page.evaluate(message=>new Promise((resolve,reject)=>{
      if(!window.bridge) {
        const b=window.bridge={worker:new Worker('/worker.js'),next:0,waiting:new Map()};
        b.worker.onmessage=({data})=>{const p=b.waiting.get(data.id);if(p){b.waiting.delete(data.id);clearTimeout(p.timer);p.resolve(data);}};
        b.worker.onerror=e=>{for(const p of b.waiting.values()){clearTimeout(p.timer);p.reject(new Error(e.message));}b.waiting.clear();};
      }
      const b=window.bridge,id=++b.next,timer=setTimeout(()=>{b.waiting.delete(id);reject(new Error('worker timeout'));},60000);
      b.waiting.set(id,{resolve,reject,timer});b.worker.postMessage({id,...message});
    }),message),'host RPC',65000);
    const output=JSON.stringify(result);fs.writeFileSync(path.join(evidence,`${n}-response.json`),output);
    trace.push({n,kind:message.kind,request_sha256:sha(raw),response_sha256:sha(output),ok:result.ok,code:result.code,stage:result.stage});
    if(result.storage?.image_bytes)measurements.push({n,...result.storage});return result;
  }
  const authority=(principal='owner',graphs=['g','Input','Output'])=>JSON.stringify({principal,writable_graphs:graphs});
  const open=(page,store,create,authority_raw=authority(),fault)=>rpc(page,{kind:'open',store,create,authority_raw,fault});
  const call=(page,request_raw,fault)=>rpc(page,{kind:'call',request_raw,fault});
  async function stop(page,store) {
    await page.reload();
    await deadline(page.evaluate(async store=>{
      const controller=new AbortController(),timer=setTimeout(()=>controller.abort(),18000);
      try{await navigator.locks.request(`weave-host-image-${store}`,{mode:'exclusive',signal:controller.signal},()=>{});}finally{clearTimeout(timer);}
    },store),'exclusive owner release');
  }
  async function reopen(page,store,authority_raw=authority()) {await stop(page,store);rawValue(await open(page,store,false,authority_raw));}
  async function saved(page,store,dump=false) {
    const result=await page.evaluate(async({store,dump})=>{
      const request=indexedDB.open(`weave-host-image-${store}`,1);
      const db=await new Promise((resolve,reject)=>{request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);});
      try {
        const record=await new Promise((resolve,reject)=>{const tx=db.transaction('state','readonly'),r=tx.objectStore('state').get('image');tx.oncomplete=()=>resolve(r.result);tx.onabort=()=>reject(tx.error);});
        const bytes=new Uint8Array(await record.bytes.arrayBuffer());
        const digest=async b=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',b)),n=>n.toString(16).padStart(2,'0')).join('');
        if(await digest(bytes)!==record.sha256||await digest(new TextEncoder().encode(record.journal_raw))!==record.journal_sha256)throw new Error('generation integrity');
        return {generation:record.generation,sha256:record.sha256,journal_raw:record.journal_raw,journal_sha256:record.journal_sha256,marker:new DataView(bytes.buffer).getUint32(60,false),bytes:bytes.length,...(dump?{image_b64:btoa(Array.from(bytes,c=>String.fromCharCode(c)).join(''))}:{})};
      }finally{db.close();}
    },{store,dump});
    if(dump){fs.writeFileSync(path.join(evidence,`${store}.sqlite`),Buffer.from(result.image_b64,'base64'));delete result.image_b64;fs.writeFileSync(path.join(evidence,`${store}-generation.json`),JSON.stringify(result,null,2));}
    return result;
  }
  async function persistedFingerprint(page,store) {
    return page.evaluate(async store=>{
      const r=indexedDB.open(`weave-host-image-${store}`,1),db=await new Promise((resolve,reject)=>{r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);});
      try {
        const rows=await new Promise((resolve,reject)=>{const tx=db.transaction('state','readonly'),s=tx.objectStore('state'),h=s.get('header'),i=s.get('image');tx.oncomplete=()=>resolve({header:h.result,image:i.result});tx.onabort=()=>reject(tx.error);});
        if(rows.image?.bytes instanceof Blob){const bytes=await rows.image.bytes.arrayBuffer();rows.image={...rows.image,bytes:Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),n=>n.toString(16).padStart(2,'0')).join('')};}
        return JSON.stringify(rows);
      }finally{db.close();}
    },store);
  }
  const blankSqlite=execFileSync('python3',['-c','import sqlite3,tempfile,pathlib,base64\nwith tempfile.TemporaryDirectory() as d:\n p=pathlib.Path(d)/"blank.db"\n c=sqlite3.connect(p);c.execute("VACUUM");c.close()\n print(base64.b64encode(p.read_bytes()).decode())'],{encoding:'utf8'}).trim();
  async function corrupt(page,store,mode) {
    await stop(page,store);
    await page.evaluate(async({store,mode,blankSqlite})=>{
      const r=indexedDB.open(`weave-host-image-${store}`,1),db=await new Promise((resolve,reject)=>{r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);});
      try {
        let row=await new Promise((resolve,reject)=>{const tx=db.transaction('state','readonly'),get=tx.objectStore('state').get('image');tx.oncomplete=()=>resolve(get.result);tx.onabort=()=>reject(tx.error);});
        const digest=async bytes=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),n=>n.toString(16).padStart(2,'0')).join('');
        if(mode==='journal-hash')row.journal_raw+=' ';
        else if(mode==='image-hash')row.sha256='0'.repeat(64);
        else if(mode==='unsupported-format')row.format=1;
        else if(mode==='generation-overflow')row.generation='18446744073709551616';
        else if(mode==='typed-journal'){row.journal_raw='{"format":1,"artifacts":[{"kind":"unknown"}]}';row.journal_sha256=await digest(new TextEncoder().encode(row.journal_raw));}
        else if(mode==='malformed'||mode==='uninitialized'||mode==='future-marker'){
          const bytes=mode==='uninitialized'?Uint8Array.from(atob(blankSqlite),c=>c.charCodeAt(0)):mode==='malformed'?new Uint8Array(512):new Uint8Array(await row.bytes.arrayBuffer());
          if(mode==='future-marker')new DataView(bytes.buffer).setUint32(60,99,false);
          row.bytes=new Blob([bytes]);row.sha256=await digest(bytes);
        }
        await new Promise((resolve,reject)=>{const tx=db.transaction('state','readwrite'),s=tx.objectStore('state');tx.oncomplete=resolve;tx.onabort=()=>reject(tx.error);if(mode==='missing')s.delete('image');else s.put(row,'image');});
      }finally{db.close();}
    },{store,mode,blankSqlite});
  }
  try {
    let context=await chromium.launchPersistentContext(profile,{headless:true});browser=context.browser();await identify();
    let page=await context.newPage();page.on('console',message=>console.error('browser:',message.text()));await page.goto(origin);
    assert.equal((await open(page,'missing',false)).code,'E_STORE_MISSING');
    await page.reload();rawValue(await open(page,'primary',true));
    let revision=control(await call(page,commit('g',null,'9007199254740993')))[0].revision;
    let baseline=rawValue(await call(page,query('g')));assert.match(baseline,/9007199254740993/);
    await reopen(page,'primary');assert.equal(rawValue(await call(page,query('g'))),baseline);
    record('arbitrary request2 Program and exact integer survive actual worker reload');
    for(const fault of ['after-sql','during-idb','after-idb','abort','synthetic-quota']) {
      const before=await saved(page,'primary'),next=commit('g',revision,'9223372036854775807');
      const interrupted=await call(page,next,fault);
      if(fault==='abort'||fault==='synthetic-quota'){
        assert.equal(interrupted.code,fault==='abort'?'E_IDB_ABORT':'E_SYNTHETIC_QUOTA');assert.equal((await call(page,query('g'))).code,'E_POISON');
      }else{assert.equal(interrupted.stage,fault);assert.equal((await call(page,query('g'))).code,'E_BUSY');}
      await stop(page,'primary');const after=await saved(page,'primary');
      if(['after-sql','abort','synthetic-quota'].includes(fault))assert.deepEqual(after,before);
      else if(fault==='after-idb')assert.equal(BigInt(after.generation),BigInt(before.generation)+1n);
      else assert.ok(after.sha256===before.sha256||BigInt(after.generation)===BigInt(before.generation)+1n);
      rawValue(await open(page,'primary',false));const current=rawValue(await call(page,query('g')));
      if(after.sha256===before.sha256)assert.equal(current,baseline);else assert.match(current,/9223372036854775807/);
      // Reconcile actual head. Never blindly replay the uncertain Program.
      revision=JSON.parse(current)[0].result.snapshots.g;baseline=current;
      record(`${fault}: no unfenced payload; complete saved generation and actual head reconciliation`);
    }
    const before=await saved(page,'primary');
    const duplicate='{"format":"weave-host-request/2","operation":{"kind":"capabilities","ki\\u006ed":"capabilities"}}';
    const rejected=await call(page,duplicate);assert.equal(rejected.ok,false);assert.match(rejected.response_json,/E_HOST_INPUT/);assert.equal(rejected.requires_fence,false);
    assert.deepEqual(await saved(page,'primary'),before);
    const legacy=await call(page,JSON.stringify({format:'weave-host-request/1',operation:{kind:'lifecycle',adapter:'none',state:'running'}}));
    assert.match(legacy.response_json,/E_HOST_VERSION/);assert.equal(legacy.requires_fence,false);
    const denied=await call(page,request({kind:'actor_state',adapter:'missing'}));assert.equal(denied.ok,false);assert.equal(denied.requires_fence,true);assert.ok(denied.storage);
    record('duplicate decoded fields/legacy opcode reject before storage; invoked ordinary error is durably fenced');
    const second=await context.newPage();await second.goto(origin);assert.equal((await open(second,'primary',false)).code,'E_OWNER_BUSY');await second.close();
    record('second actual tab cannot open an independent writable copy');

    for(const [index,fault] of ['after-sql','abort','during-idb','after-idb'].entries()) {
      const raws=fixtures.cases.flatMap(f=>[f.sdk_raw,f.upgraded_sdk_raw]),before=await saved(page,'primary');
      const sdk_raw=raws[index],interrupted=await rpc(page,{kind:'retain_sdk',sdk_raw,fault});
      if(fault==='abort')assert.equal(interrupted.code,'E_IDB_ABORT');else assert.equal(interrupted.stage,fault);
      await stop(page,'primary');const after=await saved(page,'primary');
      if(fault==='after-sql'||fault==='abort')assert.deepEqual(after,before);
      else {
        const old=after.generation===before.generation;
        if(old)assert.deepEqual(after,before);
        else{assert.equal(BigInt(after.generation),BigInt(before.generation)+1n);const journal=JSON.parse(after.journal_raw);assert.equal(journal.artifacts.at(-1).sdk_raw,sdk_raw);}
        if(fault==='after-idb')assert.notEqual(after.generation,before.generation);
      }
      rawValue(await open(page,'primary',false));assert.equal(rawValue(await call(page,query('g'))),baseline);
      record(`complete SDK journal ${fault}: atomic generation and original inventory bytes`);
    }
    const invalidBefore=await saved(page,'primary'),invalid=await rpc(page,{kind:'retain_sdk',sdk_raw:'{"format":"weave-compiler-response/1","ok":true}'});
    assert.equal(invalid.ok,false);assert.equal(invalid.requires_fence,false);assert.match(invalid.response_json,/E_HOST_ARTIFACT/);assert.deepEqual(await saved(page,'primary'),invalidBefore);
    record('invalid SDK inventory cannot enter the durable artifact journal');

    const quotaSession=await context.newCDPSession(page);
    await quotaSession.send('Storage.overrideQuotaForOrigin',{origin,quotaSize:1});assert.equal((await quotaSession.send('Storage.getUsageAndQuota',{origin})).overrideActive,true);
    // Chromium caches an IDB space allowance for 30 s. Wait for actual enforcement.
    await new Promise(resolve=>setTimeout(resolve,31000));
    try {
      const padding=',"payload":'+JSON.stringify('x'.repeat(2*1024*1024));
      const rejected=await call(page,commit('g',revision,'-9223372036854775808',padding));
      assert.equal(rejected.ok,false);assert.equal(rejected.code,'QuotaExceededError',JSON.stringify(rejected));assert.equal((await call(page,query('g'))).code,'E_POISON');
    }finally{await quotaSession.send('Storage.overrideQuotaForOrigin',{origin});await quotaSession.detach();}
    await reopen(page,'primary');assert.equal(rawValue(await call(page,query('g'))),baseline);
    record('actual browser quota enforcement withholds outcome and restores acknowledged graph and journals');

    const tooLarge=await call(page,commit('g',revision,'-9223372036854775808',',"payload":'+JSON.stringify('x'.repeat(9*1024*1024))));
    assert.equal(tooLarge.ok,false);assert.ok(['E_HOST_UNCERTAIN','E_IMAGE_BUDGET'].includes(tooLarge.code),JSON.stringify(tooLarge));assert.equal((await call(page,query('g'))).code,'E_POISON');
    await reopen(page,'primary');assert.equal(rawValue(await call(page,query('g'))),baseline);
    record('post-SQL capacity failure cannot acknowledge or expose the oversized in-memory graph');
    await stop(page,'primary');rawValue(await open(page,'near-cap',true));
    const large=await call(page,commit('g',null,'-9223372036854775808',',"payload":'+JSON.stringify('x'.repeat(7*1024*1024))));
    const largeReceipt=control(large);assert.ok(large.storage.image_bytes>=7*1024*1024&&large.storage.image_bytes+large.storage.journal_bytes<=8*1024*1024);
    await reopen(page,'near-cap');const largeQuery=rawValue(await call(page,query('g')));assert.match(largeQuery,/-9223372036854775808/);assert.ok(largeQuery.includes(largeReceipt[0].revision));await stop(page,'near-cap');
    record('near-cap actual IndexedDB image and exact minimum integer restore successfully');

    for(let index=0;index<fixtures.cases.length;index++) {
      const f=fixtures.cases[index],store=`source-${index}`;await stop(page,'primary');rawValue(await open(page,store,true));
      rawValue(await call(page,executeRaw(f.program_raw)));
      rawValue(await rpc(page,{kind:'install_handler',sdk_raw:f.sdk_raw,config_raw:f.config_raw}));
      const owned=JSON.parse((await saved(page,store)).journal_raw);assert.equal(owned.artifacts.length,1);assert.equal(owned.artifacts[0].sdk_raw,f.sdk_raw);assert.equal(owned.artifacts[0].config_raw,f.config_raw);
      control(await call(page,request({kind:'lifecycle',adapter:'projection',state:'running'})));
      const event=control(await call(page,request({kind:'poll',adapter:'projection'})));
      const preparation=control(await call(page,request({kind:'prepare',adapter:'projection',event:event.id,lease:event.lease})));
      const complete=request({kind:'complete',adapter:'projection',event:event.id,lease:event.lease,preparation:preparation.preparation_id});
      assert.equal((await call(page,complete,'after-idb')).stage,'after-idb');await reopen(page,store);
      assert.equal(control(await call(page,complete)).duplicate,true);
      const originalOutput=rawValue(await call(page,query('Output')));
      if(index===0)assert.ok(originalOutput.includes(f.exact_integer));else assert.match(originalOutput,/weave:explanation/);
      control(await call(page,request({kind:'lifecycle',adapter:'projection',state:'paused'})));
      const rebuildInputs=rawValue(await call(page,request({kind:'compiled_rebuild_inputs',adapter:'projection'})));
      const rebuild=rawRequest(`{"kind":"compiled_rebuild","request":{"inputs":${rebuildInputs},"nonce":"browser-source-rebuild"}}`);
      assert.equal((await call(page,rebuild,'after-idb')).stage,'after-idb');await reopen(page,store);assert.equal(control(await call(page,rebuild)).duplicate,true);
      const rebuiltOutput=rawValue(await call(page,query('Output')));fs.writeFileSync(path.join(evidence,`${store}-original-output.json`),originalOutput);fs.writeFileSync(path.join(evidence,`${store}-rebuilt-output.json`),rebuiltOutput);
      const inputs=rawValue(await call(page,request({kind:'compiled_migration_inputs',adapter:'projection'})));
      const destination={...f.manifest,id:'projection-v2',version:'2',config_revision:'2',artifact_digest:f.upgraded_digest};
      assert.equal((await rpc(page,{kind:'retain_sdk',sdk_raw:f.upgraded_sdk_raw,fault:'after-idb'})).stage,'after-idb');await reopen(page,store);
      const inventories=JSON.parse((await saved(page,store)).journal_raw).artifacts;
      assert.equal(inventories.length,2);assert.equal(inventories[1].sdk_raw,f.upgraded_sdk_raw);
      const upgrade=rawRequest(`{"kind":"compiled_migrate","request":{"inputs":${inputs},"destination":${JSON.stringify(destination)},"template":${f.upgraded_template_raw},"output":${JSON.stringify(f.output)},"nonce":"browser-source-upgrade","disposition":{"kind":"upgrade"}}}`);
      assert.equal((await call(page,upgrade,'after-idb')).stage,'after-idb');await reopen(page,store);assert.equal(control(await call(page,upgrade)).duplicate,true);
      control(await call(page,request({kind:'lifecycle',adapter:'projection-v2',state:'paused'})));
      const backInputs=rawValue(await call(page,request({kind:'compiled_migration_inputs',adapter:'projection-v2'})));
      const rollback=rawRequest(`{"kind":"compiled_migrate","request":{"inputs":${backInputs},"destination":${JSON.stringify({...f.manifest,id:'projection-restored'})},"template":${f.template_raw},"output":${JSON.stringify(f.output)},"nonce":"browser-source-rollback","disposition":{"kind":"rollback","restore_from":"projection"}}}`);
      assert.equal((await call(page,rollback,'after-idb')).stage,'after-idb');await reopen(page,store);assert.equal(control(await call(page,rollback)).duplicate,true);
      assert.equal(rawValue(await call(page,query('Output'))),rebuiltOutput);
      control(await call(page,request({kind:'lifecycle',adapter:'projection-restored',state:'running'})));assert.equal(control(await call(page,request({kind:'poll',adapter:'projection-restored'}))),null);
      assert.equal(control(await call(page,request({kind:'lag',adapter:'projection-restored'}))).visible_backlog_lower_bound,0);
      await reopen(page,store,authority('foreign'));const foreign=await call(page,rollback);assert.equal(foreign.ok,false);assert.match(foreign.response_json,/E_HOST_AUTH/);
      await reopen(page,store,authority('owner',['Input']));const narrowed=await call(page,rollback);assert.equal(narrowed.ok,false);assert.match(narrowed.response_json,/E_HOST_AUTH/);
      await reopen(page,store);assert.equal(control(await call(page,rollback)).duplicate,true);
      const restored=await saved(page,store,true);assert.equal(restored.marker,29);assert.equal(JSON.parse(restored.journal_raw).artifacts[0].sdk_raw,f.sdk_raw);assert.equal(JSON.parse(restored.journal_raw).artifacts[1].sdk_raw,f.upgraded_sdk_raw);
      record(`actual SDK source ${index}: complete inventory, completion/rebuild/version transfer/rollback and lost acknowledgments; current foreign/narrow authority denied`);
      await stop(page,store);
    }
    for(const mode of ['journal-hash','image-hash','unsupported-format','generation-overflow','typed-journal','malformed','uninitialized','future-marker','missing']) {
      const store=`corrupt-${mode}`;rawValue(await open(page,store,true));await corrupt(page,store,mode);
      const before=await persistedFingerprint(page,store),denied=await open(page,store,false);
      assert.equal(denied.ok,false,JSON.stringify(denied));assert.equal((await call(page,query('g'))).code,'E_POISON');assert.equal(await persistedFingerprint(page,store),before);
      await stop(page,store);assert.equal((await open(page,store,true)).code,'E_ALREADY_CREATED');await stop(page,store);
      record(`${mode}: reject corrupt/unsupported/missing saved state without silent initialization`);
    }
    rawValue(await open(page,'primary',false));assert.equal(rawValue(await call(page,query('g'))),baseline);
    await saved(page,'primary',true);
    execFileSync('python3',[path.resolve(__dirname,'check_browser_host_images.py'),'--fixtures',fixtureDir,'--evidence',evidence],{stdio:['ignore','pipe','pipe']});
    const cdp=await browser.newBrowserCDPSession(),disconnected=new Promise(resolve=>browser.once('disconnected',resolve));
    await deadline(Promise.race([cdp.send('Browser.crash').catch(()=>{}),disconnected]),'actual browser crash');await deadline(browser.close(),'crashed browser cleanup');
    context=await chromium.launchPersistentContext(profile,{headless:true});browser=context.browser();await identify();page=await context.newPage();await page.goto(origin);
    rawValue(await open(page,'primary',false));assert.equal(rawValue(await call(page,query('g'))),baseline);
    record('actual browser-process crash and saved profile reopen retains exact graph');
    const result={profile:'generic-browser-host-image/2',browser:browser.version(),checks,trace,measurements,artifact_sha256:sha(fs.readFileSync(artifact)),wasm_sha256:sha(fs.readFileSync(artifact.replace(/\.js$/,'.wasm'))),compiler_sdk_sha256:fixtures.compiler_sdk_sha256,actual_source_compilations:fixtures.compilations,source_cases:fixtures.cases.length,staged_worker_terminations:trace.filter(t=>t.stage).length,browser_process_crashes:1,physical_power_loss_tested:false,real_quota_tested:true,synthetic_quota_separate:true};
    fs.writeFileSync(path.join(evidence,'report.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result));
  }finally {
    try{if(browser)await deadline(browser.close(),'owned browser cleanup');}
    catch(e){let command='';try{command=execFileSync('ps',['-p',String(browserPid),'-o','command='],{encoding:'utf8'});}catch{}if(command.includes(profile))process.kill(browserPid,'SIGKILL');console.error(e.message);}
    finally{server.closeAllConnections();await deadline(new Promise(resolve=>server.close(resolve)),'owned server cleanup',5000);fs.rmSync(profile,{recursive:true,force:true,maxRetries:5,retryDelay:100});}
  }
}
main().catch(error=>{console.error(error);process.exitCode=1;});
