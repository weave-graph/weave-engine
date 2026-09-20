/* Experimental fixed-fixture host. No effect tickets or general remote API. */
importScripts('/probe.js');

const LIMIT = 8 * 1024 * 1024;
let module, database, namespace, generation = 0n;
let busy = false, poisoned = false, initialized = false;
const never = () => new Promise(() => {});
const fail = code => Object.assign(new Error(code), {code});
const send = (id, value) => postMessage({id, ...value});
const hex = bytes => Array.from(bytes, n => n.toString(16).padStart(2, '0')).join('');
const digest = async bytes => hex(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)));

function native(fn, ...args) {
  const pointer = module[fn](...args);
  try { return JSON.parse(module.UTF8ToString(pointer)); }
  finally { module._weave_image_free(pointer); }
}

function openDatabase(name) {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(name, 1);
    request.onupgradeneeded = () => request.result.createObjectStore('state');
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(fail('E_IDB_BLOCKED'));
    request.onsuccess = () => resolve(request.result);
  });
}

function load() {
  return new Promise((resolve, reject) => {
    const transaction = database.transaction('state', 'readonly');
    const store = transaction.objectStore('state');
    const header = store.get('header'), image = store.get('image');
    transaction.oncomplete = () => resolve({header:header.result, image:image.result});
    transaction.onabort = () => reject(transaction.error || fail('E_IDB_ABORT'));
  });
}

async function persist(id, fault) {
  const exported = native('_weave_image_export');
  if (!exported.ok) throw fail(exported.code);
  if (exported.value.bytes > LIMIT) throw fail('E_IMAGE_BUDGET');
  const bytes = module.FS.readFile('/tmp/browser.image');
  if (bytes.byteLength !== exported.value.bytes || bytes.byteLength > LIMIT) throw fail('E_IMAGE_SIZE');
  const sha256 = await digest(bytes);
  if (fault === 'after-sql') {send(id, {stage:'after-sql'}); await never();}
  const next = generation + 1n;
  if (next > 18446744073709551615n) throw fail('E_GENERATION');
  const image = {format:1, generation:next.toString(), sha256, bytes:new Blob([bytes])};
  const started = performance.now();
  const durability = await new Promise((resolve, reject) => {
    // Strict is a browser durability hint, not a physical power-loss guarantee.
    const transaction = database.transaction('state', 'readwrite', {durability:'strict'});
    const store = transaction.objectStore('state');
    transaction.oncomplete = () => resolve(transaction.durability);
    transaction.onabort = () => reject(fault === 'synthetic-quota' ? fail('E_SYNTHETIC_QUOTA') : (transaction.error || fail('E_IDB_ABORT')));
    store.put({format:1, namespace}, 'header');
    store.put(image, 'image');
    if (fault === 'abort' || fault === 'synthetic-quota') transaction.abort();
    else if (fault === 'during-idb') {
      // Keep a real readwrite transaction alive until the test terminates this worker.
      const keepAlive = () => {store.get('header').onsuccess = keepAlive;};
      keepAlive();
      send(id, {stage:'during-idb'});
    }
  });
  generation = next;
  if (fault === 'after-idb') {send(id, {stage:'after-idb'}); await never();}
  return {generation:next.toString(), image_bytes:bytes.byteLength, durability,
    persist_ms:performance.now() - started};
}

async function initialize(id, request) {
  if (initialized || module || typeof request.store !== 'string' || !/^[a-z0-9-]{1,80}$/.test(request.store)
      || typeof request.create !== 'boolean') throw fail('E_OPEN');
  namespace = `weave-image-experiment-${request.store}`;
  await new Promise((resolve, reject) => {
    navigator.locks.request(namespace, {mode:'exclusive', ifAvailable:true}, async lock => {
      if (!lock) {reject(fail('E_OWNER_BUSY')); return;}
      resolve();
      await never(); // Worker termination releases ownership; poison retains the lock.
    }).catch(reject);
  });
  database = await openDatabase(namespace);
  database.onversionchange = () => {poisoned = true; database.close();};
  const saved = await load();
  if (request.create ? !!(saved.header || saved.image) : !(saved.header && saved.image)) {
    throw fail(request.create ? 'E_ALREADY_CREATED' : 'E_STORE_MISSING');
  }
  let image;
  if (!request.create) {
    const record = saved.image;
    if (saved.header.format !== 1 || saved.header.namespace !== namespace || record.format !== 1
        || typeof record.generation !== 'string' || !/^[1-9][0-9]{0,19}$/.test(record.generation)
        || BigInt(record.generation) > 18446744073709551615n || !(record.bytes instanceof Blob)
        || record.bytes.size > LIMIT || record.bytes.size < 100 || !/^[a-f0-9]{64}$/.test(record.sha256)) throw fail('E_IMAGE_FORMAT');
    image = new Uint8Array(await record.bytes.arrayBuffer());
    if (await digest(image) !== record.sha256) throw fail('E_IMAGE_INTEGRITY');
    generation = BigInt(record.generation);
  }
  // No fallback randomness or caller-controlled authority time.
  crypto.getRandomValues(new Uint8Array(32));
  module = await createWeaveImageProbe({noInitialRun:true, print(){}, printErr(){}});
  if (image) module.FS.writeFile('/tmp/browser.sqlite', image);
  const opened = native('_weave_image_open', request.create ? 1 : 0);
  if (!opened.ok) throw fail(opened.code);
  const storage = await persist(id, request.fault);
  initialized = true;
  send(id, {...opened, storage});
}

onmessage = async ({data}) => {
  const {id} = data;
  if (poisoned) {send(id, {ok:false, code:'E_POISON'}); return;}
  if (busy) {send(id, {ok:false, code:'E_BUSY'}); return;}
  busy = true;
  try {
    if (data.kind === 'open') await initialize(id, data);
    else {
      if (!initialized || data.kind !== 'step' || !Number.isInteger(data.operation) || data.operation < 0 || data.operation > 5) throw fail('E_OPERATION');
      const result = native('_weave_image_step', data.operation);
      // Query is read-only in this fixed fixture; all writes/errors fence before reply.
      const storage = data.operation === 0 ? {generation:generation.toString()} : await persist(id, data.fault);
      send(id, {...result, storage});
    }
  } catch (error) {
    poisoned = true;
    send(id, {ok:false, code:typeof error.code === 'string' ? error.code : (error.name || 'E_HOST_UNKNOWN')});
  } finally {busy = false;}
};
