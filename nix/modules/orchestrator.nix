# `services.orchestrator`: orchestrator as a container, its settings generated
# from infra/env/orchestrator.json (#408). See mk-app.nix. It publishes no
# port, as infra/compose/orchestrator.yml does not (#143).
import ./mk-app.nix {
  name = "orchestrator";
  image = "pgathondu/orchestrator:latest";
  generated = ./generated/orchestrator.nix;
}
