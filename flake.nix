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
  outputs = {
    self,
    rust-overlay,
    nixpkgs,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [rust-overlay.overlays.default];
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
      in {
        devShells.default = pkgs.mkShell {
          buildInputs = with pkgs; [
            rustToolchain
            # Build essentials
            pkg-config
            openssl
            openssl.dev
            # whisper-rs build essentials
            clang
            llvmPackages.libclang
            llvmPackages.bintools
            cmake
            # gcc
            # libiconv
            # Dev Tools
            cargo-audit
            cargo-edit
            cargo-watch
            cargo-expand
            cargo-flamegraph
            sqlx-cli
            jq
            # cargo-nextest
            # bacon
            just
            # cargo-tarpaulin
            # DB
            sqlite
            duckdb
            # postgresql
            # Audio
            alsa-lib
            go-jsonnet
          ];
          shellHook = ''
            export RUST_BACKTRACE=1
            export RUST_LOG=debug
            export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [pkgs.openssl pkgs.alsa-lib]}:$LD_LIBRARY_PATH"
            export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
            # export DATABASE_URL=""
            echo "Rust env has loaded!"
          '';
        };
      }
    );
}
