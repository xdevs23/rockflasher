{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, advisory-db }:
  let
    inherit (nixpkgs) lib;
    linuxSystems = lib.filter (system: (lib.systems.elaborate system).isLinux) lib.systems.flakeExposed;
    forEachSystem = f: lib.genAttrs linuxSystems (system: f nixpkgs.legacyPackages.${system});
  in {
    packages = forEachSystem (pkgs: {
      default = pkgs.callPackage ./package.nix { };
    });

    checks = forEachSystem (pkgs:
    let
      package = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
    in {
      inherit package;

      licenses = pkgs.stdenvNoCC.mkDerivation {
        name = "${package.pname}-licenses";
        inherit (package) src cargoDeps;
        nativeBuildInputs = [ pkgs.cargo pkgs.cargo-deny pkgs.rustc pkgs.rustPlatform.cargoSetupHook ];
        buildPhase = ''
          export HOME=$TMPDIR
          cargo deny --frozen --config ${./deny.toml} check licenses
        '';
        installPhase = "touch $out";
      };

      advisories = pkgs.runCommand "${package.pname}-advisories" { nativeBuildInputs = [ pkgs.cargo-audit ]; } ''
        export HOME=$TMPDIR
        cargo-audit audit --no-fetch --no-yanked --db ${advisory-db} --file ${./Cargo.lock}
        touch $out
      '';
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
