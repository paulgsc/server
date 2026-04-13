{
  pkgs,
  isCI ? false,
}:
if isCI
then {
  packages = [];
  shellHook = "";
  shell = pkgs.mkShell {};
}
else let
  # ── Declared intent ────────────────────────────────────────────────────────
  # Add / remove model tags here to keep 'llm status' accurate.
  activeModels = ["llama3.2:1b" "nomic-embed-text"];

  # Host-side path where Ollama writes model data (via bind mount).
  # Must match the compose volume source.
  modelsDir = let
    envPath = builtins.getEnv "OLLAMA_MODELS_PATH";
  in
    if envPath != ""
    then envPath
    else "/mnt/storage/users/dev/models/ollama-models";

  # ── Guard: verify container is wired correctly ──────────────────────────────
  # Fails fast if OLLAMA_MODELS inside the container doesn't point at the mount.
  llmGuard = pkgs.writeShellScriptBin "llm-guard" ''
    set -euo pipefail
    EXPECTED="/root/.ollama/models"
    ACTUAL=$(docker exec tabsched-ollama sh -c 'echo $OLLAMA_MODELS' 2>/dev/null || echo "")

    if [ -z "$ACTUAL" ]; then
      echo "❌  Container not running or OLLAMA_MODELS unset inside container."
      echo "    Run 'pipeline up' first, or check your compose env."
      exit 1
    fi

    if [ "$ACTUAL" != "$EXPECTED" ]; then
      echo "❌  OLLAMA_MODELS path mismatch inside container."
      echo "    Expected: $EXPECTED"
      echo "    Got:      $ACTUAL"
      echo "    Models may be leaking to an unexpected location."
      exit 1
    fi

    echo "✅  Container storage path verified: $ACTUAL"
  '';

  # ── Retention graph inspector (read-only, no side effects) ──────────────────
  # Compares declared activeModels against what Ollama actually has on disk.
  # Parses manifest → blob mappings to estimate exclusive vs shared blob sizes,
  # so you know exactly how much disk a removal would reclaim.
  llmStatus = pkgs.writeShellScriptBin "llm-status" ''
    set -euo pipefail

    MODELS_DIR="${modelsDir}"
    MANIFESTS_DIR="$MODELS_DIR/manifests"
    BLOBS_DIR="$MODELS_DIR/blobs"
    ACTIVE="${builtins.concatStringsSep " " activeModels}"

    echo ""
    echo "📦  Ollama Model Retention Graph"
    echo "════════════════════════════════════════"
    echo ""

    # ── Declared active set ──
    echo "Declared active (in Nix registry):"
    for m in $ACTIVE; do
      echo "  ✓  $m"
    done
    echo ""

    # ── Container-reported list ──
    if ! docker ps --format '{{.Names}}' | grep -q "tabsched-ollama"; then
      echo "⚠   Container not running — showing disk-only analysis."
      INSTALLED=""
    else
      INSTALLED=$(docker exec tabsched-ollama ollama list 2>/dev/null | awk 'NR>1 {print $1}')
    fi

    if [ -n "$INSTALLED" ]; then
      echo "Installed (per ollama list):"
      for model in $INSTALLED; do
        if echo "$ACTIVE" | grep -qw "$model"; then
          echo "  ✓  $model  [ACTIVE]"
        else
          echo "  ✗  $model  [UNUSED]"
        fi
      done
      echo ""
    fi

    # ── Blob-level retention graph ──
    echo "Blob retention graph:"
    echo "────────────────────────────────────────"

    if [ ! -d "$MANIFESTS_DIR" ] || [ ! -d "$BLOBS_DIR" ]; then
      echo "  ⚠  Cannot find manifests or blobs dir at $MODELS_DIR"
      echo "     (host path may be wrong or models not yet pulled)"
    else
      # Build blob→models mapping using associative arrays
      declare -A blob_to_models
      declare -A model_to_blobs

      for manifest_file in $(find "$MANIFESTS_DIR" -type f 2>/dev/null); do
        # Derive a friendly model name from the manifest path
        # manifests/<registry>/<name>/<tag>  →  name:tag
        rel=$(echo "$manifest_file" | sed "s|$MANIFESTS_DIR/||")
        parts=$(echo "$rel" | tr '/' '\n' | tail -2)
        model_name=$(echo "$parts" | head -1)
        model_tag=$(echo "$parts" | tail -1)
        model_key="$model_name:$model_tag"

        blobs=$(${pkgs.jq}/bin/jq -r '.. | objects | select(has("digest")) | .digest' "$manifest_file" 2>/dev/null | sort -u)

        for b in $blobs; do
          # Strip sha256: prefix for filename lookup
          blob_file=$(echo "$b" | sed 's|sha256:|sha256-|')
          model_to_blobs["$model_key"]+="$blob_file "
          blob_to_models["$blob_file"]+="$model_key "
        done
      done

      TOTAL_RECLAIMABLE=0

      for model in "''${!model_to_blobs[@]}"; do
        exclusive_bytes=0
        shared_bytes=0
        shared_with=""

        for blob in ''${model_to_blobs[$model]}; do
          blob_path="$BLOBS_DIR/$blob"
          size=$(stat -c%s "$blob_path" 2>/dev/null || echo 0)
          users="''${blob_to_models[$blob]:-}"
          count=$(echo "$users" | wc -w)

          if [ "$count" -gt 1 ]; then
            shared_bytes=$((shared_bytes + size))
            for u in $users; do
              if [ "$u" != "$model" ] && ! echo "$shared_with" | grep -q "$u"; then
                shared_with="$shared_with $u"
              fi
            done
          else
            exclusive_bytes=$((exclusive_bytes + size))
          fi
        done

        exclusive_mb=$(echo "scale=1; $exclusive_bytes / 1048576" | ${pkgs.bc}/bin/bc)
        shared_mb=$(echo "scale=1; $shared_bytes / 1048576" | ${pkgs.bc}/bin/bc)

        if echo "$ACTIVE" | grep -qw "$model"; then
          status="ACTIVE"
        else
          status="UNUSED"
          TOTAL_RECLAIMABLE=$((TOTAL_RECLAIMABLE + exclusive_bytes))
        fi

        echo ""
        echo "  Model: $model"
        echo "  Status:    $status"
        echo "  Exclusive: ''${exclusive_mb} MB  (freed if removed)"
        if [ -n "$shared_with" ]; then
          echo "  Shared:    ''${shared_mb} MB  (shared with:$shared_with)"
        fi
        if [ "$status" = "UNUSED" ]; then
          echo "  ⚠  Removable — not in declared active set"
        fi
      done

      echo ""
      echo "────────────────────────────────────────"
      reclaimable_mb=$(echo "scale=1; $TOTAL_RECLAIMABLE / 1048576" | ${pkgs.bc}/bin/bc)
      echo "  Reclaimable (unused exclusive blobs): ''${reclaimable_mb} MB"
    fi

    # ── Raw disk usage ──
    echo ""
    echo "Disk usage (host path):"
    echo "  $MODELS_DIR"
    du -sh "$MODELS_DIR" 2>/dev/null | awk '{print "  " $1}'

    # ── Shadow storage check ──
    echo ""
    echo "Shadow storage check (unexpected write locations):"
    if [ -d "$HOME/.ollama" ]; then
      size=$(du -sh "$HOME/.ollama" 2>/dev/null | awk '{print $1}')
      echo "  ⚠  ~/.ollama exists ($size) — models may have leaked before bind mount was configured"
    else
      echo "  ✓  ~/.ollama not present on host"
    fi

    # ── Manual removal reminder ──
    echo ""
    echo "To remove an unused model (no auto-prune — intentional):"
    echo "  docker exec tabsched-ollama ollama rm <model>"
    echo ""
  '';

  # ── Gated downloader (with guard pre-check) ────────────────────────────────
  llmDownload = pkgs.writeShellScriptBin "llm-download" ''
    set -euo pipefail

    # Verify storage path before writing anything
    ${llmGuard}/bin/llm-guard

    echo ""
    echo -e "\033[0;34m📥  Ollama Model Downloader\033[0m"
    echo ""
    echo "  1. llama3.2:1b      (1.3 GB)  — Smallest/Fastest LLM"
    echo "  2. llama3.2:3b      (2.0 GB)  — Balanced LLM"
    echo "  3. nomic-embed-text (274 MB)  — Required for RAG/Embeddings"
    echo ""
    read -p "Select model [1]: " CHOICE
    CHOICE=''${CHOICE:-1}

    case $CHOICE in
      1) MODEL="llama3.2:1b" ;;
      2) MODEL="llama3.2:3b" ;;
      3) MODEL="nomic-embed-text" ;;
      *) echo "Invalid choice"; exit 1 ;;
    esac

    echo ""
    echo "Pulling $MODEL..."
    docker exec -it tabsched-ollama ollama pull "$MODEL"
    echo ""
    echo "Done. Run 'llm status' to see updated disk usage."
  '';

  # ── CLI dispatcher ─────────────────────────────────────────────────────────
  llmCLI = pkgs.writeShellScriptBin "llm" ''
    case "''${1:-help}" in
      download) ${llmDownload}/bin/llm-download ;;
      list)     docker exec -it tabsched-ollama ollama list ;;
      status)   ${llmStatus}/bin/llm-status ;;
      guard)    ${llmGuard}/bin/llm-guard ;;
      *)
        echo ""
        echo "Usage: llm <command>"
        echo ""
        echo "  download   Pull a model (with storage guard pre-check)"
        echo "  list       List installed models (raw ollama list)"
        echo "  status     Retention graph: disk usage, active vs unused, reclaimable bytes"
        echo "  guard      Verify container storage path is correctly wired"
        echo ""
        ;;
    esac
  '';
in {
  packages = [llmCLI pkgs.docker-compose];
  shell = pkgs.mkShell {
    buildInputs = [llmCLI pkgs.docker-compose];
  };
  shellHook = ''
    export OLLAMA_MODELS_PATH="${modelsDir}"
    echo "🤖  LLM Manager loaded — commands: llm download | list | status | guard"
  '';
}
