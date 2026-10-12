{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
  let
    inherit (nixpkgs) lib;
    linuxSystems = lib.filter (system: (lib.systems.elaborate system).isLinux) lib.systems.flakeExposed;
    forEachSystem = f: lib.genAttrs linuxSystems (system: f nixpkgs.legacyPackages.${system});
  in {
    packages = forEachSystem (pkgs: {
      default = pkgs.callPackage ./package.nix { };
    });

    devShells = forEachSystem (pkgs: {
      default = pkgs.mkShell {
        inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.default ];
        packages = [ pkgs.clippy ];
        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      };
    });
  };
}
