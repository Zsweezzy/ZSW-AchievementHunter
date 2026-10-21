#!/bin/sh
# Smallest shippable binary: the size-tuned release build, packed with UPX.
set -eu
cargo build --release
bin=target/release/achievement-hunter
out=target/release/achievement-hunter.upx
cp "$bin" "$out"
if command -v upx >/dev/null 2>&1; then
    upx -q --best --lzma "$out"
else
    echo "upx not installed: shipping the uncompressed build" >&2
fi
ls -lh "$out"
