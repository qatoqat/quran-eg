#!/bin/sh
# Build the web app into dist/.
#
#   scripts/build-web.sh               # served from the root of a site
#   scripts/build-web.sh /quran-eg/    # served under a base path (GitHub Pages)
#
# Needs the wasm32-unknown-unknown target and the wasm-bindgen command of the
# very version in Cargo.lock:
#   cargo install wasm-bindgen-cli --locked --version "$(scripts/build-web.sh --bindgen-version)"
# wasm-opt (binaryen) is used to shrink the result if it is installed.
set -eu
cd "$(dirname "$0")/.."

bindgen_version=$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/[^0-9.]/, ""); print; exit }' Cargo.lock)
if [ "${1:-}" = "--bindgen-version" ]; then
  echo "$bindgen_version"
  exit 0
fi

# The base path, with a slash at each end.
base=$(printf '/%s/' "${1:-}" | sed 's|//*|/|g')

installed=$(wasm-bindgen --version 2>/dev/null | awk '{ print $2 }')
if [ "$installed" != "$bindgen_version" ]; then
  echo "wasm-bindgen $bindgen_version is needed (found: ${installed:-none})." >&2
  echo "  cargo install wasm-bindgen-cli --locked --version $bindgen_version" >&2
  exit 1
fi

cargo build -p quran-eg --target wasm32-unknown-unknown --profile web --locked

rm -rf dist
mkdir -p dist
wasm-bindgen --target web --no-typescript --out-dir dist --out-name quran-eg \
  target/wasm32-unknown-unknown/web/quran-eg.wasm
if command -v wasm-opt >/dev/null 2>&1; then
  wasm-opt -O2 -o dist/quran-eg_bg.wasm.opt dist/quran-eg_bg.wasm && mv dist/quran-eg_bg.wasm.opt dist/quran-eg_bg.wasm
fi

sed "s|<base href=\"/\">|<base href=\"$base\">|" app/web/index.html > dist/index.html
# Any path below the base (/2, /juz/30, ...) is the app: static hosts serve 404.html for them.
cp dist/index.html dist/404.html
cp -r app/web/fonts dist/fonts
cp packaging/quran-eg.svg dist/icon.svg
# (GitHub Pages: files are served as they are, without Jekyll.)
: > dist/.nojekyll

echo "dist/ is ready for $base ($(wc -c < dist/quran-eg_bg.wasm) bytes of wasm)"
