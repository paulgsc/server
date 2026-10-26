{
  pkgs,
  isCI ? false,
}:
# Whisper manager is completely disabled in CI environments
if isCI
then {
  packages = [];
  shellHook = "";
  shell = pkgs.mkShell {};
}
else let
  # ===== CONFIGURATION (edit this) =====
  # What file_host's voice-note transcriber reads (`file_host::transcribe`,
  # paulgsc/server#462), mounted into its container at /models by
  # infra/compose/file_host.yml. `whisper fetch` downloads exactly these.
  # Multilingual, not `.en`: the notes mix Korean into English, and the
  # transcriber refuses an English-only model at startup.
  activeModels = [
    "ggml-small-q5_1.bin"
    "ggml-silero-v5.1.2.bin"
  ];

  # Each active model's source and SHA-256 (Hugging Face's own, from the
  # file's `X-Linked-ETag`), so a fetch keeps only the bytes it expects.
  pinnedModels = [
    {
      name = "ggml-small-q5_1.bin";
      url = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small-q5_1.bin";
      sha256 = "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb";
      size = "190MB";
    }
    {
      name = "ggml-silero-v5.1.2.bin";
      url = "https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin";
      sha256 = "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf";
      size = "1MB";
    }
  ];

  # Where the models go: the directory infra/compose/file_host.yml mounts at
  # /models, and its HOST-SPECIFIC default (#406). TRANSCRIBE_MODELS_DIR, the
  # variable that mount reads, overrides it here too, but only when exported
  # and nix evaluates impurely (`nix develop --impure`): a pure evaluation
  # reads every variable as unset.
  modelsDir = let
    envPath = builtins.getEnv "TRANSCRIBE_MODELS_DIR";
  in
    if envPath != ""
    then envPath
    else "/mnt/storage/users/dev/models/whisper-models";
  # ======================================
  pruneUnusedModels = pkgs.writeShellScriptBin "whisper-prune" ''
    set -euo pipefail
    MODELS_DIR="${modelsDir}"

    echo "🧹 Pruning models not in active registry..."

    if [ ! -d "$MODELS_DIR" ]; then
      echo "✓ No models directory exists yet"
      exit 0
    fi

    cd "$MODELS_DIR"
    shopt -s nullglob
    for model in ggml-*.bin; do
      KEEP=false
      ${builtins.concatStringsSep "\n" (map (m: ''
        if [ "$model" = "${m}" ]; then
          KEEP=true
        fi
      '')
      activeModels)}

      if [ "$KEEP" = "false" ]; then
        echo "🗑️  Removing unused: $model"
        rm -f "$model"
      fi
    done

    echo "✅ Pruning complete"
  '';

  # Non-interactive: every pinned model, verified, skipping what is already
  # there with the right hash. Prunes nothing.
  fetchModels = pkgs.writeShellScriptBin "whisper-fetch" ''
    set -euo pipefail
    MODELS_DIR="${modelsDir}"
    mkdir -p "$MODELS_DIR"
    ${builtins.concatStringsSep "\n" (map (m: ''
        if [ -f "$MODELS_DIR/${m.name}" ] && echo "${m.sha256}  $MODELS_DIR/${m.name}" | ${pkgs.coreutils}/bin/sha256sum --check --status; then
          echo "✓ ${m.name} already there"
        else
          echo "⬇️  ${m.name} (${m.size})"
          PART="$MODELS_DIR/${m.name}.part"
          ${pkgs.curl}/bin/curl -fL --progress-bar -o "$PART" "${m.url}"
          if ! echo "${m.sha256}  $PART" | ${pkgs.coreutils}/bin/sha256sum --check --status; then
            rm -f "$PART"
            echo "❌ ${m.name}: SHA-256 does not match; nothing kept" >&2
            exit 1
          fi
          chmod 444 "$PART"
          rm -f "$MODELS_DIR/${m.name}"
          mv "$PART" "$MODELS_DIR/${m.name}"
          echo "✅ ${m.name}"
        fi
      '')
      pinnedModels)}
    echo ""
    echo "Models are in $MODELS_DIR. file_host reads them from /models when"
    echo "TRANSCRIBE_ENABLED=true (infra/compose/file_host.yml mounts this directory)."
  '';

  listModels = pkgs.writeShellScriptBin "whisper-list" ''
    set -euo pipefail

    # Colors
    GREEN='\033[0;32m'
    BLUE='\033[0;34m'
    NC='\033[0m'

    MODELS_DIR="${modelsDir}"

    echo -e "''${BLUE}📋 Whisper Models Status''${NC}"
    echo ""
    echo -e "''${GREEN}Active (in nix/whisper-manager/default.nix):''${NC}"
    ${builtins.concatStringsSep "\n" (map (m: ''echo "  ✓ ${m}"'') activeModels)}
    echo ""

    if [ -d "$MODELS_DIR" ]; then
      echo "Downloaded (on disk):"
      cd "$MODELS_DIR"
      shopt -s nullglob
      for model in ggml-*.bin; do
        SIZE=$(${pkgs.coreutils}/bin/du -h "$model" | cut -f1)
        echo "  • $model ($SIZE)"
      done
    else
      echo "No models downloaded yet"
    fi

    echo ""
    FREE_GB=$(df -BG "${modelsDir}" 2>/dev/null | awk 'NR==2 {gsub("G","",$4); print $4}' || echo "?")
    echo "💾 Free space: ''${FREE_GB}GB"
  '';

  whisperCLI = pkgs.writeShellScriptBin "whisper" ''
    case "''${1:-help}" in
      fetch) ${fetchModels}/bin/whisper-fetch ;;
      prune) ${pruneUnusedModels}/bin/whisper-prune ;;
      list)  ${listModels}/bin/whisper-list ;;
      # Retired (#476): an interactive menu of English-only models the
      # transcriber refuses, without the VAD model, that pruned first.
      download)
        echo "whisper download is retired: run 'whisper fetch' for the transcriber's models" >&2
        exit 1
        ;;
      *)
        echo "Whisper Model Manager"
        echo ""
        echo "Usage:"
        echo "  whisper fetch  - Download the transcriber's models, verified"
        echo "  whisper prune  - Remove models not in activeModels list"
        echo "  whisper list   - Show active vs downloaded models"
        ;;
    esac
  '';

  packagesList = [
    whisperCLI
    fetchModels
    pruneUnusedModels
    listModels
    pkgs.curl
  ];

  # Exports nothing: file_host warns at startup about the retired
  # WHISPER_MODELS_PATH (#476), and this hook runs in the default shell too.
  hookScript = ''
    echo "🎯 Whisper Manager loaded"
    echo "   Commands: whisper {fetch|prune|list}"
    echo "   Models: ${modelsDir}"
  '';
in {
  packages = packagesList;

  # Export individual commands for direct access
  whisper-cli = whisperCLI;
  whisper-fetch = fetchModels;
  whisper-prune = pruneUnusedModels;
  whisper-list = listModels;

  shellHook = hookScript;

  # Standalone shell
  shell = pkgs.mkShell {
    buildInputs = packagesList;
    shellHook = hookScript;
  };
}
