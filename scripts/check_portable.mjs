// Execute only the fixed conformance-vector export; this is not a production host.
import { readFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
const module = await WebAssembly.compile(await readFile(process.argv[2]));
assert.equal(WebAssembly.Module.imports(module).length, 0, 'golden module must require no host imports');
const instance = await WebAssembly.instantiate(module, {});
const pointer = instance.exports.weave_golden_ptr();
const length = instance.exports.weave_golden_len();
assert(length > 0 && length <= 1_048_576, 'bounded golden profile output');
const bytes = new Uint8Array(instance.exports.memory.buffer, pointer, length);
process.stdout.write(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
