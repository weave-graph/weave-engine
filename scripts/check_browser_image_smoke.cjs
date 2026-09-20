// Uses an already installed Playwright/browser. No package or browser downloads.
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const assert = require('node:assert/strict');
const { chromium } = require('playwright');

async function main() {
  const artifact = path.resolve(process.argv[2] || 'target/wasm32-unknown-emscripten/debug/examples/browser_image_probe.js');
  const worker = `importScripts('/probe.js');
    createWeaveImageProbe({print: text => postMessage({kind:'output',text}),
      printErr: text => postMessage({kind:'stderr',text})})
      .then(() => postMessage({kind:'complete'}))
      .catch(error => postMessage({kind:'error',text:String(error)}));`;
  const server = http.createServer((request, response) => {
    let file;
    if (request.url === '/probe.js') file = artifact;
    else if (request.url === '/browser_image_probe.wasm') file = artifact.replace(/\.js$/, '.wasm');
    else {
      response.setHeader('Content-Type', request.url === '/worker.js' ? 'text/javascript' : 'text/html');
      response.end(request.url === '/worker.js' ? worker : '<!doctype html><title>Weave image smoke</title>');
      return;
    }
    response.setHeader('Content-Type', file.endsWith('.wasm') ? 'application/wasm' : 'text/javascript');
    fs.createReadStream(file).on('error', error => response.destroy(error)).pipe(response);
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  let browser;
  try {
    browser = await chromium.launch({headless:true});
    const page = await browser.newPage();
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    const records = await page.evaluate(() => new Promise((resolve, reject) => {
      const worker = new Worker('/worker.js');
      const records = [];
      const timer = setTimeout(() => {worker.terminate(); reject(new Error('worker timeout'));}, 30000);
      worker.onerror = error => {clearTimeout(timer); worker.terminate(); reject(new Error(error.message));};
      worker.onmessage = ({data}) => {
        if (data.kind === 'error') {clearTimeout(timer); worker.terminate(); reject(new Error(data.text));}
        else if (data.kind === 'complete') {clearTimeout(timer); worker.terminate(); resolve(records);}
        else records.push(data);
      };
    }));
    const report = JSON.parse(records.find(row => row.kind === 'output').text);
    assert.equal(report.restored_query_identical, true);
    assert.equal(report.exact_i64, true);
    assert.match(report.query_json, /9007199254740993/);
    assert.equal(report.indexeddb_durability_tested, false);
    console.log(JSON.stringify({browser:browser.version(), execution:'dedicated-worker', ...report}));
  } finally {
    if (browser) await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
}
main().catch(error => {console.error(error);process.exitCode=1;});
