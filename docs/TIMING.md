# Operation timing (`microscaler-timing`)

Design note, November 2026. Goal: **annotating a function (or wrapping a block) is the only step
needed for its p50/p95/p99 to appear on a dashboard.** No per-function metric declarations, no
per-function dashboard edits.

## Shape

One histogram family for everything that is timed:

```text
op_duration_seconds{component, op, outcome}      (+ the scrape's namespace/job/pod/... labels)
op_in_flight{component, op}                      (optional gauge, off by default)
```

| label | meaning | examples |
|---|---|---|
| `component` | crate / subsystem, dotted | `brrtrouter.client`, `brrtrouter.handler`, `lifeguard.pool` |
| `op` | the operation: a function name or a spec operationId | `checkout`, `connect`, `listPets` |
| `outcome` | `ok` \| `err` \| `timeout` | |

All three are static strings (`&'static str`): string literals in the macros, or names from a
bounded set fixed at startup via `intern()` (a router's operationIds). Never ids, tickers, URLs,
SQL or user data.

A single dashboard queries the family with `histogram_quantile` and is templated on
`namespace`, `job`, `component`, `op`, so a newly annotated function shows up without anyone
touching the dashboard.

## API

```rust
use microscaler_timing::{timed, timed_block, Outcome};

#[timed(component = "lifeguard.pool", op = "checkout")]   // sync, async, methods
fn checkout(&self) -> Result<Conn, PoolError> { ... }     // Ok -> ok, Err -> err / timeout

let rows = timed_block!("lifeguard.query", "decode", { decode(&raw) });

static WRITE: Site = Site::new("brrtrouter.client", "request_write");
let t = WRITE.start();  ...  t.finish(Outcome::Ok);        // guard for manual spans
WRITE.record(Outcome::Ok, duration);                        // durations measured elsewhere
```

- Defaults: `op` = the function name, `component` = `module_path!()` (`::` rendered as `.`).
- `Result` return types (last path segment ends in `Result`) set the outcome; an error type that
  implements `IsTimeout` (done for `std::io::Error`: `TimedOut`, `WouldBlock`) records `timeout`.
- A panic records `err`; an async call dropped before completion records `err`.

## Why direct recording, not a `tracing` layer

The brief preferred building on `tracing` spans with a metrics layer if that integrated with the
existing setup. It does not fit well, so recording is direct:

1. **Filtering.** The services' subscriber filters spans globally; a span below the filter (the
   client phase spans are `DEBUG`) is never created, so a metrics layer would see nothing unless
   every span were enabled for every layer, which costs a registry slot per span on the hot path.
2. **may coroutines.** Request context is coroutine-scoped (`may_tracing`), not entered on the OS
   thread; span enter/exit timing on a thread is wrong when a coroutine migrates. Span
   extensions also take a per-span lock.
3. **Independence.** Timing must work with no subscriber at all (tests, tools, a service that
   never calls `init()`), and its cost must not depend on the trace sampling rate.

Traces keep their spans; the histogram is the source of truth for dashboards. Exemplars
(trace ids on buckets) are not emitted: Prometheus text exemplars need OpenMetrics negotiation
and a per-bucket trace-id slot, which is not lock-free with an atomic-only design.

## may safety

The services run on `may`, where a blocked OS thread stalls every coroutine scheduled on it.

- A `Site` is a `static` (or a leaked, interned site): labels, an in-flight counter, and per
  outcome 12 bucket counters + a sum, all `AtomicU64`. Recording = 2 relaxed `fetch_add`s
  (+2 for in-flight with the guard). No mutex, no lazy init, no allocation, no thread-locals.
- The first record links the site into a process-wide list with a CAS loop (lock-free).
- The start `Instant` lives in the `Timer` on the caller's stack: correct across coroutine
  migration and `.await`.
- `tests/no_blocking.rs` scans the record-path source for blocking primitives and runs 2000
  coroutines on 2 workers that yield between start and finish.
- Only `intern()` locks, and it is called at setup (handler registration), never per call.
- The prometheus crate was not used: it adds a dependency (and `parking_lot` / lazy statics) to
  every crate that wants one attribute, and its registry rejects a second registration of the
  same family from a second crate.

## Cardinality and cost (Managed Prometheus)

Managed Prometheus bills per sample: a histogram point is **2 samples + 1 per non-empty bucket**
($0.06 per million for the first 50 billion a month). Choices that follow:

- **11 buckets**, 1 ms to 2.5 s: `1, 2.5, 5, 10, 25, 50, 100, 250, 500, 1000, 2500 ms`, then
  `+Inf`. 50 ms and 100 ms (the SLO low/high-water marks) are bucket bounds, so the quantiles
  are exact at the targets.
- **Only observed series are rendered**: a site, and each outcome of it, appears on the first
  record. Code paths that never run cost nothing.
- **Same labels, one series**: two sites with the same (component, op) are merged at render.
- **In-flight gauge off** unless `MICROSCALER_TIMING_IN_FLIGHT=1` (one more series per op).
- `intern()` caps at 2048 sites, then `_overflow/_other`.

Estimate for pp: see the README of this repository (Operation timing, Cost).

## One copy per binary

Sites register in a list owned by the crate; two copies (two git revisions in one binary's
graph) would keep two lists and only one would be rendered. Depend on it with
`branch = "main"` from every crate (`cargo tree -d -i microscaler-timing` shows duplicates).
The crate is not re-exported by `microscaler-observability` for the same reason: services pin
that crate by revision.

## Where it is wired

| component | ops | where |
|---|---|---|
| `brrtrouter.queue` / `brrtrouter.handler` | operationId | BRRTRouter dispatcher, typed handlers, worker pools |
| `brrtrouter.client` / `brrtrouter.proxy` | `pool_checkout`, `pool_wait`, `connect`, `connect_tls`, `request_write`, `response_headers`, `response_body`, `call` | BRRTRouter's `fetch_*` pool and BFF proxy, from the may_minihttp client's lifecycle events |
| `brrtrouter.net` / `brrtrouter.proxy` | `resolve_cached`, `resolve` / `dns_cached`, `dns_resolve` | BRRTRouter's resolver and the proxy's address cache |
| `brrtrouter.auth` | `jwt_verify` | bearer JWT validation |
| `lifeguard.pool` | `checkout`, `checkout_rls` | Lifeguard's worker pool |
| `lifeguard.query` | `execute`, `query_opt`, `query_all`, `rls_unit`, `decode` | Lifeguard statements and `FromRow` |
| `lifeguard.cache` | `get_hit`, `get_miss`, `set`, `invalidate` | the model cache |
| `lifereflector` | `notify_to_applied`, `flush` | lifeReflector |
| service components | `#[timed]` on their own I/O | e.g. ClickHouse, message bus, broker gateway, billing API, session store, token signing |

`response_headers` is from the request being written to the response head being read (the
downstream's time plus the network); `response_body` is reading the body. A slow proxied call
splits into the two.

## Decisions against the brief

- Direct atomic recording, not a `tracing` metrics layer (see above).
- No `prometheus` crate: zero dependencies, and one registry per process without double
  registration.
- The crate lives in this repository but is not re-exported from `microscaler-observability`
  (one-copy rule).
- `op_in_flight` is off by default (cost); `MICROSCALER_TIMING_IN_FLIGHT=1` turns it on.
- Existing histograms (`brrtrouter_request_queue_seconds`, `sesame_bff_store_seconds`,
  `lifeguard_*`, `lifereflector_*`) are kept for now. The queue and session-store ones duplicate
  the shared family and can go once nothing reads them.
