#!/usr/bin/env bash
# T030 impairment matrix. Usage: bash run-matrix.sh <out.jsonl>
# Expects release binaries at target-unpatched/ (registry quinn-proto 0.11.19,
# --locked) and target-patched/ (same lockfile, quinn-proto path-patched with
# dedup-window.patch only). One line of JSON per run.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
out="$1"
: > "$out"
run() {  # variant mode profile rate seconds seed
  local bin="$here/target-$1/release/t030-quinn-probe"
  local label="$1"; [ "$2" = udp ] && label="raw-udp"
  timeout 120 "$bin" --mode "$2" --profile "$3" --rate "$4" --seconds "$5" \
    --payload 1000 --seed "$6" --label "$label" >> "$out" \
    || echo "{\"failed\":\"$*\"}" >> "$out"
}
for profile in clean reorder impaired; do
  for rate in 30 300 3000; do
    case $rate in 30|300) secs=20;; 3000) secs=10;; esac
    run unpatched quic "$profile" "$rate" "$secs" 1
    run patched quic "$profile" "$rate" "$secs" 1
    run patched udp "$profile" "$rate" "$secs" 1
  done
done
for seed in 2 3; do
  for rate in 30 300; do
    run unpatched quic impaired "$rate" 20 "$seed"
    run patched quic impaired "$rate" 20 "$seed"
    run patched udp impaired "$rate" 20 "$seed"
  done
done
