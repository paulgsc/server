{
  description = "Modular Rust + Whisper Manager Environment (CI-optimized)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-24.05";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
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
      };

      whisperManager = import ./nix/whisper-manager {
        inherit pkgs isCI;
      };

      # Combine inputs (whisper auto-excludes in CI)
      combinedBuildInputs =
        (rustEnv.buildInputs or [])
        ++ (whisperManager.packages or []);

      combinedShellHook = ''
        export CARGO_BUILD_JOBS=3
        ${rustEnv.shellHook or ""}
        ${whisperManager.shellHook or ""}
        ${
          if !isCI
          then ''echo "🦀 Rust + Whisper environment loaded!"''
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
