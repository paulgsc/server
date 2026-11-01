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
      # The role's host-side values for tools run by hand (`sqlx migrate run`),
      # from deploy/<role>/tools.sops.env, decrypted into this shell's
      # environment only (#634). The role is $DEPLOY, home by default, as for
      # `make up`; DEPLOY=dev for a scratch database. ENVCTL_NO_EXPORT=1 skips
      # it. The shell's own variables win, and only names are printed.
      envctl_repo="$(git rev-parse --show-toplevel 2>/dev/null)"
      envctl_role="''${DEPLOY:-home}"
      envctl_tools="$envctl_repo/deploy/$envctl_role/tools.sops.env"
      if [ -z "''${ENVCTL_NO_EXPORT:-}" ] && [ -n "$envctl_repo" ] && [ -d "$envctl_repo/apps/envctl" ]; then
        if [ -f "$envctl_tools" ]; then
          [ -x "$envctl_repo/target/debug/envctl" ] || echo "envctl: building it once to read deploy/$envctl_role/tools.sops.env (about a minute)"
          if envctl_out="$(cargo run -q --manifest-path "$envctl_repo/Cargo.toml" -p envctl -- --repo "$envctl_repo" export --role "$envctl_role")"; then
            eval "$envctl_out"
          else
            echo "envctl: deploy/$envctl_role/tools.sops.env not exported (above); the shell opens without it"
          fi
          unset envctl_out
        else
          echo "envctl: no deploy/$envctl_role/tools.sops.env, so tools get no role values (sops edit deploy/$envctl_role/tools.sops.env)"
        fi
      fi
      unset envctl_repo envctl_role envctl_tools
    '';
in {
  inherit buildInputs shellHook;

  shell = pkgs.mkShell {
    inherit buildInputs shellHook;
  };
}
