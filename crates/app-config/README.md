# app-config

The configuration groups more than one binary reads, declared once
([#405](https://github.com/paulgsc/server/issues/405)): `NatsArgs` (`NATS_URL`),
`LogArgs` (`RUST_LOG`) and `OtelArgs` (the `OTEL_*` variables and `ENVIRONMENT`).
Each binary pulls the ones it uses into its clap `Config` with
`#[command(flatten)]`. See the crate docs, and README.md, "Configuration".
