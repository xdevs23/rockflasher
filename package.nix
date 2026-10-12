{ lib, rustPlatform }:

let
  cargoPackage = (lib.importTOML ./Cargo.toml).package;
in
rustPlatform.buildRustPackage {
  pname = cargoPackage.name;
  inherit (cargoPackage) version;
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./src ];
  };
  cargoLock.lockFile = ./Cargo.lock;
  meta = {
    inherit (cargoPackage) description;
    license = lib.licenses.mit;
    mainProgram = "rockflasher";
  };
}
