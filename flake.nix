
{
    description = "My first Rust nixos dev env";

    inputs = {
        nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
        rust-overlay = {
            url = "github:oxalica/rust-overlay";
            inputs.nixpkgs.follows = "nixpkgs";
        };
        flake-utils.url = "github:numtide/flake-utils";
    };

    outputs = { self, rust-overlay, nixpkgs, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
        let
            overlays = [ rust-overlay.overlays.default ];
            pkgs = import nixpkgs {
                inherit system overlays;
            };

            rustToolchain = pkgs.rust-bin.stable.latest.default.override {
                extensions = [
                    "rust-src"
                    "rust-analyzer"
                    "clippy"
                ];
                targets = [
                    "x86_64-unknown-linux-gnu"
                    # "wasm32-unknown-unknown"
                ];
            };
        in
        {
            devShells.default = pkgs.mkShell {
                buildInputs = with pkgs; [
                    rustToolchain

                    # Build essentials
                    pkg-config
                    openssl
                    openssl.dev
                    # cmake
                    # gcc
                    # libiconv

                    # Dev Tools
                    rust-analyzer
                    cargo-audit
                    cargo-edit
                    cargo-watch
                    cargo-expand
                    cargo-flamegraph
                    sqlx-cli
                    # cargo-tarpaulin

                    # DB
                    sqlite
                    # postgresql

                    # Audio
                    alsa-lib

                ];

                shellHook = ''
                    export RUST_BACKTRACE=1
                    export RUST_LOG=debug
                    # export DATABASE_URL=""

                    echo "Rust env has loaded!"
                '';

                RUST_BACKTRACE = 1;
                RUST_LOG = "debug";
            };
        }
    );
}


