#!/bin/sh
# Builds the browser package into crates/dense/web/pkg, or with `threads` the
# threaded package into crates/dense/web/pkg-threads.
#
# Needs the wasm32 target and wasm-bindgen-cli of the version in Cargo.lock:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.129 --locked
#
# The threaded package also needs the standard library's source, because std is
# rebuilt with shared memory (atomics); stable rustc accepts the unstable
# `-Z build-std` with RUSTC_BOOTSTRAP=1:
#   rustup component add rust-src
# It runs only where the page is cross-origin isolated (see crispmesh-dense.js).
set -eu
cd "$(dirname "$0")"
target="${CARGO_TARGET_DIR:-target}"
if [ "${1:-}" = threads ]; then
  # A target directory of its own: std and every crate are compiled with other flags.
  target="${target}-wasm-threads"
  CARGO_TARGET_DIR="$target" RUSTC_BOOTSTRAP=1 \
    RUSTFLAGS="-C target-feature=+atomics,+bulk-memory,+mutable-globals -C link-arg=--shared-memory -C link-arg=--import-memory -C link-arg=--max-memory=4294967296 -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base" \
    cargo build --release --locked --target wasm32-unknown-unknown --features threads -Z build-std=panic_abort,std
  wasm-bindgen --target web --out-dir pkg-threads "$target/wasm32-unknown-unknown/release/crispmesh_dense_web.wasm"
  ls -l pkg-threads
  exit 0
fi
cargo build --release --locked --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir pkg "$target/wasm32-unknown-unknown/release/crispmesh_dense_web.wasm"
ls -l pkg
