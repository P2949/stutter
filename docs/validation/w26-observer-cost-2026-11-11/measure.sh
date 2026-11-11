set -u
W=/tmp/claude-1000/nixfull/w26
pids() { for u in stutter-autotune-bounded@p2949 stutter-privileged-worker@p2949; do systemctl show -p MainPID --value $u.service; done; }
read_state() { # ticks rss_kb fds wbytes
  t=0; r=0; f=0; w=0
  for p in $(pids); do
    s=$(cut -d')' -f2- /proc/$p/stat); set -- $s
    t=$((t + ${12} + ${13}))
    r=$((r + $(awk '/VmRSS/{print $2}' /proc/$p/status)))
    f=$((f + $(sudo -n ls /proc/$p/fd | wc -l)))
    w=$((w + $(sudo -n awk '/^write_bytes/{print $2}' /proc/$p/io)))
  done
  echo "$t $r $f $w"
}
echo -e "interval\tcpu_pct\trss_kb\tfds\twrite_bytes_per_s\tload1" > $W/result.tsv
hz=$(getconf CLK_TCK)
for i in 1 2 3 4 5; do
  if pgrep -af 'SteamLaunch AppId|steamapps/common|gamescope|pressure-vessel' | grep -qv 'zsh -c\|pgrep\|measure.sh'; then echo "INVALID game" >> $W/result.tsv; exit 1; fi
  a=($(read_state)); sleep 60; b=($(read_state))
  echo -e "$i\t$(awk -v d=$((b[0]-a[0])) -v hz=$hz 'BEGIN{printf "%.3f", 100*d/hz/60}')\t${b[1]}\t${b[2]}\t$(( (b[3]-a[3]) / 60 ))\t$(cut -d' ' -f1 /proc/loadavg)" >> $W/result.tsv
done
echo DONE >> $W/result.tsv
