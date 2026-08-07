{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flakelight.url = "github:nix-community/flakelight";
    flakelight.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      flakelight,
      self,
      ...
    }:
    flakelight ./. (
      { lib, outputs, ... }:
      {
        flakelight.builtinFormatters = false;
        systems = lib.systems.flakeExposed;
        package =
          pkgs:
          pkgs.callPackage ./default.nix {
            shortRev = self.shortRev or "dirty";
          };
        devShell = import ./shell.nix;
        checks = {
          format =
            pkgs:
            outputs.packages.${pkgs.stdenv.hostPlatform.system}.default.overrideAttrs (prev: {
              pname = "${prev.pname}-format";
              dontBuild = true;
              doCheck = true;
              nativeCheckInputs = [ pkgs.rustfmt ];
              checkPhase = "cargo fmt --check";
              installPhase = "touch $out";
            });
          clippy =
            pkgs:
            outputs.packages.${pkgs.stdenv.hostPlatform.system}.default.overrideAttrs (prev: {
              pname = "${prev.pname}-clippy";
              dontBuild = true;
              doCheck = true;
              nativeCheckInputs = [ pkgs.clippy ];
              checkPhase = "cargo clippy --all-targets -- -Dwarnings";
              installPhase = "touch $out";
            });
        };
      }
    );
}
