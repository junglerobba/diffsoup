{
  lib,
  rustPlatform,
  shortRev ? "",
  git,
  jujutsu,
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

  preCheck = ''
    export XDG_CONFIG_HOME=$(mktemp -d)
  '';

  nativeCheckInputs = [
    git
    jujutsu
  ];

  meta.mainProgram = "diffsoup";
}
