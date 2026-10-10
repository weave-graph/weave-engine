#!/bin/sh
set -eu
# Same pinned SQLite/Rust/Emscripten ABI as the preserved browser image proof.
: "${WEAVE_EMSDK:?Set WEAVE_EMSDK to the pinned emsdk directory}"
export EM_CONFIG="$WEAVE_EMSDK/.emscripten"
export PATH="$WEAVE_EMSDK/upstream/emscripten:$PATH"
export CARGO_BUILD_JOBS=2
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_INCREMENTAL=0
toolchain="${WEAVE_RUST_TOOLCHAIN:-nightly-2026-09-19}"
rust_version=$(rustc +"$toolchain" --version)
emcc_version=$(emcc --version)
printf '%s\n' "$rust_version" "$emcc_version"
printf '%s\n' "$rust_version" | grep -Fq '1.100.0-nightly (420ed2a0c 2026-09-18)'
printf '%s\n' "$emcc_version" | grep -Eq '6\.0\.9(-git)? \(4e4223852a0835923411059a3929907d7df1232e\)'
export RUSTFLAGS='-Cpanic=abort -C link-arg=-sWASM_BIGINT=1 -C link-arg=-sALLOW_MEMORY_GROWTH=1 -C link-arg=-sMAXIMUM_MEMORY=268435456 -C link-arg=-sSTACK_SIZE=4194304 -C link-arg=-sSTACK_OVERFLOW_CHECK=2 -C link-arg=-sENVIRONMENT=node,worker -C link-arg=-sMODULARIZE=1 -C link-arg=-sEXPORT_NAME=createWeaveBrowserHost -C link-arg=-sEXPORTED_FUNCTIONS=["_main","_weave_browser_open","_weave_browser_operation","_weave_browser_export","_weave_browser_poison","_weave_browser_free"] -C link-arg=-sEXPORTED_RUNTIME_METHODS=["FS","UTF8ToString"]'
nice -n 10 cargo +"$toolchain" -Z build-std=std,panic_abort build --locked \
  --target wasm32-unknown-emscripten -p weave-browser-host \
  --example browser_host --features browser-image-experiment
