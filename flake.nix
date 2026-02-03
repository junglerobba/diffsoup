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
      { lib, ... }:
      {
        systems = lib.systems.flakeExposed;
        package =
          pkgs:
          pkgs.callPackage ./default.nix {
            shortRev = self.shortRev or "dirty";
          };
        devShell = import ./shell.nix;
      }
    );
}
