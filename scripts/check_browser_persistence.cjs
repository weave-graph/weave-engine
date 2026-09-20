// Actual browser worker + IndexedDB failure fences. Uses existing Playwright/browser only.
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const os = require('node:os');
const assert = require('node:assert/strict');
const {execFileSync} = require('node:child_process');
const {chromium} = require('playwright');

async function deadline(promise, label, millis=15000) {
  let timer;
  try {return await Promise.race([promise,new Promise((_,reject)=>{
    timer=setTimeout(()=>reject(new Error(`${label} timed out`)),millis);
  })]);} finally {clearTimeout(timer);}
}

async function main() {
  const artifact = path.resolve(process.argv[2] || 'target/wasm32-unknown-emscripten/debug/examples/browser_image_probe.js');
  const files = new Map([
    ['/probe.js', artifact], ['/browser_image_probe.wasm', artifact.replace(/\.js$/, '.wasm')],
    ['/worker.js', path.resolve('examples/browser-image/worker.js')],
  ]);
  const server = http.createServer((request, response) => {
    const file = files.get(request.url);
    if (!file) {response.end('<!doctype html><title>Weave persistence experiment</title>');return;}
    response.setHeader('Content-Type', file.endsWith('.wasm') ? 'application/wasm' : 'text/javascript');
    fs.createReadStream(file).on('error', error => response.destroy(error)).pipe(response);
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const blankSqlite = execFileSync('python3', ['-c',
    'import sqlite3,tempfile,pathlib,base64\nwith tempfile.TemporaryDirectory() as d:\n p=pathlib.Path(d)/"blank.db"\n c=sqlite3.connect(p);c.execute("VACUUM");c.close()\n print(base64.b64encode(p.read_bytes()).decode())'
  ], {encoding:'utf8'}).trim();
  let browser, browserPid;
  const profile=fs.mkdtempSync(path.join(os.tmpdir(),'weave-browser-idb-'));
  const checks = [], measurements = [];
  const record=text=>{checks.push(text);console.error(text);};
  async function rememberOwnedBrowser() {
    const session=await deadline(browser.newBrowserCDPSession(),'browser session');
    const info=await deadline(session.send('SystemInfo.getProcessInfo'),'browser process identity');
    browserPid=info.processInfo.find(process=>process.type==='browser').id;
    await deadline(session.detach(),'detach browser identity session');
  }
  async function rpc(page, message) {
    return page.evaluate(message => new Promise((resolve, reject) => {
      if (!window.bridge) {
        const bridge = window.bridge = {worker:new Worker('/worker.js'), next:0, waiting:new Map()};
        bridge.worker.onmessage = ({data}) => {const p=bridge.waiting.get(data.id);if(p){bridge.waiting.delete(data.id);clearTimeout(p.timer);p.resolve(data);}};
        bridge.worker.onerror = error => {for(const p of bridge.waiting.values()){clearTimeout(p.timer);p.reject(new Error(error.message));}bridge.waiting.clear();};
      }
      const id=++window.bridge.next;
      const timer=setTimeout(()=>{window.bridge.waiting.delete(id);reject(new Error('worker timeout'));},60000);
      window.bridge.waiting.set(id,{resolve,reject,timer});
      window.bridge.worker.postMessage({id,...message});
    }), message);
  }
  const step = (page, operation, fault) => rpc(page, {kind:'step',operation,fault});
  async function open(page, store, create) {
    const result=await rpc(page,{kind:'open',store,create});
    if(result.storage) measurements.push({stage:'open',...result.storage});
    return result;
  }
  async function reload(page, store) {
    await page.reload();
    const opened=await open(page,store,false);assert.equal(opened.ok,true,JSON.stringify(opened));
    const result=await step(page,0);assert.equal(result.ok,true,JSON.stringify(result));return result.value;
  }
  const value = result => {assert.equal(result.ok,true,JSON.stringify(result));if(result.storage) measurements.push({stage:'operation',...result.storage});return result.value;};
  async function corrupt(page, store, operation) {
    await page.reload(); // Stop owning worker before intentional persisted-state fault injection.
    await page.evaluate(async ({store,operation,blankSqlite}) => {
      const request=indexedDB.open(`weave-image-experiment-${store}`,1);
      const db=await new Promise((resolve,reject)=>{request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error);});
      try {
        let replacement;
        if(operation==='malformed' || operation==='uninitialized') {
          const bytes=operation==='uninitialized' ? Uint8Array.from(atob(blankSqlite),c=>c.charCodeAt(0)) : new Uint8Array(512);
          const sha256=Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)),n=>n.toString(16).padStart(2,'0')).join('');
          replacement={format:1,generation:'1',sha256,bytes:new Blob([bytes])};
        }
        await new Promise((resolve,reject)=>{
          const tx=db.transaction('state','readwrite');tx.oncomplete=resolve;tx.onabort=()=>reject(tx.error);
          if(operation==='missing')tx.objectStore('state').delete('image');
          else tx.objectStore('state').put(replacement,'image');
        });
      } finally {db.close();}
    },{store,operation,blankSqlite});
  }
  try {
    const context=await chromium.launchPersistentContext(profile,{headless:true});
    browser=context.browser();
    await rememberOwnedBrowser();
    const page=await context.newPage();await page.goto(origin);
    assert.equal((await open(page,'missing',false)).code,'E_STORE_MISSING');
    await page.reload();assert.equal((await open(page,'primary',true)).ok,true);
    value(await step(page,1));const baseline=value(await step(page,0));
    let recovered=await reload(page,'primary');assert.deepEqual(recovered,baseline);
    assert.match(recovered.result_json,/9007199254740993/);
    assert.match(recovered.result_json,/9223372036854775807/);
    record('create, acknowledged commit, actual page reload, exact i64');

    const afterSql=await step(page,2,'after-sql');assert.equal(afterSql.stage,'after-sql');
    assert.equal((await step(page,0)).code,'E_BUSY');
    recovered=await reload(page,'primary');assert.deepEqual(recovered,baseline);
    record('death after SQL before IDB retains previous image; no concurrent unflushed read');

    assert.equal((await step(page,2,'during-idb')).stage,'during-idb');
    recovered=await reload(page,'primary');
    const duringStep=JSON.parse(recovered.result_json)[0].result.graph.nodes[0].properties.step;
    assert.ok([1,2].includes(duringStep));assert.equal(recovered.events,duringStep);
    record('actual readwrite transaction termination restores a complete old or new state');

    value(await step(page,1));const beforeLost=value(await step(page,0));
    assert.equal((await step(page,2,'after-idb')).stage,'after-idb');
    const afterLost=await reload(page,'primary');
    assert.notEqual(afterLost.head,beforeLost.head);assert.equal(afterLost.events,beforeLost.events+1);
    assert.match(afterLost.result_json,/"step":2/);
    record('IDB commit before lost acknowledgement survives reload without blind replay');

    assert.equal((await step(page,1,'abort')).code,'E_IDB_ABORT');
    assert.equal((await step(page,0)).code,'E_POISON');
    assert.deepEqual(await reload(page,'primary'),afterLost);
    assert.equal((await step(page,1,'synthetic-quota')).code,'E_SYNTHETIC_QUOTA');
    assert.equal((await step(page,0)).code,'E_POISON');
    assert.deepEqual(await reload(page,'primary'),afterLost);
    record('actual transaction abort and separately labeled synthetic quota poison host');

    const session=await context.newCDPSession(page);
    await session.send('Storage.overrideQuotaForOrigin',{origin,quotaSize:1});
    assert.equal((await session.send("Storage.getUsageAndQuota",{origin})).overrideActive,true);
    // Chromium retains an IndexedDB space allowance for30s; expire it before testing enforcement.
    await new Promise(resolve=>setTimeout(resolve,31000));
    try {
      const quota=await step(page,5);assert.equal(quota.ok,false,JSON.stringify(quota));assert.equal(quota.code,'QuotaExceededError');
      assert.equal((await step(page,0)).code,'E_POISON');
    } finally {await session.send('Storage.overrideQuotaForOrigin',{origin});}
    assert.deepEqual(await reload(page,'primary'),afterLost);
    record('real browser quota-enforced IDB failure retains acknowledged image');

    const second=await context.newPage();await second.goto(origin);
    assert.equal((await open(second,'primary',false)).code,'E_OWNER_BUSY');
    await page.reload();assert.deepEqual(await reload(second,'primary'),afterLost);
    await second.close();
    assert.deepEqual(await reload(page,'primary'),afterLost);
    record('cross-tab lifetime lock and owner termination/reacquisition');

    const rejected=await step(page,3);assert.equal(rejected.ok,false);
    assert.deepEqual(value(await step(page,0)),afterLost);
    assert.deepEqual(await reload(page,'primary'),afterLost);
    record('mixed Program authorization failure preserves atomic rollback');

    const oversized=await step(page,4);assert.equal(oversized.code,'E_IMAGE_BUDGET',JSON.stringify(oversized));
    assert.equal((await step(page,0)).code,'E_POISON');
    assert.deepEqual(await reload(page,'primary'),afterLost);
    record('post-SQL oversized image withholds result, poisons and reloads prior state');

    await page.reload();assert.equal((await open(page,'near-cap',true)).ok,true);
    const large=await step(page,5);const largeValue=value(large);
    assert.ok(large.storage.image_bytes>=7*1024*1024 && large.storage.image_bytes<=8*1024*1024);
    const reopenedLarge=await reload(page,'near-cap');
    assert.equal(reopenedLarge.head,largeValue.head);assert.equal(reopenedLarge.events,largeValue.events);
    record('near-cap image acknowledges and restores through real IndexedDB');

    await page.reload();assert.equal((await open(page,'malformed',true)).ok,true);
    value(await step(page,1));await corrupt(page,'malformed','malformed');
    assert.equal((await open(page,'malformed',false)).ok,false);
    assert.equal((await step(page,0)).code,'E_POISON');
    await page.reload();assert.equal((await open(page,'uninitialized',true)).ok,true);
    await corrupt(page,'uninitialized','uninitialized');
    assert.equal((await open(page,'uninitialized',false)).code,'E_IMAGE_UNINITIALIZED');
    assert.equal((await step(page,0)).code,'E_POISON');
    await page.reload();assert.equal((await open(page,'sentinel',true)).ok,true);
    await corrupt(page,'sentinel','missing');
    assert.equal((await open(page,'sentinel',false)).code,'E_STORE_MISSING');
    await page.reload();assert.equal((await open(page,'sentinel',true)).code,'E_ALREADY_CREATED');
    record('malformed, valid uninitialized SQLite, and missing image never silently reset');

    const browserCdp=await browser.newBrowserCDPSession();
    const disconnected=new Promise(resolve=>browser.once('disconnected',resolve));
    await deadline(Promise.race([browserCdp.send('Browser.crash').catch(()=>{}),disconnected]),'browser crash disconnect');
    console.error('browser process disconnected after crash');
    await deadline(browser.close(),'crashed browser close');
    console.error('opening persisted profile in new browser process');
    const restarted=await chromium.launchPersistentContext(profile,{headless:true});
    browser=restarted.browser();
    await rememberOwnedBrowser();
    const restartedPage=await restarted.newPage();await restartedPage.goto(origin);
    assert.equal((await open(restartedPage,'primary',false)).ok,true);
    assert.deepEqual(value(await step(restartedPage,0)),afterLost);
    record('actual browser-process crash and profile reopen retains acknowledged state');
    console.log(JSON.stringify({profile:'experimental-idb-image-v1',browser:browser.version(),
      image_cap_bytes:8*1024*1024,checks,measurements,physical_power_loss_tested:false,
      real_quota_enforcement:true,synthetic_quota_separate:true}));
    await deadline(restarted.close(),'restarted context close');
  } finally {
    try {if(browser)await deadline(browser.close(),'owned browser cleanup');}
    catch(error) {
      // This first host harness runs on macOS/Linux. Kill only the recorded browser
      // PID after verifying its command still contains this exact temporary profile.
      let command='';
      try {command=execFileSync('ps',['-p',String(browserPid),'-o','command='],{encoding:'utf8'});} catch {}
      if(command.includes(profile)) {process.kill(browserPid,'SIGKILL');await new Promise(resolve=>setTimeout(resolve,250));}
      console.error(error.message);
    } finally {
      server.closeAllConnections();
      await deadline(new Promise(resolve=>server.close(resolve)),'owned server close',5000);
      fs.rmSync(profile,{recursive:true,force:true,maxRetries:5,retryDelay:100});
    }
  }
}
main().catch(error=>{console.error(error);process.exitCode=1;});
