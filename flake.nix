{
  description = "Modular Rust + Whisper Manager Environment (CI-optimized)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";
    # sqlx-cli only (#385). nixos-24.05 ships 0.7.4, but the workspace is on
    # sqlx 0.8.6 (#351), and `sqlx prepare` must match the macros' version.
    # A second input, rather than moving `nixpkgs`, keeps every other tool in
    # the devshell where it is. nixos-26.05's sqlx-cli is exactly 0.8.6 and
    # prebuilt on cache.nixos.org; prepare-sqlx asserts the version, so a
    # `nix flake update` that moves it fails CI instead of drifting.
    nixpkgs-sqlx.url = "github:NixOS/nixpkgs/nixos-26.05";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    nixpkgs-sqlx,
    rust-overlay,
    flake-utils,
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      overlays = [rust-overlay.overlays.default];
      pkgs = import nixpkgs {inherit system overlays;};

      # Detect CI environment
      isCI = builtins.getEnv "CI" == "true";

      # Import submodules with CI flag
      rustEnv = import ./nix/rust-env {
        inherit pkgs isCI;
        sqlx-cli = nixpkgs-sqlx.legacyPackages.${system}.sqlx-cli;
      };

      whisperManager = import ./nix/whisper-manager {
        inherit pkgs isCI;
      };

      llmManager = import ./nix/llm-manager {
        inherit pkgs isCI;
      };

      # Combine inputs (whisper auto-excludes in CI)
      combinedBuildInputs =
        (rustEnv.buildInputs or [])
        ++ (whisperManager.packages or [])
        ++ (llmManager.packages or []);

      combinedShellHook = ''
        export CARGO_BUILD_JOBS=3
        ${rustEnv.shellHook or ""}
        ${whisperManager.shellHook or ""}
        ${llmManager.shellHook or ""}
        ${
          if !isCI
          then ''echo "🦀 Rust + Whisper  + LLM environment loaded!"''
          else ""
        }
      '';
    in {
      # Default shell = full local development environment
      devShells.default = pkgs.mkShell {
        buildInputs = combinedBuildInputs;
        shellHook = combinedShellHook;
      };

      # Individual shells for modular use
      devShells.rust = rustEnv.shell;
      devShells.whisper = whisperManager.shell;
      devShells.llm = llmManager.shell;

      # Minimal CI shell - no dev tools, no whisper, no hooks
      devShells.ci = pkgs.mkShell {
        buildInputs = rustEnv.buildInputs;
        shellHook = ''
          export CARGO_BUILD_JOBS=3
        '';
      };

      # For consistent formatting
      formatter = pkgs.alejandra;
    });
}
