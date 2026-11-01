# A NixOS module for one of this repo's binaries, run as an OCI container,
# from its generated options (`generated/<binary>.nix`, `envctl nix`,
# #408). The module declares no variable of its own: every setting is the
# binary's, typed after its schema, and null leaves the binary's default.
#
# A missing required setting, one required while another is on, or a number
# out of its limits fails the build with an assertion naming it, all of them
# in one evaluation, as `make preflight` names them for compose.
#
# Secrets have no option: the store is world-readable. They reach the
# container only through `environmentFiles`, which on the host are files
# sops-nix renders (paulgsc/dotfiles, nixos/secrets). The module cannot see
# inside those files, so for a secret it requires that there is one, and the
# binary itself refuses to start without the value (ENV1).
#
# The module carries no host values: the host's configuration sets them
# (#406). It is off unless `enable`.
{
  name,
  image,
  generated,
  ports ? [],
}: {
  config,
  lib,
  ...
}: let
  cfg = config.services.${name};
  schema = import generated {inherit lib;};

  # A float through `toJSON`, not `toString`, which keeps six decimals and
  # would hand the binary 0.000000 for 0.0000001; Rust reads `1e-07` back.
  scalar = value:
    if builtins.isFloat value
    then builtins.toJSON value
    else toString value;
  render = value:
    if builtins.isBool value
    then lib.boolToString value
    else if builtins.isList value
    then lib.concatMapStringsSep "," scalar value
    else scalar value;

  given = lib.filterAttrs (_: value: value != null) cfg.settings;
  environment = lib.mapAttrs (_: render) given // cfg.extraEnvironment;

  # What the binary will read for `variable`: what is set here, else its default.
  effective = variable: environment.${variable} or (schema.defaults.${variable} or null);
  isSet = variable: environment ? ${variable} && environment.${variable} != "";
  secret = variable: builtins.elem variable schema.secrets;
  holds = when: let
    value = effective when.variable;
  in
    if when.is == "*"
    then value != null && value != ""
    else value == when.is;

  # A secret is accounted for once some environment file is given; anything
  # else must be set here.
  provided = variable:
    if secret variable
    then cfg.environmentFiles != [] || isSet variable
    else isSet variable;

  missing = map (variable: "services.${name}.settings.${variable} is required" + lib.optionalString (secret variable) " (a secret: put it in one of services.${name}.environmentFiles)") (builtins.filter (variable: !provided variable) schema.required);

  requiredWhen = map (rule: "services.${name}.settings.${rule.variable} is required while ${rule.when} is ${rule.is}" + lib.optionalString (secret rule.variable) " (a secret: put it in one of services.${name}.environmentFiles)") (builtins.filter (rule:
    holds {
      variable = rule.when;
      inherit (rule) is;
    }
    && !provided rule.variable)
  schema.requiredWhen);

  limit = bound:
    if bound.min != null && bound.max != null
    then "from ${toString bound.min} to ${toString bound.max}"
    else if bound.min != null
    then "at least ${toString bound.min}"
    else "at most ${toString bound.max}";
  outOfBounds = map (bound: "services.${name}.settings.${bound.variable} must be ${limit bound}") (builtins.filter (bound: let
    value = given.${bound.variable} or null;
  in
    value
    != null
    && lib.all holds bound.when
    && ((bound.min != null && value < bound.min) || (bound.max != null && value > bound.max)))
  schema.bounds);
in {
  options.services.${name} = {
    enable = lib.mkEnableOption "the ${name} container, configured from its schema";

    image = lib.mkOption {
      type = lib.types.str;
      default = image;
      description = "The image the container runs.";
    };

    settings = lib.mkOption {
      type = lib.types.submodule {options = schema.settings;};
      default = {};
      description = "Every variable ${name} reads but its secrets, each typed after its schema (infra/env). Null leaves the binary's default.";
    };

    environmentFiles = lib.mkOption {
      type = lib.types.listOf lib.types.path;
      default = [];
      description = "Files of `NAME=value` lines given to the container, for the secrets (${lib.concatStringsSep ", " schema.secrets}): a sops-nix template's path, never a file in the store.";
    };

    extraEnvironment = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = {};
      description = "Variables the schema does not list, for the container's dependencies (TOKIO_WORKER_THREADS, RUST_BACKTRACE). Not for secrets: these land in the store.";
    };

    ports = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = ports;
      description = "Ports to publish, as `docker run -p` takes them.";
    };

    volumes = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "Host paths to mount, as `docker run -v` takes them. HOST-SPECIFIC by nature (#406): set by the host.";
    };

    extraOptions = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [];
      description = "Extra `docker run` options, such as `--network=...`.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = map (message: {
      assertion = false;
      inherit message;
    }) (missing ++ requiredWhen ++ outOfBounds);

    virtualisation.oci-containers.containers.${name} = {
      inherit (cfg) image ports volumes extraOptions environmentFiles;
      inherit environment;
    };
  };
}
