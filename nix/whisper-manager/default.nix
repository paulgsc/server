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
  # paulgsc/server#462), and the take library's meaning model beside it,
  # mounted into its container at /models by infra/compose/file_host.yml.
  # `whisper fetch` downloads exactly these.
  # Multilingual, not `.en`: the notes mix Korean into English, and the
  # transcriber refuses an English-only model at startup. Each with its
  # source and SHA-256 (Hugging Face's own, from the file's `X-Linked-ETag`),
  # so a fetch keeps only the bytes it expects.
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
  ]
  # The take library's meaning index (`file_host::meaning`,
  # docs/take-library.md), read only when MEANING_ENABLED: Model2Vec's
  # potion-multilingual-128M (MIT), its three files in a directory of their
  # own, which `file_host::meaning::DIR` names under /models. Pinned to one
  # commit of the model's repository, so a fetch is the same bytes every
  # time; the two large files' SHA-256 are Hugging Face's own
  # (`X-Linked-ETag`), and config.json's is of the bytes at that commit.
  ++ (map (m: m // {url = "https://huggingface.co/minishlab/potion-multilingual-128M/resolve/73908c3438cf03b6a01bcb9611d62b23d0726f08/${baseNameOf m.name}";}) [
    {
      name = "potion-multilingual-128M/model.safetensors";
      sha256 = "14b5eb39cb4ce5666da8ad1f3dc6be4346e9b2d601c073302fa0a31bf7943397";
      size = "512MB";
    }
    {
      name = "potion-multilingual-128M/tokenizer.json";
      sha256 = "19f1909063da3cfe3bd83a782381f040dccea475f4816de11116444a73e1b6a1";
      size = "19MB";
    }
    {
      name = "potion-multilingual-128M/config.json";
      sha256 = "595e4cab2093732efd5dbe084fd5c1826b5eea693b73b4c1fd971672867d2e54";
      size = "1KB";
    }
  ]);

  # What `whisper prune` keeps and `whisper list` shows as active.
  activeModels = map (m: m.name) pinnedModels;

  # Where the models go: the directory infra/compose/file_host.yml mounts at
  # /models, and its HOST-SPECIFIC default (#406). TRANSCRIBE_MODELS_DIR, the
  # variable that mount reads, overrides it when exported: each command
  # reads it as it runs.
  modelsDir = "/mnt/storage/users/dev/models/whisper-models";
  # ======================================
  pruneUnusedModels = pkgs.writeShellScriptBin "whisper-prune" ''
    set -euo pipefail
    MODELS_DIR="''${TRANSCRIBE_MODELS_DIR:-${modelsDir}}"

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
    MODELS_DIR="''${TRANSCRIBE_MODELS_DIR:-${modelsDir}}"
    mkdir -p "$MODELS_DIR"
    ${builtins.concatStringsSep "\n" (map (m: ''
        if [ -f "$MODELS_DIR/${m.name}" ] && echo "${m.sha256}  $MODELS_DIR/${m.name}" | ${pkgs.coreutils}/bin/sha256sum --check --status; then
          echo "✓ ${m.name} already there"
        else
          echo "⬇️  ${m.name} (${m.size})"
          mkdir -p "$(dirname "$MODELS_DIR/${m.name}")"
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
    echo "TRANSCRIBE_ENABLED=true, and the meaning model when MEANING_ENABLED=true"
    echo "(infra/compose/file_host.yml mounts this directory)."
  '';

  listModels = pkgs.writeShellScriptBin "whisper-list" ''
    set -euo pipefail

    # Colors
    GREEN='\033[0;32m'
    BLUE='\033[0;34m'
    NC='\033[0m'

    MODELS_DIR="''${TRANSCRIBE_MODELS_DIR:-${modelsDir}}"

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
    FREE_GB=$(df -BG "$MODELS_DIR" 2>/dev/null | awk 'NR==2 {gsub("G","",$4); print $4}' || echo "?")
    echo "💾 Free space: ''${FREE_GB}GB"
  '';

  whisperCLI = pkgs.writeShellScriptBin "whisper" ''
    usage() {
      echo "Whisper Model Manager"
      echo ""
      echo "Usage:"
      echo "  whisper fetch  - Download the transcriber's models, verified"
      echo "  whisper prune  - Remove models not in pinnedModels"
      echo "  whisper list   - Show active vs downloaded models"
    }

    case "''${1:-}" in
      fetch) ${fetchModels}/bin/whisper-fetch ;;
      prune) ${pruneUnusedModels}/bin/whisper-prune ;;
      list)  ${listModels}/bin/whisper-list ;;
      help|"") usage ;;
      *)
        echo "whisper: unknown command '$1'" >&2
        usage >&2
        exit 1
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

  # Export no WHISPER_* variable: file_host warns at startup on any.
  hookScript = ''
    echo "🎯 Whisper Manager loaded"
    echo "   Commands: whisper {fetch|prune|list}"
    echo "   Models: ''${TRANSCRIBE_MODELS_DIR:-${modelsDir}}"
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
