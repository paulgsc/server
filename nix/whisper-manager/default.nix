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

  modelsDir = let
    envPath = builtins.getEnv "WHISPER_MODELS_PATH";
  in
    if envPath != ""
    then envPath
    else "/mnt/storage/users/dev/models/whisper-models";
  # ======================================

  # Model metadata for download script
  modelInfo = {
    "1" = {
      name = "ggml-tiny.en-q5_1.bin";
      size = "30MB";
      threads = "2";
      buffer = "2";
    };
    "2" = {
      name = "ggml-base.en-q5_1.bin";
      size = "60MB";
      threads = "2";
      buffer = "3";
    };
    "3" = {
      name = "ggml-small.en-q5_1.bin";
      size = "240MB";
      threads = "3";
      buffer = "4";
    };
    "4" = {
      name = "ggml-medium.en-q5_1.bin";
      size = "760MB";
      threads = "3";
      buffer = "5";
    };
  };

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

  downloadModel = pkgs.writeShellScriptBin "whisper-download" ''
    set -euo pipefail

    # Colors
    GREEN='\033[0;32m'
    BLUE='\033[0;34m'
    YELLOW='\033[1;33m'
    RED='\033[0;31m'
    NC='\033[0m'

    MODELS_DIR="${modelsDir}"
    BASE_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main"

    # Auto-prune before download
    ${pruneUnusedModels}/bin/whisper-prune

    echo -e "''${BLUE}📥 Whisper Model Downloader''${NC}"
    echo ""
    echo "Active models in nix/whisper-manager.nix:"
    ${builtins.concatStringsSep "\n" (map (m: ''echo -e "  ✓ ${m}"'') activeModels)}
    echo ""
    echo -e "''${GREEN}Available models:''${NC}"
    echo "1. tiny.en-q5_1    (~30MB)  - Fastest, lowest accuracy"
    echo "2. base.en-q5_1    (~60MB)  - Good balance ⭐ RECOMMENDED"
    echo "3. small.en-q5_1   (~240MB) - Better accuracy"
    echo "4. medium.en-q5_1  (~760MB) - High accuracy (heavy CPU)"
    echo ""

    read -p "Select model (1-4) [2]: " CHOICE
    CHOICE=''${CHOICE:-2}

    case $CHOICE in
      1) MODEL="${modelInfo."1".name}"
         SIZE="${modelInfo."1".size}"
         THREADS="${modelInfo."1".threads}"
         BUFFER="${modelInfo."1".buffer}" ;;
      2) MODEL="${modelInfo."2".name}"
         SIZE="${modelInfo."2".size}"
         THREADS="${modelInfo."2".threads}"
         BUFFER="${modelInfo."2".buffer}" ;;
      3) MODEL="${modelInfo."3".name}"
         SIZE="${modelInfo."3".size}"
         THREADS="${modelInfo."3".threads}"
         BUFFER="${modelInfo."3".buffer}" ;;
      4) MODEL="${modelInfo."4".name}"
         SIZE="${modelInfo."4".size}"
         THREADS="${modelInfo."4".threads}"
         BUFFER="${modelInfo."4".buffer}" ;;
      *) echo -e "''${RED}Invalid choice''${NC}"; exit 1 ;;
    esac

    mkdir -p "$MODELS_DIR"

    if [ -f "$MODELS_DIR/$MODEL" ]; then
      echo -e "''${YELLOW}✓ Model already downloaded: $MODEL''${NC}"
      read -p "Re-download? (y/N): " REDOWNLOAD
      if [[ ! "$REDOWNLOAD" =~ ^[Yy]$ ]]; then
        echo -e "''${GREEN}✓ Using existing model''${NC}"
        exit 0
      fi
    fi

    # Check disk space
    FREE_GB=$(df -BG "$MODELS_DIR" | awk 'NR==2 {gsub("G","",$4); print $4}')
    if (( FREE_GB < 2 )); then
      echo -e "''${RED}❌ Insufficient disk space: ''${FREE_GB}GB free''${NC}"
      exit 1
    fi

    echo -e "''${BLUE}⬇️  Downloading $MODEL ($SIZE)...''${NC}"
    cd "$MODELS_DIR"

    if command -v ${pkgs.wget}/bin/wget &> /dev/null; then
      ${pkgs.wget}/bin/wget --show-progress -O "$MODEL" "$BASE_URL/$MODEL"
    elif command -v ${pkgs.curl}/bin/curl &> /dev/null; then
      ${pkgs.curl}/bin/curl -L --progress-bar -o "$MODEL" "$BASE_URL/$MODEL"
    else
      echo -e "''${RED}❌ Neither wget nor curl available''${NC}"
      exit 1
    fi

    if [ -f "$MODEL" ]; then
      chmod 444 "$MODEL"
      ACTUAL_SIZE=$(${pkgs.coreutils}/bin/du -h "$MODEL" | cut -f1)
      echo ""
      echo -e "''${GREEN}✅ Downloaded: $MODEL ($ACTUAL_SIZE)''${NC}"
      echo ""
      echo -e "''${YELLOW}📝 Add to your .env file:''${NC}"
      echo "WHISPER_MODELS_PATH=$MODELS_DIR"
      echo "WHISPER_MODEL=/models/$MODEL"
      echo ""
      echo -e "''${GREEN}💡 Performance recommendations:''${NC}"
      echo "   WHISPER_THREADS=$THREADS"
      echo "   BUFFER_DURATION=$BUFFER"
      echo ""
      echo -e "''${YELLOW}⚠️  Add to activeModels in nix/whisper-manager.nix:''${NC}"
      echo "   activeModels = [ \"$MODEL\" ];"
    else
      echo -e "''${RED}❌ Download failed''${NC}"
      exit 1
    fi
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
    echo -e "''${GREEN}Active (in nix/whisper-manager.nix):''${NC}"
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
      fetch)    ${fetchModels}/bin/whisper-fetch ;;
      download) ${downloadModel}/bin/whisper-download ;;
      prune)    ${pruneUnusedModels}/bin/whisper-prune ;;
      list)     ${listModels}/bin/whisper-list ;;
      *)
        echo "Whisper Model Manager"
        echo ""
        echo "Usage:"
        echo "  whisper fetch     - Download the transcriber's models, verified"
        echo "  whisper download  - Download a new model (auto-prunes)"
        echo "  whisper prune     - Remove models not in activeModels list"
        echo "  whisper list      - Show active vs downloaded models"
        ;;
    esac
  '';

  packagesList = [
    whisperCLI
    fetchModels
    downloadModel
    pruneUnusedModels
    listModels
    pkgs.wget
    pkgs.curl
  ];

  hookScript = ''
    export WHISPER_MODELS_PATH="${modelsDir}"
    echo "🎯 Whisper Manager loaded"
    echo "   Commands: whisper {fetch|download|prune|list}"
    echo "   Models: ${modelsDir}"
  '';
in {
  packages = packagesList;

  # Export individual commands for direct access
  whisper-cli = whisperCLI;
  whisper-fetch = fetchModels;
  whisper-download = downloadModel;
  whisper-prune = pruneUnusedModels;
  whisper-list = listModels;

  shellHook = hookScript;

  # Standalone shell
  shell = pkgs.mkShell {
    buildInputs = packagesList;
    shellHook = hookScript;
  };
}
