{
  pkgs ? import <nixpkgs> { },
}:
let
  package = (pkgs.callPackage ./default.nix { }).overrideAttrs (_: {
    # do not care about source changes, only package itself matters
    src = null;
  });
in
pkgs.mkShell {
  src = null;
  inputsFrom = [
    package
  ];
  packages = with pkgs; [
    clippy
    rustfmt
    rust-analyzer
  ];
  RUST_BACKTRACE = "full";
}
