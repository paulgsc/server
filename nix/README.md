# Nix Development Environment

> **Philosophy**: No cron jobs. No background daemons. No active memory burden.  
> Just declarative, self-pruning infrastructure that doesn't rot.

---

## Structure

```
nix/
├── README.md              # You are here
├── rust-env.nix           # Rust toolchain + build deps
└── whisper-manager.nix    # ML model lifecycle management
```

Each module is **self-contained** and **composable**. The root `flake.nix` just wires them together.

---

## Quick Start

### Enter the default environment (Rust + Whisper)
```bash
nix develop
```

You now have:
- Full Rust toolchain (stable + rust-analyzer + clippy)
- Whisper model management CLI
- Build essentials (OpenSSL, ALSA, LLVM, etc.)

### Use specialized shells

```bash
# Rust-only (lighter, no ML tools)
nix develop .#rust

# Whisper-only (model management without Rust overhead)
nix develop .#whisper
```

---

## Modules

### `rust-env.nix` – Rust Toolchain

**What it provides:**
- `rustc`, `cargo`, `rust-analyzer`, `clippy`
- Build tools: `pkg-config`, `cmake`, `clang`
- Database CLIs: `sqlite`, `duckdb`, `sqlx-cli`
- Dev utils: `cargo-watch`, `cargo-audit`, `just`, `jq`
- Audio libs: `alsa-lib`

**Configuration:**
Edit targets/extensions directly in the file:
```nix
targets = [
  "x86_64-unknown-linux-gnu"
  # "wasm32-unknown-unknown"  # Uncomment for WASM
];
```

---

### `whisper-manager.nix` – ML Model Lifecycle

**Problem it solves:**  
You download 5 Whisper models, forget which ones you need, run out of disk space, and have no idea what to delete.

**Solution:**  
Declarative model registry. Only models in `activeModels` are kept. Everything else gets pruned automatically.

#### Configuration

Edit the `activeModels` list in `whisper-manager.nix`:

```nix
activeModels = [
  "ggml-base.en-q5_1.bin"        # Keep this
  # "ggml-small.en-q5_1.bin"     # Commented out = will be pruned
];
```

#### Commands

```bash
whisper download   # Interactive model download (auto-prunes unused first)
whisper prune      # Remove models not in activeModels list
whisper list       # Show active vs downloaded models + disk usage
```

#### Design Principles

1. **Reactive, not scheduled**: Pruning happens before downloads, not on a timer
2. **Single source of truth**: Your Nix file IS your inventory
3. **Zero daemons**: No cron, no systemd timers, no background processes
4. **Self-documenting**: `whisper list` always shows ground truth

#### Workflow Example

```bash
# Download a model
$ whisper download
[Interactive menu appears]
Select model (1-4): 3
⬇️  Downloading ggml-small.en-q5_1.bin...
✅ Downloaded (240MB)

⚠️  Add to activeModels in nix/whisper-manager.nix:
   activeModels = [ "ggml-small.en-q5_1.bin" ];

# Edit nix/whisper-manager.nix to register it
# (Otherwise it'll be pruned next time)

# Later, decide you don't need it
# Just comment it out:
activeModels = [
  "ggml-base.en-q5_1.bin"
  # "ggml-small.en-q5_1.bin"  # Don't need anymore
];

# Next download or prune operation removes it
$ whisper download
🧹 Pruning models not in active registry...
🗑️  Removing unused: ggml-small.en-q5_1.bin
✅ Pruning complete
```

---

## Adding New Modules

Want to add a Python environment, Docker tools, or Postgres?

1. **Create a new module** (e.g., `nix/python-env.nix`)
2. **Import it in root `flake.nix`:**
   ```nix
   pythonEnv = import ./nix/python-env.nix { inherit pkgs; };
   ```
3. **Add to buildInputs or create a new shell:**
   ```nix
   devShells.python = pkgs.mkShell {
     buildInputs = pythonEnv.packages;
     shellHook = pythonEnv.shellHook;
   };
   ```

**Rule of thumb:**  
If it's >30 lines or has its own config, it gets its own module.

---

## Why This Structure?

### Traditional approach (god-file):
```nix
# flake.nix (500 lines)
# - Rust stuff
# - Python stuff  
# - Docker stuff
# - ML models
# - Database migrations
# - ...you get the idea
```
❌ Hard to read  
❌ Merge conflicts  
❌ Can't reuse across projects  
❌ No separation of concerns  

### Modular approach (this repo):
```nix
# flake.nix (30 lines - just composition)
# nix/rust-env.nix (focused, reusable)
# nix/whisper-manager.nix (focused, reusable)
```
✅ Clear responsibilities  
✅ Easy git history  
✅ Copy modules to other projects  
✅ Change Rust config without touching ML config  

---

## Philosophy: No Active Memory Burden

Traditional systems require you to remember:
- "Did I set up a cron job for model cleanup?"
- "Which models am I using again?"
- "Is there a systemd timer I forgot about?"

This Nix setup requires you to remember **nothing**.

Want to know what models you're using?  
→ Look at `activeModels` in `whisper-manager.nix`

Want to know what's installed?  
→ `nix flake show` or check the module files

Want to clean up unused models?  
→ Don't. They self-prune on next operation.

**Your flake is your memory. Your git history is your audit log.**

---

## Debugging

### Shell not loading?
```bash
# Check flake syntax
nix flake check

# Verbose output
nix develop --print-build-logs
```

### Module import failing?
```bash
# Ensure paths are correct (relative to flake.nix)
# Should be: import ./nix/module-name.nix
```

### Whisper models not pruning?
```bash
# Run manually to see output
whisper-prune

# Check what's active
whisper list
```

---

## Related Docs

- [Root README](../README.md) – Project overview
- [System Architecture](../docs/system_design/) – Service topology
- [whisper.cpp repo](https://github.com/ggerganov/whisper.cpp) – Model source

---

## Contributing to Nix Modules

When adding new functionality:

1. **Keep modules focused** – one concern per file
2. **Export a shell** – makes testing easy (`nix develop .#yourmodule`)
3. **Document config options** – future you will thank present you
4. **No side effects** – modules should be pure functions of `{ pkgs }`

**Good module structure:**
```nix
{ pkgs }:
let
  # Configuration at the top
  config = { ... };
  
  # Tools/scripts
  myTool = pkgs.writeShellScriptBin "tool" ''...'';
in
{
  packages = [ myTool ];
  shellHook = ''...'';
  shell = pkgs.mkShell { ... };
}
```

---

## Known Issues

- **Model downloads require internet** – obviously, but worth stating
- **Disk space checks are advisory** – won't stop you from filling /
- **No integrity checking** – we trust Hugging Face's CDN
- **Concurrent prunes might race** – don't run `whisper prune` in parallel

None of these are critical for a dev environment. If you need production robustness, consider adding checksums and locking.

---
