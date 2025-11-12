{
  description = "Modular Rust + Whisper Manager Environment (cron-free self-cleaning)";

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

      # Import your submodules (each returns an attrset)
      rustEnv = import ./nix/rust-env {inherit pkgs;};
      whisperManager = import ./nix/whisper-manager {inherit pkgs;};

      # Combine their inputs
      combinedBuildInputs =
        (rustEnv.buildInputs or [])
        ++ (whisperManager.packages or []);

      combinedShellHook = ''
        ${rustEnv.shellHook or ""}
        ${whisperManager.shellHook or ""}
        echo "🦀 Rust + Whisper environment loaded!"
      '';
    in {
      # Default shell = combined environment
      devShells.default = pkgs.mkShell {
        buildInputs = combinedBuildInputs;
        shellHook = combinedShellHook;
      };

      # Individual shells for modular use
      devShells.rust = rustEnv.shell;
      devShells.whisper = whisperManager.shell;

      # For consistent formatting
      formatter = pkgs.alejandra;
    });
}
