# Copyright 2026 Gentoo Authors
# Distributed under the terms of the GNU General Public License v2
#
# Packaging skeleton only.
#
# This ebuild is intentionally not production-ready yet. The upstream project
# still builds its eBPF object through the Rust nightly/bpfel build path, which
# does not cleanly fit Gentoo's offline cargo.eclass flow today.
#
# Supported install path for now:
#   scripts/install-local.sh
#
# Revisit this ebuild once stutter has stable tagged releases, a stable eBPF
# object build/install story, and production-ready service packaging.

EAPI=8

inherit cargo systemd

DESCRIPTION="Linux scheduler latency profiler and tuning-validation prototype"
HOMEPAGE="https://github.com/P2949/stutter"
EGIT_REPO_URI="https://github.com/P2949/stutter.git"
EGIT_BRANCH="main"

if [[ ${PV} == 9999 ]]; then
	inherit git-r3
fi

# Packaging skeleton only; not part of the proposed assessed FYP scope.
# The source workspace is multi-licensed: userspace crates are MIT OR Apache-2.0;
# stutter-ebpf is MIT OR GPL-2.0-only. This skeleton declares MIT because MIT is
# available across the packaged workspace components. Revisit before production
# packaging.
LICENSE="MIT"
SLOT="0"
KEYWORDS=""
IUSE="systemd openrc"

BDEPEND="
	|| ( dev-lang/rust-bin dev-lang/rust )
	llvm-core/clang
	dev-util/bpf-linker
	llvm-core/llvm
"

src_compile() {
	export RUSTC_BOOTSTRAP=1
	RUSTUP_TOOLCHAIN=nightly-2026-06-06 cargo_src_compile -p stutter
}

src_unpack() {
	git-r3_src_unpack
	cargo_live_src_unpack
}

src_install() {
	dobin "$(cargo_target_dir)"/stutter
	dodoc docs/INSTALL.md docs/PACKAGING.md docs/DAEMON_CONTRACT.md

	if use systemd; then
		systemd_dounit packaging/systemd/stutter-agent.service
		systemd_dounit packaging/systemd/stutter-autotune-observe.service
		systemd_dounit packaging/systemd/stutter-autotune-low-risk.service
	fi

	if use openrc; then
		newinitd packaging/openrc/stutter-agent stutter-agent
		newinitd packaging/openrc/stutter-autotune-observe stutter-autotune-observe
		newinitd packaging/openrc/stutter-autotune-low-risk stutter-autotune-low-risk
	fi

	keepdir /etc/stutter
	keepdir /var/lib/stutter
	keepdir /var/log/stutter
}

pkg_postinst() {
	elog "Run 'stutter service doctor --manager systemd-system --mode system-observe'"
	elog "or 'stutter service doctor --manager openrc --mode system-observe' before enabling services."
	elog "Low-risk apply mode remains opt-in and should be tested with restore dry-runs first."
}
