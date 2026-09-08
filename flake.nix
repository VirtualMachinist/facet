{
  description = "Facet M1 development shell with the Rust Turso native build dependencies";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/c25784012c9982bca5b3e0de87e90bbdac8927d3";
    rust-overlay = {
      url = "github:oxalica/rust-overlay/ca7f624be3935a5bc46d2c240515491ab8675503";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, rust-overlay, ... }:
    let
      systems = [ "aarch64-linux" "x86_64-linux" ];
      forSystems = nixpkgs.lib.genAttrs systems;
    in {
      devShells = forSystems (system:
        let
          pkgs = import nixpkgs { inherit system; overlays = [ rust-overlay.overlays.default ]; };
          rust = pkgs.rust-bin.stable."1.98.1".minimal.override {
            extensions = [ "rustfmt" "clippy" ];
          };
        in {
          default = pkgs.mkShell {
            packages = [ rust pkgs.cmake pkgs.pkg-config pkgs.git pkgs.python3 pkgs.openssl ];
            nativeBuildInputs = [ pkgs.rustPlatform.bindgenHook ];
            CARGO_BUILD_JOBS = "2";
            # Bound build space in the M1 guest; this is not a release profile.
            CARGO_PROFILE_DEV_DEBUG = "0";
            CARGO_PROFILE_TEST_DEBUG = "0";
          };
        });
    };
}
