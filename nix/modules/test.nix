# The modules' own test (#408): evaluates NixOS configurations that import
# them, without building anything, and fails (throws) unless each says what
# it should. CI runs it as `.github/workflows/lint.yml`'s `NixOS modules` job:
#
#   nix eval --impure --expr 'import ./nix/modules/test.nix { nixpkgs = (builtins.getFlake (toString ./.)).inputs.nixpkgs; }'
{nixpkgs}: let
  lib = import "${nixpkgs}/lib";
  eval = settings:
    import "${nixpkgs}/nixos/lib/eval-config.nix" {
      system = "x86_64-linux";
      modules = [
        ./file-host.nix
        ./orchestrator.nix
        {
          boot.isContainer = true;
          system.stateVersion = "24.05";
        }
        settings
      ];
    };
  failed = configuration: map (assertion: assertion.message) (builtins.filter (assertion: !assertion.assertion) configuration.config.assertions);
  expect = what: condition:
    if condition
    then true
    else throw "nix/modules/test.nix: ${what}";

  # On, with nothing set: every gap at once.
  bare = failed (eval {services.file-host.enable = true;});
  # Off: nothing asked, nothing run.
  off = eval {};
  # Everything the binary needs, its secrets through a file.
  complete = eval {
    services.file-host = {
      enable = true;
      environmentFiles = ["/run/secrets/file-host.env"];
      settings = {
        AUTH_ENABLED = false;
        NUDGE_TIMEZONE = "America/Denver";
        ALLOWED_ORIGINS = ["https://a.example" "https://b.example"];
        APP_BASE_URL = "https://a.example/";
        NATS_URL = "nats://nats:4222";
        REDIS_URL = "redis://redis:6379";
      };
    };
    services.orchestrator = {
      enable = true;
      settings.NATS_URL = "nats://nats:4222";
    };
  };
  # On, with nothing set: since #617 no NATS is assumed.
  bareOrchestrator = failed (eval {services.orchestrator.enable = true;});
  container = complete.config.virtualisation.oci-containers.containers.file-host;
  # The mailbox on, with neither its URL nor a secrets file. (No numeric
  # bound is left to test here: since #617 the one file_host keeps,
  # MAILBOX_ARCHIVE_DAYS, holds only while a secret is set, which settings
  # cannot say. app_config's own tests cover bounds.)
  mailbox = failed (eval {
    services.file-host = {
      enable = true;
      settings = {
        AUTH_ENABLED = false;
        MAILBOX_ENABLED = true;
      };
    };
  });
in
  lib.all (x: x) [
    (expect "a bare file-host names its required settings and secrets" (lib.all (gap: builtins.any (lib.hasInfix gap) bare) [
      "settings.DATABASE_URL is required (a secret"
      "settings.WEBAUTHN_RP_ID is required while AUTH_ENABLED is true"
      "settings.ALLOWED_ORIGINS is required"
      "settings.APP_BASE_URL is required"
      "settings.NATS_URL is required"
      "settings.REDIS_URL is required"
    ]))
    (expect "a bare orchestrator names NATS_URL" (builtins.any (lib.hasInfix "services.orchestrator.settings.NATS_URL is required") bareOrchestrator))
    (expect "an off module asks for nothing and runs nothing" (failed off == [] && off.config.virtualisation.oci-containers.containers == {}))
    (expect "a complete file-host asserts nothing" (failed complete == []))
    (expect "set settings reach the container as strings, unset ones not at all" (container.environment
      == {
        AUTH_ENABLED = "false";
        NUDGE_TIMEZONE = "America/Denver";
        ALLOWED_ORIGINS = "https://a.example,https://b.example";
        APP_BASE_URL = "https://a.example/";
        NATS_URL = "nats://nats:4222";
        REDIS_URL = "redis://redis:6379";
      }))
    (expect "secrets reach the container through its files" (container.environmentFiles == ["/run/secrets/file-host.env"]))
    (expect "file-host publishes what compose does" (container.ports == ["127.0.0.1:3000:3000"] && container.image == "pgathondu/server:latest"))
    (expect "orchestrator publishes no port" (complete.config.virtualisation.oci-containers.containers.orchestrator.ports == []))
    (expect "the mailbox's own requirements are named while it is on" (builtins.any (lib.hasInfix "MAILBOX_URL is required while MAILBOX_ENABLED is true") mailbox
        && builtins.any (lib.hasInfix "MAILBOX_TOKEN is required while MAILBOX_ENABLED is true (a secret") mailbox))
  ]
