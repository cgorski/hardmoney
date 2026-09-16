#!/usr/bin/env sh
# Seeds every target's corpus with the real filings under tests/fixtures/
# (the ones up to 64 KiB; the corpus is not committed -- see .gitignore --
# because libFuzzer rewrites it as it runs, and CI restores an evolved
# copy from its cache). Run from anywhere:
#
#   ./fuzz/seed.sh
#
# Then, on nightly:
#
#   cargo +nightly fuzz run parse_bytes -- -dict=fuzz/dictionary.dict -max_len=65536
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/.." && pwd)"
for target in parse_bytes stream_vs_eager roundtrip validate_reconcile; do
    dir="$here/corpus/$target"
    mkdir -p "$dir"
    find "$root/tests/fixtures" -name '*.fec' -size -64k -exec cp {} "$dir/" \;
done
count="$(find "$here/corpus/parse_bytes" -type f | wc -l | tr -d ' ')"
echo "seeded 4 corpora with $count fixtures each"
