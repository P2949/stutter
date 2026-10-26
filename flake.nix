{
  description = "Stutter development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    # Newer bpf-linker supports LLVM 22 bitcode from the pinned nightly.
    nixpkgs-linker.url = "github:NixOS/nixpkgs/nixos-unstable";

    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      nixpkgs-linker,
      rust-overlay,
      ...
    }:
    let
      system = "x86_64-linux";

      pkgs = import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      };

      linkerPkgs = import nixpkgs-linker { inherit system; };

      rustToolchain = pkgs.rust-bin.nightly."2026-06-06".default.override {
        extensions = [
          "rust-src"
          "rustfmt"
          "clippy"
        ];
      };
    in
    {
      # Only publication needs the GitHub client; verification never does (`nix develop .#publish`
      # for `xtask release-publish`).
      devShells.${system} = {
      publish = pkgs.mkShell {
        packages = [
          pkgs.git
          pkgs.gh
        ];
      };

      default = pkgs.mkShell {
        packages = [
          rustToolchain

          # eBPF build tool used by .cargo/config.toml.
          (linkerPkgs.bpf-linker.override {
            llvmPackagesForLinker = linkerPkgs.llvmPackages_22;
          })

          # Repository dependency-policy gate.
          pkgs.cargo-deny

          # Editor support.
          pkgs.rust-analyzer

          # Build metadata and repository tooling.
          pkgs.git

          # Offline smoke scripts validate generated JSON with Python.
          pkgs.python3

          # Deterministic workload tools for the validation campaign (never downloaded at
          # collection time): frame workload, frame logging, storage and CPU stress, network
          # reachability probe. No hosted-service client: verification is local-only.
          pkgs.vkmark
          pkgs.vulkan-tools
          pkgs.mangohud
          pkgs.fio
          pkgs.stress-ng
          pkgs.iputils
          pkgs.util-linux
        ];

        shellHook = ''
          export RUSTUP_TOOLCHAIN=nightly-2026-06-06

          # The Nix shell deliberately owns Rust. Stutter's Aya build path can
          # use the compiler already on PATH when rustup is absent. Refuse an
          # ambiguous environment rather than silently selecting a second
          # independently managed Rust installation.
          if command -v rustup >/dev/null 2>&1 || [ -x "$HOME/.cargo/bin/rustup" ]; then
            echo "error: rustup is visible while entering the Stutter Nix development shell." >&2
            echo "The Stutter Nix shell must use its Nix-provided pinned Rust toolchain." >&2
            echo "Remove or isolate the external rustup installation before continuing." >&2
            exit 1
          fi

          unset RUSTC
          unset RUSTC_WRAPPER
          unset RUSTC_WORKSPACE_WRAPPER

          echo "Stutter development environment"
          echo "Rust:        $(rustc --version)"
          echo "Cargo:       $(cargo --version)"
          echo "bpf-linker:  $(bpf-linker --version 2>/dev/null || true)"
          echo "cargo-deny:  $(cargo deny --version 2>/dev/null || true)"
        '';
      };
      };
    };
}
