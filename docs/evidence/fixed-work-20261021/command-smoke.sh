set -euo pipefail
cd /home/p2949/Development/stutter
smoke_dir=$(mktemp -d /tmp/stutter-fixed-work-real.XXXXXX)
printf '%s\n' "$smoke_dir" > /tmp/stutter-fixed-work-smoke-path
export CARGO_TARGET_DIR="$smoke_dir/target"
cargo build --locked -p stutter-core --jobs 4 > "$smoke_dir/prime.log" 2>&1
jq -n --arg tree "git:$(git rev-parse 'HEAD^{tree}')" --arg cwd "$PWD" --arg toolchain "$(rustc -Vv)" --arg environment "Pinned nix develop; CARGO_TARGET_DIR=$CARGO_TARGET_DIR; inherited shell environment; functional timing smoke only" '{kind:"compile", source_tree:$tree, target:"stutter-core library", build_state:"package-clean dependencies warm", cache_policy:"clean stutter-core before each measured build; retain dependency artifacts; no compiler cache wrapper", job_count:4, command:["sh","-c","cargo clean -p stutter-core && cargo build --locked -p stutter-core --jobs 4"], working_directory:$cwd, toolchain:$toolchain, environment:$environment}' > "$smoke_dir/identity.json"
for execution in baseline candidate; do
  target/debug/xtask fixed-work --identity "$smoke_dir/identity.json" --markers "$smoke_dir/$execution.ndjson" --target-root-pid "$$" > "$smoke_dir/$execution.log" 2>&1
done
git rev-parse HEAD > "$smoke_dir/source-commit.txt"
jq -s -e 'length == 4 and .[0].phase == "WORKLOAD_START" and .[1].phase == "WORKLOAD_COMPLETE" and .[2].phase == "WORKLOAD_START" and .[3].phase == "WORKLOAD_COMPLETE" and .[1].elapsed_ns > 0 and .[3].elapsed_ns > 0 and .[0].identity == .[2].identity and .[0].target_root_pid == .[2].target_root_pid and .[0].target_starttime_ticks == .[2].target_starttime_ticks and .[2].monotonic_ns >= .[1].monotonic_ns and .[0].execution_id != .[2].execution_id' "$smoke_dir/baseline.ndjson" "$smoke_dir/candidate.ndjson"
printf 'PASS fixed-work command timing: %s\n' "$smoke_dir"
