#!/bin/sh
# The tests draw page 1 of the two QCF font sets. Those files are King Fahd
# Complex fonts as served by quran.com; they are not kept in this repository.
# Run this once before `cargo test` (CI does).
set -eu
cd "$(dirname "$0")"
base=https://verses.quran.foundation/fonts/quran/hafs
curl -fsSL "$base/v2/woff2/p1.woff2" -o v2_p1.woff2
curl -fsSL "$base/v4/colrv1/woff2/p1.woff2" -o v4_colr_p1.woff2
