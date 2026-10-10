/* Trusted app embedding. Raw source/graph JSON cannot supply host authority. */
importScripts('/host.js');
const LIMIT = 8 * 1024 * 1024;
const REQUEST_LIMIT = 16 * 1024 * 1024;
const SDK_LIMIT = REQUEST_LIMIT + 4096;
const encoder = new TextEncoder(), decoder = new TextDecoder('utf-8', {fatal:true});
const never = () => new Promise(() => {});
const fail = (code, preflight=false) => Object.assign(new Error(code), {code,preflight});
const send = (id, value) => postMessage({id,...value});
const hex = bytes => Array.from(bytes,n=>n.toString(16).padStart(2,'0')).join('');
const digest = async bytes => hex(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes)));
let module, database, namespace, generation=0n, busy=false, poisoned=false, initialized=false;
let journal={format:1,artifacts:[]};
function rawBytes(raw, limit) {
  if (typeof raw !== 'string' || raw.length > limit) throw fail('E_HOST_INPUT',true);
  const bytes=encoder.encode(raw);
  if (bytes.length > limit) throw fail('E_HOST_BUDGET',true);
  if (decoder.decode(bytes) !== raw) throw fail('E_HOST_INPUT',true);
  return bytes;
}
function native(fn,...args) {
  let pointer;
  try { pointer=module[fn](...args); }
  catch(error) { console.error('Weave WASM trap',String(error));throw error; }
  if (!pointer) throw fail('E_HOST_UNCERTAIN');
  let frame;
  try { frame=JSON.parse(module.UTF8ToString(pointer)); }
  finally { module._weave_browser_free(pointer); }
  if (frame.format !== 'weave-browser-outcome/1' || typeof frame.ok !== 'boolean'
      || typeof frame.requires_fence !== 'boolean' || typeof frame.poisoned !== 'boolean'
      || typeof frame.response_json !== 'string') throw fail('E_HOST_UNCERTAIN');
  if (frame.poisoned) throw fail('E_HOST_UNCERTAIN'); // Withhold every operational payload.
  return frame;
}
function openDatabase(name) {
  return new Promise((resolve,reject)=>{
    const request=indexedDB.open(name,1);
    request.onupgradeneeded=()=>request.result.createObjectStore('state');
    request.onerror=()=>reject(request.error);
    request.onblocked=()=>reject(fail('E_IDB_BLOCKED'));
    request.onsuccess=()=>resolve(request.result);
  });
}
function load() {
  return new Promise((resolve,reject)=>{
    const tx=database.transaction('state','readonly'), store=tx.objectStore('state');
    const header=store.get('header'), image=store.get('image');
    tx.oncomplete=()=>resolve({header:header.result,image:image.result});
    tx.onabort=()=>reject(tx.error||fail('E_IDB_ABORT'));
  });
}
function validateJournal(value) {
  if (!value || value.format !== 1 || !Array.isArray(value.artifacts) || value.artifacts.length > 16
      || Object.keys(value).sort().join(',') !== 'artifacts,format') throw fail('E_JOURNAL_FORMAT');
  for (const entry of value.artifacts) {
    if (entry && entry.kind==='sdk') {
      if (Object.keys(entry).sort().join(',') !== 'kind,sdk_raw') throw fail('E_JOURNAL_FORMAT');
      rawBytes(entry.sdk_raw,SDK_LIMIT);continue;
    }
    if (!entry || typeof entry.config_raw !== 'string' || !['handler','actor'].includes(entry.kind)) throw fail('E_JOURNAL_FORMAT');
    rawBytes(entry.config_raw,entry.kind==='actor'?2*1024*1024:256*1024);
    if (entry.kind==='handler') rawBytes(entry.sdk_raw,SDK_LIMIT);
    const keys=Object.keys(entry).sort().join(',');
    if (keys !== (entry.kind==='handler'?'config_raw,kind,sdk_raw':'config_raw,kind')) throw fail('E_JOURNAL_FORMAT');
  }
  return value;
}
async function persist(id, pendingJournal, fault) {
  const exported=native('_weave_browser_export');
  if (!exported.ok) throw fail('E_IMAGE_STORAGE');
  // Only this native export metadata is decoded. Operational graph/artifact
  // payloads remain opaque UTF-8 inside response_json throughout the worker.
  const count=JSON.parse(exported.response_json).value.bytes;
  const bytes=module.FS.readFile('/tmp/runtime.image');
  const journalRaw=JSON.stringify(pendingJournal), journalBytes=encoder.encode(journalRaw);
  if (!Number.isSafeInteger(count) || count!==bytes.length || count<100
      || bytes.length+journalBytes.length > LIMIT) throw fail('E_IMAGE_BUDGET');
  const sha256=await digest(bytes), journalSha256=await digest(journalBytes);
  if (fault==='after-sql') { send(id,{stage:'after-sql'});await never(); }
  const next=generation+1n;
  if (next>18446744073709551615n) throw fail('E_GENERATION');
  const record={format:2,generation:next.toString(),sha256,bytes:new Blob([bytes]),journal_raw:journalRaw,journal_sha256:journalSha256};
  const started=performance.now();
  const durability=await new Promise((resolve,reject)=>{
    const tx=database.transaction('state','readwrite',{durability:'strict'}), store=tx.objectStore('state');
    tx.oncomplete=()=>resolve(tx.durability);
    tx.onabort=()=>reject(fault==='synthetic-quota'?fail('E_SYNTHETIC_QUOTA'):(tx.error||fail('E_IDB_ABORT')));
    store.put({format:2,namespace},'header');store.put(record,'image');
    if (fault==='abort'||fault==='synthetic-quota') tx.abort();
    else if (fault==='during-idb') {
      const keepAlive=()=>{store.get('header').onsuccess=keepAlive;};keepAlive();send(id,{stage:'during-idb'});
    }
  });
  generation=next;journal=pendingJournal;
  if (fault==='after-idb') { send(id,{stage:'after-idb'});await never(); }
  return {generation:next.toString(),image_bytes:bytes.length,journal_bytes:journalBytes.length,durability,persist_ms:performance.now()-started};
}
async function initialize(id, request) {
  if (initialized||module||typeof request.store!=='string'||!/^[a-z0-9-]{1,80}$/.test(request.store)
      || typeof request.create!=='boolean') throw fail('E_OPEN');
  const authority=rawBytes(request.authority_raw,128*1024);
  namespace=`weave-host-image-${request.store}`;
  await new Promise((resolve,reject)=>{
    navigator.locks.request(namespace,{mode:'exclusive',ifAvailable:true},async lock=>{
      if (!lock) {reject(fail('E_OWNER_BUSY'));return;}resolve();await never();
    }).catch(reject);
  });
  database=await openDatabase(namespace);
  database.onversionchange=()=>{poisoned=true;database.close();if(module)module._weave_browser_poison();};
  const saved=await load();
  if (request.create?!!(saved.header||saved.image):!(saved.header&&saved.image)) throw fail(request.create?'E_ALREADY_CREATED':'E_STORE_MISSING');
  let image;
  if (!request.create) {
    const record=saved.image;
    if (saved.header.format!==2||saved.header.namespace!==namespace||record.format!==2
        || typeof record.generation!=='string'||!/^[1-9][0-9]{0,19}$/.test(record.generation)
        || BigInt(record.generation)>18446744073709551615n||!(record.bytes instanceof Blob)
        || typeof record.journal_raw!=='string'||record.journal_raw.length>LIMIT
        || record.bytes.size<100||record.bytes.size>LIMIT||!/^[a-f0-9]{64}$/.test(record.sha256)
        || !/^[a-f0-9]{64}$/.test(record.journal_sha256)) throw fail('E_IMAGE_FORMAT');
    const journalBytes=rawBytes(record.journal_raw,LIMIT);
    if (record.bytes.size+journalBytes.length>LIMIT) throw fail('E_IMAGE_BUDGET');
    image=new Uint8Array(await record.bytes.arrayBuffer());
    if (await digest(image)!==record.sha256||await digest(journalBytes)!==record.journal_sha256) throw fail('E_IMAGE_INTEGRITY');
    journal=validateJournal(JSON.parse(record.journal_raw));generation=BigInt(record.generation);
  }
  crypto.getRandomValues(new Uint8Array(32)); // Fail closed if required browser entropy is absent.
  module=await createWeaveBrowserHost({noInitialRun:true,print(){},printErr(message){console.error('Weave WASM:',message);}});
  if (image) module.FS.writeFile('/tmp/runtime.sqlite',image);
  module.FS.writeFile('/tmp/authority.json',authority);
  const opened=native('_weave_browser_open',request.create?1:0);
  if (!opened.ok) throw fail('E_IMAGE_OPEN');
  const storage=await persist(id,journal,request.fault);
  initialized=true;send(id,{...opened,storage});
}
onmessage=async({data})=>{
  const {id}=data;
  if (poisoned) {send(id,{ok:false,code:'E_POISON'});return;}
  if (busy) {send(id,{ok:false,code:'E_BUSY'});return;}
  busy=true;
  try {
    if (data.kind==='open') await initialize(id,data);
    else {
      if (!initialized) throw fail('E_NOT_OPEN',true);
      let operation, pendingJournal=journal, entry;
      if (data.kind==='call') { module.FS.writeFile('/tmp/request.json',rawBytes(data.request_raw,REQUEST_LIMIT));operation=0; }
      else if (data.kind==='install_handler'||data.kind==='install_actor') {
        const actor=data.kind==='install_actor';
        const config=rawBytes(data.config_raw,actor?2*1024*1024:256*1024);
        if (!actor) module.FS.writeFile('/tmp/sdk.json',rawBytes(data.sdk_raw,SDK_LIMIT));
        module.FS.writeFile('/tmp/install.json',config);operation=actor?2:1;
        entry={kind:actor?'actor':'handler',config_raw:data.config_raw,...(actor?{}:{sdk_raw:data.sdk_raw})};
      } else if (data.kind==='retain_sdk') {
        module.FS.writeFile('/tmp/sdk.json',rawBytes(data.sdk_raw,SDK_LIMIT));operation=3;
        entry={kind:'sdk',sdk_raw:data.sdk_raw};
      } else throw fail('E_OPERATION',true);
      if (entry&&!journal.artifacts.some(e=>JSON.stringify(e)===JSON.stringify(entry))) {
        if (journal.artifacts.length>=16) throw fail('E_JOURNAL_BUDGET',true);
        pendingJournal={format:1,artifacts:[...journal.artifacts,entry]};
        if (encoder.encode(JSON.stringify(pendingJournal)).length>LIMIT) throw fail('E_JOURNAL_BUDGET',true);
      }
      const result=native('_weave_browser_operation',operation);
      if (!result.ok) pendingJournal=journal;
      const storage=result.requires_fence?await persist(id,pendingJournal,data.fault):{generation:generation.toString()};
      send(id,{...result,storage});
    }
  } catch(error) {
    if (!error.preflight||data.kind==='open') {
      poisoned=true;try{if(module)module._weave_browser_poison();}catch{}
    }
    send(id,{ok:false,code:typeof error.code==='string'?error.code:(error.name||'E_HOST_UNKNOWN')});
  } finally {busy=false;}
};
