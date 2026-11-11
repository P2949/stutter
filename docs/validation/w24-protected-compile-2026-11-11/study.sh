set -u
cd /tmp/claude-1000/nixfull/w23/src
game() { pgrep -af 'SteamLaunch AppId|steamapps/common|gamescope|pressure-vessel' | grep -vc 'zsh -c\|pgrep\|study.sh'; }
exec1() { # study arm
  study=$1; arm=$2; block=$3
  [ "$(game)" = 0 ] || { echo "INVALID game running study=$study block=$block" >> /tmp/claude-1000/nixfull/w24/results.tsv; return 1; }
  rm -rf /tmp/claude-1000/nixfull/w23/src/target
  : > /tmp/claude-1000/nixfull/w24/m.ndjson
  hog=""
  if [ "$study" = 1 ]; then
    n=0; [ "$arm" = cand ] && n=19
    nice -n $n stress-ng --cpu 12 --timeout 300s >/dev/null 2>&1 & hog=$!
    sleep 2
  fi
  pre=""; [ "$study" = 2 ] && [ "$arm" = cand ] && pre="taskset -c 0"
  CARGO_INCREMENTAL=0 $pre /home/p2949/Development/stutter/target/debug/xtask fixed-work --identity /tmp/claude-1000/nixfull/w24/identity.json --markers /tmp/claude-1000/nixfull/w24/m.ndjson >/dev/null 2>&1; rc=$?
  [ -n "$hog" ] && { kill $hog 2>/dev/null; wait $hog 2>/dev/null; }
  ns=$(jq -r 'select(.phase=="WORKLOAD_COMPLETE") | .elapsed_ns' /tmp/claude-1000/nixfull/w24/m.ndjson)
  cat /tmp/claude-1000/nixfull/w24/m.ndjson >> /tmp/claude-1000/nixfull/w24/markers-all.ndjson
  echo -e "$study\t$block\t$arm\t$rc\t${ns:-NA}" >> /tmp/claude-1000/nixfull/w24/results.tsv
}
: > /tmp/claude-1000/nixfull/w24/results.tsv
for study in 1 2; do
  for b in 1 2 3 4 5 6 7 8; do
    case $b in 1|4|5|8) order="base cand";; *) order="cand base";; esac
    for arm in $order; do exec1 $study $arm $b; done
  done
done
echo DONE >> /tmp/claude-1000/nixfull/w24/results.tsv
