{
  lib,
  rustPlatform,
}:
rustPlatform.buildRustPackage {
  pname = "diffsoup";
  version = "0.1.0";

  src = lib.cleanSource ./.;
  cargoLock = {
    lockFile = ./Cargo.lock;
  };

  meta.mainProgram = "diffsoup";
}
