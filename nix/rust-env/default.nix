{
  pkgs,
  isCI ? false,
  # From the flake's `nixpkgs-sqlx` input, not `pkgs` — see flake.nix (#385).
  sqlx-cli,
  # Likewise: 3.9 or later, for `sops decrypt`, `encrypt` and `edit`.
  sops,
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
    sqlx-cli # need this in ci as well for migrations and prepare (0.8.6, see flake.nix)
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
  ]
  ++ [
    # Every deployment value is in deploy/<role>/<app>.sops.env (#613), which
    # make up, make dev and make import decrypt or write. The argument, not
    # pkgs.sops: see above.
    sops
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
      export RUST_LOG=info,async_nats=warn,orchestrator=info,file_host=debug,tabsched_pipeline=debug
      export LD_LIBRARY_PATH="${pkgs.lib.makeLibraryPath [pkgs.openssl pkgs.alsa-lib]}:$LD_LIBRARY_PATH"
      export LIBCLANG_PATH="${pkgs.llvmPackages.libclang.lib}/lib"
      echo "🦀 Rust development environment loaded!"
      # `tool <command>`: one command run with the role's host-side values for
      # tools run by hand (`tool sqlx migrate run --dry-run`), from
      # deploy/<role>/tools.sops.env, decrypted into that command's
      # environment only (#634). Nothing is exported into this shell, so no
      # role's value can reach `make dev` or anything else run here. The role
      # is $DEPLOY, home by default, as for `make up`; a variable this shell
      # sets itself wins (`DATABASE_URL=sqlite://x.db tool sqlx ...`).
      tool() {
        local repo
        repo="$(git rev-parse --show-toplevel 2>/dev/null)" || { echo "tool: not in a checkout" >&2; return 2; }
        cargo run -q --manifest-path "$repo/Cargo.toml" -p envctl -- --repo "$repo" run --role "''${DEPLOY:-home}" --app tools -- "$@"
      }
    '';
in {
  inherit buildInputs shellHook;

  shell = pkgs.mkShell {
    inherit buildInputs shellHook;
  };
}
