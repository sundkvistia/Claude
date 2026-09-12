{
  description = "Rust -> Wasm sketch app (draw with mouse/touch, pan & zoom)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };

        rustToolchain = pkgs.rust-bin.stable.latest.default.override {
          targets = [ "wasm32-unknown-unknown" ];
        };
      in
      {
        devShells.default = pkgs.mkShell {
          packages = [
            rustToolchain
            pkgs.trunk
            pkgs.wasm-bindgen-cli
            pkgs.binaryen # provides wasm-opt, used by trunk's release builds
            pkgs.pkg-config
          ];

          # trunk shells out to `wasm-opt`; make sure it can find it even if
          # something upstream doesn't already put it on PATH.
          shellHook = ''
            export PATH="${pkgs.binaryen}/bin:$PATH"
          '';
        };
      });
}
