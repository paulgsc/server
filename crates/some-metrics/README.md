# some-metrics

The seam decided in [#139](https://github.com/paulgsc/server/issues/139):
application crates speak the `metrics` facade (`counter!`, `gauge!`,
`histogram!`); this crate is the only place that knows the facade's output is
a Prometheus scrape.

```text
application crates
      |
      |  metrics::{counter!, gauge!, histogram!}
      v
   `metrics`
      |
      | recorder installed once, here
      v
metrics-exporter-prometheus
      |
      v
   /metrics
```

A service depends on this crate and calls one function:

- [`install`] once at startup, to register the global recorder.
- [`route`] to mount `GET /metrics` on an axum router the service already
  runs (see `file_host`).
- [`serve`] to stand up a bare listener whose only job is `/metrics`, for a
  service with no other HTTP surface (see `orchestrator`). Bind it to an
  address reachable only on the compose network — do not publish it to the
  host unless the service already publishes a port for other reasons.

No service-specific metrics belong in this crate. It owns installation and
exposition, not measurement.
