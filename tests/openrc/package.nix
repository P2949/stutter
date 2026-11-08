# Opt-in test dependency, not a product service or host activation.
# nix build --no-link --print-out-paths --impure --file tests/openrc/package.nix
let
  pkgs = (builtins.getFlake "git+file://${toString ../..}").inputs.nixpkgs.legacyPackages.x86_64-linux;
in pkgs.stdenv.mkDerivation {
  pname = "stutter-openrc-fixture";
  version = "0.64.1";
  src = pkgs.fetchurl {
    name = "openrc-0.64.1.tar.gz";
    url = "https://codeload.github.com/OpenRC/openrc/tar.gz/refs/tags/0.64.1";
    hash = "sha256-VtbSTH/SyhXRq9H7gs5UMGKacayyw/oSdgMAJD29C6o=";
  };
  nativeBuildInputs = [ pkgs.meson pkgs.ninja pkgs.pkg-config ];
  buildInputs = [ pkgs.libcap ];
  # Upstream's install script requires DESTDIR to be set, even for a direct store install.
  preInstall = ''export DESTDIR=""'';
  mesonFlags = [
    "-Dsysconfdir=${placeholder "out"}/etc"
    "-Dpam=false" "-Daudit=disabled" "-Dselinux=disabled"
    "-Dnewnet=false" "-Dsysvinit=false"
    "-Dbash-completions=false" "-Dzsh-completions=false"
  ];
  # Upstream's full init tests are deliberately not run on the development host.
  doCheck = false;
}
