
# Nix Flake Architecture

This flake provides **three development shells** optimized for different use cases:

## Shell Comparison

| Shell       | Purpose              | Includes                                    |
|-------------|----------------------|---------------------------------------------|
| `default`   | Local development    | Rust + dev tools + Whisper + audio libs     |
| `rust`      | Rust-only work       | Rust + build deps + dev tools               |
| `ci`        | CI/CD pipelines      | **Minimal**: Rust compiler + build deps only|

## Usage

### Local Development (Full Environment)

```bash
# Enter full dev environment
nix develop

# Available commands:
# - cargo, rustc, clippy, rust-analyzer
# - cargo-audit, cargo-watch, cargo-expand, etc.
# - whisper fetch/prune/list
# - sqlx, just, jq, etc.
```

### Rust-Only Development

```bash
# Skip whisper manager
nix develop .#rust
```

### CI/CD (Minimal)

```bash
# Minimal environment - no dev tools, no whisper
nix develop .#ci --command cargo build --release

# In GitHub Actions:
# nix develop .#ci --command cargo build -p my-package
```

## What Gets Excluded in CI

The `.#ci` shell removes:

- ❌ Whisper manager (scripts, curl)
- ❌ Audio libraries (alsa-lib)
- ❌ Dev tooling (cargo-watch, cargo-audit, flamegraph, etc.)
- ❌ Database CLIs (sqlite, duckdb)
- ❌ Utilities (jq, just, jsonnet)
- ❌ IDE extensions (rust-analyzer, rust-src)
- ❌ Shell hooks and environment customization

This significantly reduces:
- Closure size (~500MB+ savings)
- Evaluation time
- Network bandwidth
- Cache storage

## CI Detection

The flake automatically detects `CI=true` environment variable:

```bash
# Automatic CI mode
export CI=true
nix develop  # Uses minimal buildInputs

# Manual CI shell
nix develop .#ci  # Explicitly uses minimal shell
```

## File Structure

```
.
├── flake.nix                 # Main flake with shell definitions
└── nix/
    ├── rust-env.nix          # Rust toolchain (CI-aware)
    └── whisper-manager/      # Whisper models (disabled in CI)
```

## Modifying Active Packages

### Add a Dev Tool (Local Only)

Edit `nix/rust-env.nix`:

```nix
devTools = with pkgs; [
  # ... existing tools
  your-new-tool
];
```

### Add a Build Dependency (Included in CI)

Edit `nix/rust-env.nix`:

```nix
coreBuildInputs = with pkgs; [
  # ... existing deps
  your-build-dep
];
```

### Change Active Whisper Models

Edit `nix/whisper-manager/default.nix`: name the model in `activeModels` and
give it a `pinnedModels` entry (URL and SHA-256), which is what `whisper fetch`
downloads. Multilingual only, never `.en`: file_host's transcriber refuses an
English-only model at startup.

```nix
activeModels = [
  "ggml-small-q5_1.bin"
  "ggml-silero-v5.1.2.bin"
];
```

## Verification

Check what's in each shell:

```bash
# See all packages in default shell
nix develop --command sh -c 'echo $buildInputs'

# See minimal CI packages
nix develop .#ci --command sh -c 'echo $buildInputs'

# Compare sizes
nix path-info -rsSh $(nix eval .#devShells.x86_64-linux.default --raw)
nix path-info -rsSh $(nix eval .#devShells.x86_64-linux.ci --raw)
```
