#!/bin/bash -eu

cd "$SRC/procherd"
cargo fuzz build -O --debug-assertions

fuzz_output="fuzz/target/x86_64-unknown-linux-gnu/release"
for source in fuzz/fuzz_targets/*.rs; do
    target="$(basename "${source%.*}")"
    cp "$fuzz_output/$target" "$OUT/$target"
done

zip -q -j "$OUT/persisted_run_state_seed_corpus.zip" \
    tests/fixtures/contracts/v0.1/run_01J00000000000000000000000/state.json
