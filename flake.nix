{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flakelight.url = "github:nix-community/flakelight";
    flakelight.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    {
      flakelight,
      ...
    }:
    flakelight ./. (
      { lib, ... }:
      {
        systems = lib.systems.flakeExposed;
        package = import ./default.nix;
        devShell = import ./shell.nix;
      }
    );
}
