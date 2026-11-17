#!/usr/bin/env bash
# Model-checks EngineChain.tla with TLC, one run per (bounds file, property).
# usage: check.sh [Property ...]     (default: every property)
# prints PASS/FAIL per run; exits 1 if any property fails, 2 on a tool or model error.
set -uo pipefail

# TLC 2.19 (tlaplus 1.7.4) from a fixed nixpkgs commit, so every run uses the same checker
TLA=github:NixOS/nixpkgs/55bfb4d7437bd4e09ab7cba58a43bd68eb310044#tlaplus

# bounds file -> properties checked under it
RUNS=(
  "EngineChain.cfg NoRealloc"
  "EngineChain.cfg NoAudioThreadDrop"
  "EngineChain.cfg OrderAgrees"
  "EngineChain.cfg ChainAgrees"
  "EngineChain.cfg AwaitedComesBack"
  "EngineChainStall.cfg NoAudioThreadDrop"
)

here=$(cd "$(dirname "$0")" && pwd)
out=$(mktemp -d "${TMPDIR:-/tmp}/engine-chain.XXXXXX")
cp "$here/EngineChain.tla" "$out/"
status=0
for run in "${RUNS[@]}"; do
  read -r cfg p <<< "$run"
  if [ $# -gt 0 ] && [[ " $* " != *" $p "* ]]; then continue; fi
  name="${cfg%.cfg}-$p"
  { cat "$here/$cfg"; echo "INVARIANT $p"; } > "$out/$name.cfg"
  nix shell "$TLA" -c tlc -workers auto -cleanup -config "$out/$name.cfg" \
    -metadir "$out/meta-$name" "$out/EngineChain.tla" > "$out/$name.log" 2>&1
  if grep -q "is violated" "$out/$name.log"; then
    echo "FAIL  $cfg $p   trace: $out/$name.log"
    status=$(( status > 1 ? status : 1 ))
  elif grep -q "No error has been found" "$out/$name.log"; then
    echo "PASS  $cfg $p   $(grep -o '[0-9,]* distinct states found' "$out/$name.log" | tail -1)"
  else
    echo "ERROR $cfg $p   tool or model error, not a result: $out/$name.log"
    status=2
  fi
done
exit $status
