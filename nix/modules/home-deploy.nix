# `services.home-deploy`: home pulls a newer file_host image on demand, when
# a client needs a newer server than home runs, and only then (the owner's
# ask, 2026-11-10; README.md, "Deploying to home"). No timer and no polling:
#
# - a client asks `GET /compat?needs=<epoch>`; when the epoch is above the
#   running one (and the bounds in apps/servers/file_host/src/deploy.rs
#   allow it), file_host writes ${STORAGE}/deploy/wanted;
# - the path unit below starts home-deploy.service when that file exists;
# - the service runs scripts/deploy.py --on-demand: pull, read the image's
#   epoch, deploy only a newer one (file_host migrates as it starts),
#   health-wait, roll back, record ${STORAGE}/deploy/state.json, and remove
#   the marker. It never restores the database.
#
# It runs as root, since ${STORAGE} is file_host's uid's and docker is
# root's, and runs `make` (envctl, sops) as `user`, who holds the age key
# (call C83). Home's own configuration (paulgsc/dotfiles) imports this and
# sets the three values only home knows.
{
  config,
  lib,
  pkgs,
  ...
}: let
  cfg = config.services.home-deploy;
in {
  options.services.home-deploy = {
    enable = lib.mkEnableOption "home's on-demand deploy of file_host (scripts/deploy.py --on-demand)";
    user = lib.mkOption {
      type = lib.types.str;
      default = "";
      description = ''
        The user who runs `make up` in the checkout today: in the `docker`
        group, holding the age key `.sops.yaml` names. `make` runs as them.
      '';
    };
    checkout = lib.mkOption {
      type = lib.types.str;
      default = "";
      description = "The absolute path of this repository's checkout on the host.";
    };
    storage = lib.mkOption {
      type = lib.types.str;
      default = "";
      description = "STORAGE, as deploy/<role>/compose.sops.env sets it: the host directory holding file_host's database. The marker is `<storage>/deploy/wanted`.";
    };
    role = lib.mkOption {
      type = lib.types.str;
      default = "home";
      description = "The role under deploy/ (DEPLOY), as for `make up`.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = cfg.user != "";
        message = "services.home-deploy.user is required: the user who runs `make up` in the checkout";
      }
      {
        assertion = lib.hasPrefix "/" cfg.checkout;
        message = "services.home-deploy.checkout is required, as an absolute path: the repository's checkout on the host";
      }
      {
        assertion = lib.hasPrefix "/" cfg.storage;
        message = "services.home-deploy.storage is required, as an absolute path: STORAGE, file_host's database directory";
      }
    ];

    systemd.paths.home-deploy = {
      description = "A client needs a newer file_host than home runs";
      wantedBy = ["multi-user.target"];
      pathConfig = {
        PathExists = "${cfg.storage}/deploy/wanted";
        Unit = "home-deploy.service";
      };
    };

    systemd.services.home-deploy = {
      description = "Home's on-demand deploy: pull file_host, deploy a newer contract epoch, health-wait, roll back";
      after = ["docker.service" "network-online.target"];
      wants = ["network-online.target"];
      # What `make up` and the script call: make builds envctl through
      # `nix develop .#rust` when no cargo is on PATH, and envctl runs sops.
      path = [
        pkgs.bash
        pkgs.coreutils
        pkgs.docker
        pkgs.git
        pkgs.gnumake
        pkgs.openssh
        pkgs.python3
        pkgs.sops
        pkgs.util-linux
        config.nix.package
      ];
      environment.DEPLOY = cfg.role;
      serviceConfig = {
        Type = "oneshot";
        WorkingDirectory = cfg.checkout;
        # envctl's decrypted files: tmpfs, handed to `user` by the script.
        RuntimeDirectory = "home-deploy";
        RuntimeDirectoryMode = "0700";
        # The images that failed here, so a bad one is not put up again.
        StateDirectory = "home-deploy";
        ExecStart = lib.concatStringsSep " " [
          "${pkgs.python3}/bin/python3"
          "${cfg.checkout}/scripts/deploy.py"
          "--on-demand ${cfg.storage}/deploy"
          "--as-user ${cfg.user}"
          "--runtime-dir /run/home-deploy"
          "--state-dir /var/lib/home-deploy"
        ];
        # A pull, maybe envctl's first build, and up to 360 s of waiting,
        # twice when it rolls back.
        TimeoutStartSec = "30min";
      };
    };
  };
}
