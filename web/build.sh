#!/usr/bin/env bash
# Build the web bundle into dist/ — the GitHub Pages site.
#
# Needs: rust wasm32-unknown-unknown target and the wasm-bindgen CLI at the
# version pinned in Cargo.toml (0.2.126). A mismatch between the CLI and the
# crate is the single most common wasm-bindgen failure, so it is checked here
# rather than left to produce a confusing schema error later.
#
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.126 --locked
#   (optional) binaryen, for wasm-opt
set -euo pipefail
cd "$(dirname "$0")/.."

PINNED=$(grep -oP 'wasm-bindgen = "=\K[0-9.]+' Cargo.toml)
HAVE=$(wasm-bindgen --version | awk '{print $2}')
if [[ "$PINNED" != "$HAVE" ]]; then
    echo "wasm-bindgen CLI $HAVE does not match the pinned crate $PINNED" >&2
    exit 1
fi

cargo build --release --target wasm32-unknown-unknown --lib

rm -rf dist
mkdir -p dist/pkg
wasm-bindgen --target web --no-typescript \
    --out-dir dist/pkg --out-name divine \
    target/wasm32-unknown-unknown/release/divine.wasm

if command -v wasm-opt >/dev/null; then
    WASM=dist/pkg/divine_bg.wasm
    echo "wasm-opt: $(du -h "$WASM" | cut -f1) →"
    wasm-opt -O2 --enable-bulk-memory --enable-nontrapping-float-to-int \
        "$WASM" -o "$WASM.opt"
    # An old wasm-opt (notably Ubuntu's apt `binaryen`) reorders the module's
    # tables but leaves wasm-bindgen's __wbindgen_externrefs export bound to the
    # fixed-size function table, so the browser's table.grow(4) at start-up
    # throws "failed to grow table by 4" and the page never renders. Only adopt
    # the optimised wasm if its externref table survived intact; otherwise keep
    # wasm-bindgen's own (correct, if larger) output.
    if python3 web/check_externref.py "$WASM.opt"; then
        mv "$WASM.opt" "$WASM"
        echo "          $(du -h "$WASM" | cut -f1)"
    else
        rm -f "$WASM.opt"
        echo "          skipped — this wasm-opt breaks the externref table; upgrade binaryen" >&2
    fi
else
    echo "wasm-opt not found — skipping (install binaryen to shrink the wasm)"
fi

cp web/index.html dist/index.html
# Pages would otherwise run the output through Jekyll, which ignores files and
# directories beginning with an underscore.
touch dist/.nojekyll

echo
echo "dist/ ready — $(du -sh dist | cut -f1)"
