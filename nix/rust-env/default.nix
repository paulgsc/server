{ pkgs }:

let
  rustToolchain = pkgs.rust-bin.stable.latest.default.override {
    extensions = [ "rust-src" "rust-analyzer" "clippy" ];
    targets = [ "x86_64-unknown-linux-gnu" ];
  };

  buildInputs = with pkgs; [
    rustToolchain
    pkg-config
    openssl
    openssl.dev
    clang
    llvmPackages.libclang
    llvmPackages.bintools
    cmake
    cargo-audit
    cargo-edit
    cargo-watch
    cargo-expand
    cargo-flamegraph
    sqlx-cli
    jq
    just
    sqlite
    duckdb
    alsa-lib
    go-jsonnet
  ];

  shellHook = ''
    export RUST_BACKTRACE=1
    export RUST_LOG=debug
    export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [ pkgs.openssl pkgs.alsa-lib ]}:$LD_LIBRARY_PATH"
    export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
    echo "🦀 Rust environment loaded!"
  '';
in {
  inherit buildInputs shellHook;

  shell = pkgs.mkShell {
    inherit buildInputs shellHook;
  };
}
