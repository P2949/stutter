set -u
cd /tmp/claude-1000/nixfull/w23/src
game() { pgrep -af 'SteamLaunch AppId|steamapps/common|gamescope|pressure-vessel' | grep -vc 'zsh -c\|pgrep\|study.sh'; }
: > /tmp/claude-1000/nixfull/w25/results.tsv
arms=(0 3 6 10 19)
for b in 1 2 3 4 5 6 7 8; do
  for k in 0 1 2 3 4; do
    nv=${arms[$(( (k + b) % 5 ))]}
    [ "$(game)" = 0 ] || { echo "INVALID game block=$b" >> /tmp/claude-1000/nixfull/w25/results.tsv; continue; }
    rm -rf /tmp/claude-1000/nixfull/w23/src/target; : > /tmp/claude-1000/nixfull/w25/m.ndjson
    nice -n $nv stress-ng --cpu 12 --timeout 300s >/dev/null 2>&1 & hog=$!
    sleep 2
    CARGO_INCREMENTAL=0 /home/p2949/Development/stutter/target/debug/xtask fixed-work --identity /tmp/claude-1000/nixfull/w25/identity.json --markers /tmp/claude-1000/nixfull/w25/m.ndjson >/dev/null 2>&1; rc=$?
    kill $hog 2>/dev/null; wait $hog 2>/dev/null
    ns=$(jq -r 'select(.phase=="WORKLOAD_COMPLETE") | .elapsed_ns' /tmp/claude-1000/nixfull/w25/m.ndjson)
    cat /tmp/claude-1000/nixfull/w25/m.ndjson >> /tmp/claude-1000/nixfull/w25/markers-all.ndjson
    echo -e "$b\t$nv\t$rc\t${ns:-NA}" >> /tmp/claude-1000/nixfull/w25/results.tsv
  done
done
echo DONE >> /tmp/claude-1000/nixfull/w25/results.tsv
