set -u
cd /tmp/claude-1000/nixfull/w23/src
for i in $(seq 1 $1); do
  rm -rf /tmp/claude-1000/nixfull/w23/src/target
  CARGO_INCREMENTAL=0 /home/p2949/Development/stutter/target/debug/xtask fixed-work --identity /tmp/claude-1000/nixfull/w23/identity.json --markers /tmp/claude-1000/nixfull/w23/markers.ndjson >/dev/null 2>/tmp/claude-1000/nixfull/w23/err-$i.log || echo "run $i failed"
done
