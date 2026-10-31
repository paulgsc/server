# `services.file-host`: file_host as a container, its settings generated from
# infra/env/file_host.json (#408). See mk-app.nix.
import ./mk-app.nix {
  name = "file-host";
  image = "pgathondu/server:latest";
  generated = ./generated/file_host.nix;
  # As infra/compose/file_host.yml publishes it: the host's Caddy serves it on.
  ports = ["127.0.0.1:3000:3000"];
}
