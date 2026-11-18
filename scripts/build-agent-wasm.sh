#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Copyright (c) ibis inc.
# Additional terms under AGPL-3.0 section 7(b): see NOTICE.
# Build crates/ibis-agent for a JavaScript runtime with no window (wasm-bindgen's nodejs target) into target/agent-wasm/.
#
# Needs: rustup with the wasm32-unknown-unknown target, and wasm-bindgen-cli whose version matches the wasm-bindgen crate
# in Cargo.lock.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/dist-profile.sh

rustup target list --installed | grep -qx wasm32-unknown-unknown || rustup target add wasm32-unknown-unknown
REMAP="--remap-path-prefix=$HOME/.cargo/registry/src=crates.io --remap-path-prefix=$HOME/.cargo/git/checkouts=git --remap-path-prefix=$HOME/.rustup=rustup --remap-path-prefix=$(pwd)=."
RUSTFLAGS="$REMAP" CARGO_TARGET_DIR=target/agent cargo build -p ibis-agent --lib --target wasm32-unknown-unknown --release
rm -rf target/agent-wasm
wasm-bindgen target/agent/wasm32-unknown-unknown/release/ibis_agent.wasm --out-dir target/agent-wasm --target nodejs
if command -v wasm-opt >/dev/null; then
  wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int target/agent-wasm/ibis_agent_bg.wasm -o target/agent-wasm/ibis_agent_bg.wasm
fi
echo "Built target/agent-wasm/"
