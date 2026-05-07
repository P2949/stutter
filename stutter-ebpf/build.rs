use which::which;

/// Building this crate has an undeclared dependency on the `bpf-linker` binary. This would be
/// better expressed by [artifact-dependencies][bindeps] but issues such as
/// https://github.com/rust-lang/cargo/issues/12385 make their use impractical for the time being.
///
/// This file implements an imperfect solution: it causes cargo to rebuild the crate whenever the
/// mtime of `which bpf-linker` changes. Note that possibility that a new bpf-linker is added to
/// $PATH ahead of the one used as the cache key still exists. Solving this in the general case
/// would require rebuild-if-changed-env=PATH *and* rebuild-if-changed={every-directory-in-PATH}
/// which would likely mean far too much cache invalidation.
///
/// [bindeps]: https://doc.rust-lang.org/nightly/cargo/reference/unstable.html?highlight=feature#artifact-dependencies
fn main() {
    println!("cargo:rerun-if-env-changed=AYA_BUILD_SKIP");

    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("bpf") {
        return;
    }

    let bpf_linker = match which("bpf-linker") {
        Ok(path) => path,
        Err(err) => {
            eprintln!(
                "error: failed to find `bpf-linker` while building stutter-ebpf for bpfel-unknown-none: {err}"
            );
            eprintln!(
                "hint: build stutter as your normal user with bpf-linker on PATH, then run the built stutter binary with doas/sudo."
            );
            eprintln!(
                "hint: avoid `doas cargo run` unless root has rustup, cargo, and bpf-linker configured."
            );
            eprintln!("hint: install with `cargo install bpf-linker` or preserve PATH explicitly.");
            std::process::exit(1);
        }
    };

    let Some(path) = bpf_linker.to_str() else {
        eprintln!(
            "error: path to bpf-linker is not valid UTF-8: {}",
            bpf_linker.display()
        );
        std::process::exit(1);
    };

    println!("cargo:rerun-if-changed={path}");
}
