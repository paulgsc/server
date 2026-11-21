# outbound

The one way home reaches outside (paulgsc/some-ui#3330). Every HTTP client in
`file_host` and its crates is built by `outbound::client(Caller::…)`, and every
request it sends is recorded once:

- `outbound_requests_total{caller, host, outcome}`;
- `outbound_request_duration_seconds{caller, host}`;
- `outbound_sent_bytes_total{caller, host}` and `outbound_received_bytes_total{caller, host}`.

`Caller`, `Host` and `Outcome` are closed enums: a new worker that fetches
anything adds a `Caller` case, and `host` is that case's fixed name, never read
from a URL. `clippy.toml` refuses `reqwest::Client::new` and
`reqwest::Client::builder` anywhere but here. The OUTBOUND Grafana dashboard
(`infra/grafana`) draws the series.
