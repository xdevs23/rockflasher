{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-24.11";
  };

  outputs = { nixpkgs,  ... }:
  let forEachSystem = nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed;
  in {
    devShell = forEachSystem (system:
      let pkgs = import nixpkgs { inherit system; };
      in pkgs.mkShell {
        buildInputs = with pkgs; [
          cargo rustc pkg-config openssl openssl.dev
        ];
        RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
      }
    );
  };
}