// outbound-panels.libsonnet
//
// The OUTBOUND dashboard's panels (paulgsc/some-ui#3330): every request home
// makes to the outside, and every container's network beside it.
//
// The `outbound_*` series come from crates/outbound, the one constructor
// every HTTP client in file_host and its crates is built by. Their labels
// are closed sets the compiler holds: `caller` (one per worker), `host` (a
// fixed name per caller, never read from a URL) and `outcome` (ok,
// refused_4xx, refused_5xx, timeout, no_answer). Each caller's counters are
// set to zero when its client is built, so a worker that is off draws
// nothing and one that is on reads 0 until it sends. What is measured ends at
// the answer's headers: a body that stalls or fails after them counts as its
// status, and bytes received are declared lengths (crates/outbound's doc).
//
// The `container_network_*` series are cadvisor's, per container: the
// catch-all for the services that are not file_host (render, kokoro, the
// orchestrator...), which make requests this crate never sees. A container
// on the host's network has no interface of its own and draws nothing here.

local ds = { type: 'prometheus', uid: 'prometheus' };

local stat(title, description, expr, unit, steps) = {
  title: title,
  description: description,
  type: 'stat',
  datasource: ds,
  targets: [{ expr: expr, instant: true, refId: 'A' }],
  fieldConfig: {
    defaults: {
      unit: unit,
      color: { mode: 'thresholds' },
      thresholds: { mode: 'absolute', steps: steps },
      noValue: 'no data',
    },
    overrides: [],
  },
  options: {
    colorMode: 'value',
    graphMode: 'none',
    justifyMode: 'center',
    orientation: 'horizontal',
    reduceOptions: { calcs: ['lastNotNull'], values: false },
    textMode: 'value',
  },
};

local series(title, description, targets, unit, stack=false) = {
  title: title,
  description: description,
  type: 'timeseries',
  datasource: ds,
  targets: targets,
  fieldConfig: {
    defaults: {
      unit: unit,
      custom: {
        drawStyle: 'line',
        lineWidth: 1,
        fillOpacity: if stack then 30 else 10,
        spanNulls: false,
        stacking: { mode: if stack then 'normal' else 'none', group: 'A' },
      },
    },
    overrides: [],
  },
  options: { legend: { displayMode: 'table', placement: 'right', calcs: ['mean', 'max'] }, tooltip: { mode: 'multi', sort: 'desc' } },
};

local perMinute(by) = '60 * sum by (' + by + ') (rate(outbound_requests_total[5m]))';

{
  requestsPerMinute: stat(
    'REQUESTS / MIN',
    'Requests file_host sent to the outside per minute, over the last 5 minutes, every caller together. At rest this is the game feed and the pollers\' timers alone.',
    '60 * sum(rate(outbound_requests_total[5m]))',
    'none',
    [{ color: 'green', value: null }],
  ) { fieldConfig+: { defaults+: { decimals: 1 } } },

  requestsToday: stat(
    'REQUESTS, 24H',
    'Requests sent in the last 24 hours, every caller together.',
    'sum(increase(outbound_requests_total[24h]))',
    'none',
    [{ color: 'green', value: null }],
  ) { fieldConfig+: { defaults+: { decimals: 0 } } },

  failedToday: stat(
    'NOT OK, 24H',
    'Requests in the last 24 hours that were refused (4xx, 5xx) or got no answer\'s headers (timeout, no_answer); a body that fails after its headers is counted by its status, and shows on the worker\'s own dashboard (GAME FEED\'s timeout and transport kinds). Some are normal: a push service answers 410 for a browser that left, ESPN rate-limits.',
    'sum(increase(outbound_requests_total{outcome!="ok"}[24h]))',
    'none',
    [{ color: 'green', value: null }, { color: 'orange', value: 1 }],
  ) { fieldConfig+: { defaults+: { decimals: 0 } } },

  p95: stat(
    'P95 WAIT, 1H',
    'The 95th percentile of the time from sending a request to its answer\'s headers, or to its failure, over the last hour, every caller together.',
    'histogram_quantile(0.95, sum by (le) (rate(outbound_request_duration_seconds_bucket[1h])))',
    's',
    [{ color: 'green', value: null }, { color: 'orange', value: 5 }],
  ),

  bytesToday: stat(
    'RECEIVED, 24H',
    'Answer bytes declared (Content-Length) in the last 24 hours, whether or not the caller then read them (the game feed leaves an oversized or non-200 body unread). An answer that declares no length is not counted; the container row below is the wire\'s own count.',
    'sum(increase(outbound_received_bytes_total[24h]))',
    'bytes',
    [{ color: 'green', value: null }],
  ),

  byCaller: series(
    'Requests per minute, by caller',
    'Each worker that reaches outside home: game_feed (ESPN), call_weeks and cards_essays (paulgsc/testing on GitHub), mailbox (the Jobs mailbox Worker), notice (FCM), nudge (Web Push), snips (the snips bundle of a commit, from the Actions API of GitHub), health_probe (file_host asking itself; never scraped).',
    [{ expr: perMinute('caller'), legendFormat: '{{caller}}', refId: 'A' }],
    'none',
    true,
  ),

  byHost: series(
    'Requests per minute, by host',
    'Where the requests go. The host is a fixed name per caller, never read from a URL: github_raw is call_weeks and cards_essays together; github_api is snips, the storage its downloads redirect to included.',
    [{ expr: perMinute('host'), legendFormat: '{{host}}', refId: 'A' }],
    'none',
    true,
  ),

  failures: series(
    'Not OK, by outcome and caller',
    'Per 15 minutes. refused_4xx and refused_5xx are answers by status class; timeout is no answer in time; no_answer is no connection, a reset or TLS. The game feed\'s own dashboard splits its answers further, by what came back.',
    [{ expr: 'sum by (caller, outcome) (increase(outbound_requests_total{outcome!="ok"}[15m]))', legendFormat: '{{caller}} {{outcome}}', refId: 'A' }],
    'none',
  ),

  latency: series(
    'Wait for an answer, p95 by caller',
    'From send to the answer\'s headers, or to the failure, over 15 minutes. A caller at its timeout (20 s for the game feed, 30 s for the pollers) is getting no answer; the nudge has no timeout, so a hung push sits above the top bucket (60 s).',
    [{ expr: 'histogram_quantile(0.95, sum by (caller, le) (rate(outbound_request_duration_seconds_bucket[15m])))', legendFormat: '{{caller}}', refId: 'A' }],
    's',
  ),

  bytes: series(
    'Bytes, by caller',
    'Per second, over 5 minutes. sent is request bodies; received is answers\' declared lengths, read or not.',
    [
      { expr: 'sum by (caller) (rate(outbound_sent_bytes_total[5m]))', legendFormat: '{{caller}} sent', refId: 'A' },
      { expr: 'sum by (caller) (rate(outbound_received_bytes_total[5m]))', legendFormat: '{{caller}} received', refId: 'B' },
    ],
    'Bps',
  ),

  containersNamed: stat(
    'CONTAINERS NAMED',
    'Containers cadvisor reports with a `name` label, the label the two panels beside this group by. Every compose service is a container, so this should be about their count; no data means cadvisor names none of them (it cannot read Docker\'s own record of them), and the network panels then draw nothing rather than one sum of every cgroup.',
    'count(count by (name) (container_network_receive_bytes_total{name!=""}))',
    'none',
    [{ color: 'green', value: null }],
  ) { fieldConfig+: { defaults+: { decimals: 0 } } },

  containerTransmit: series(
    'Every container: sent',
    'cadvisor\'s bytes sent per second, by container name, every interface summed, over 5 minutes. A series without a name (a cgroup cadvisor could not match to a container) is left out, not summed into one. Includes what a container sends on the compose network, not only to the outside.',
    [{ expr: 'sum by (name) (rate(container_network_transmit_bytes_total{name!=""}[5m]))', legendFormat: '{{name}}', refId: 'A' }],
    'Bps',
    true,
  ),

  containerReceive: series(
    'Every container: received',
    'cadvisor\'s bytes received per second, by container, every interface summed, over 5 minutes.',
    [{ expr: 'sum by (name) (rate(container_network_receive_bytes_total{name!=""}[5m]))', legendFormat: '{{name}}', refId: 'A' }],
    'Bps',
    true,
  ),
}
