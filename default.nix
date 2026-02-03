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

  src = lib.cleanSource ./.;
  cargoLock = {
    lockFile = ./Cargo.lock;
  };

  meta.mainProgram = "diffsoup";
}
