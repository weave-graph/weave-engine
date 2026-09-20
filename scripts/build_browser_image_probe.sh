#!/bin/sh
set -eu

# Installation is separate and pinned in docs/BROWSER_PERSISTENCE_PROTOTYPE.md.
: "${WEAVE_EMSDK:?Set WEAVE_EMSDK to the approved pinned emsdk directory}"
export EM_CONFIG="$WEAVE_EMSDK/.emscripten"
export PATH="$WEAVE_EMSDK/upstream/emscripten:$PATH"
export CARGO_BUILD_JOBS=2
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_INCREMENTAL=0
rustc +nightly --version | rg -q '1.100.0-nightly \(420ed2a0c 2026-09-18\)'
emcc --version | rg -q '6.0.9-git \(4e4223852a0835923411059a3929907d7df1232e\)'
# WASM_BIGINT is redundant/deprecated in this pinned SDK; retained here to exactly
# reproduce the first proof's std/codegen/link settings and target artifacts.
export RUSTFLAGS='-Cpanic=abort -C link-arg=-sWASM_BIGINT=1 -C link-arg=-sALLOW_MEMORY_GROWTH=1 -C link-arg=-sMAXIMUM_MEMORY=268435456 -C link-arg=-sENVIRONMENT=node,worker -C link-arg=-sMODULARIZE=1 -C link-arg=-sEXPORT_NAME=createWeaveImageProbe'
nice -n 10 cargo +nightly -Z build-std=std,panic_abort build --locked \
    --target wasm32-unknown-emscripten -p weave-engine \
    --example browser_image_probe --features browser-image-experiment
