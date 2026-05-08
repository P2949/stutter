#!/usr/bin/env bash

repo_root() {
    # Resolve relative to scripts/smoke where this helper resides
    local script_dir
    script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    echo "$(cd "${script_dir}/../.." && pwd)"
}

make_smoke_dir() {
    local name="$1"
    local timestamp
    timestamp=$(date +%Y%m%d-%H%M%S)
    local base_dir="${STUTTER_SMOKE_OUT:-$(repo_root)/target/stutter-smoke}"
    local smoke_dir="${base_dir}/${name}-${timestamp}"
    mkdir -p "${smoke_dir}"
    echo "${smoke_dir}"
}

write_basic_metadata() {
    local out_dir="$1"
    uname -a > "${out_dir}/uname.txt"
    env | sort > "${out_dir}/env.txt"
}

run_or_skip_live() {
    # Run command and capture stdout/stderr to output.log in the current directory
    local status=0
    "$@" > output.log 2>&1 || status=$?

    if [ $status -eq 0 ]; then
        return 0
    fi

    # Check if failure is due to missing permissions or capabilities for eBPF
    if grep -qiE "permission denied|operation not permitted|failed to load ebpf|capabilities|insufficient" output.log; then
        echo "SKIP: live eBPF smoke requires root/capabilities"
        return 77
    fi

    return $status
}
