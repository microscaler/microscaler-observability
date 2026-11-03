# microscaler-timing

Annotate a function; its latency histogram appears on `/metrics` as
`op_duration_seconds{component, op, outcome}` and on the generic operation-latency dashboard.
Zero runtime dependencies, lock-free, safe on `may` coroutines. See the
[repository README](../README.md#operation-timing) and [design note](../docs/TIMING.md).
