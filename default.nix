{
  lib,
  rustPlatform,
  shortRev ? "",
}:
let
  packageVersion = (fromTOML (builtins.readFile ./Cargo.toml)).package.version;
  version =
    if (builtins.stringLength shortRev) > 0 then "${packageVersion}-${shortRev}" else packageVersion;
in
rustPlatform.buildRustPackage {
  pname = "diffsoup";
  inherit version;

  src =
    let
      fs = lib.fileset;
    in
    fs.toSource {
      root = ./.;
      fileset = fs.unions [
        ./src
        ./Cargo.lock
        ./Cargo.toml
      ];
    };

  cargoLock = {
    lockFile = ./Cargo.lock;
  };

  meta.mainProgram = "diffsoup";
}
