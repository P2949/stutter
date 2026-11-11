set -u
S=/tmp/claude-1000/nixfull/w23/src; O=/tmp/claude-1000/nixfull/w25b; mkdir -p $O; cp /tmp/claude-1000/nixfull/w25/identity.json $O/identity.json
cd $S
game() { pgrep -af 'SteamLaunch AppId|steamapps/common|gamescope|pressure-vessel' | grep -vc 'zsh -c\|pgrep\|study.sh'; }
: > $O/results.tsv; : > $O/markers-all.ndjson
arms=(0 3 6 10 19)
for b in 1 2 3 4; do
  for k in 0 1 2 3 4; do
    nv=${arms[$(( (k + b) % 5 ))]}
    [ "$(game)" = 0 ] || { echo "INVALID game block=$b" >> $O/results.tsv; continue; }
    rm -rf "${S:?}/target"; : > $O/m.ndjson
    taskset -c 0-5 nice -n $nv stress-ng --cpu 6 --timeout 300s >/dev/null 2>&1 & hog=$!
    sleep 2
    CARGO_INCREMENTAL=0 taskset -c 0-5 /home/p2949/Development/stutter/target/debug/xtask fixed-work --identity $O/identity.json --markers $O/m.ndjson >/dev/null 2>&1; rc=$?
    kill $hog 2>/dev/null; wait $hog 2>/dev/null
    ns=$(jq -r 'select(.phase=="WORKLOAD_COMPLETE") | .elapsed_ns' $O/m.ndjson)
    cat $O/m.ndjson >> $O/markers-all.ndjson
    echo -e "$b\t$nv\t$rc\t${ns:-NA}" >> $O/results.tsv
  done
done
echo DONE >> $O/results.tsv
