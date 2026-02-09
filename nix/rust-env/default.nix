{
  pkgs,
  isCI ? false,
}: let
  rustToolchain = pkgs.rust-bin.stable.latest.default.override {
    extensions =
      if isCI
      then []
      else ["rust-src" "rust-analyzer" "clippy"];
    targets = ["x86_64-unknown-linux-gnu"];
  };

  # Core dependencies needed for compilation
  coreBuildInputs = with pkgs; [
    rustToolchain
    pkg-config
    openssl
    openssl.dev
    clang
    llvmPackages.libclang
    llvmPackages.bintools
    cmake
    sqlx-cli # need this in ci as well for migrations and prepare
    sqlite # need this in ci as well for migrations and prepare
  ];

  # Development-only tooling (excluded from CI)
  devTools = with pkgs; [
    cargo-audit
    cargo-edit
    cargo-watch
    cargo-expand
    cargo-flamegraph
    jq
    just
    duckdb
    alsa-lib
    go-jsonnet
  ];

  buildInputs =
    coreBuildInputs
    ++ (
      if isCI
      then []
      else devTools
    );

  shellHook =
    if isCI
    then ""
    else ''
      export RUST_BACKTRACE=1
      export RUST_LOG=info,async_nats=warn,orchestrator=info,file_host=debug
      export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [pkgs.openssl pkgs.alsa-lib]}:$LD_LIBRARY_PATH"
      export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
      echo "🦀 Rust development environment loaded!"
    '';
in {
  inherit buildInputs shellHook;

  shell = pkgs.mkShell {
    inherit buildInputs shellHook;
  };
}
