//! RAII shutdown handle returned by [`crate::init`].

/// Opaque RAII handle. Hold this in `main()` for the lifetime of the
/// process; its [`Drop`] impl is where the batch processors get flushed and
/// the global providers shut down.
///
/// ```ignore
/// fn main() -> anyhow::Result<()> {
///     let _obs = microscaler_observability::init(/* ... */)?;
///     // ... run the service ...
///     // _obs dropped here, flush happens before std::process::exit
///     Ok(())
/// }
/// ```
///
/// For tests that construct a subscriber scoped to a single test, use
/// `tracing::subscriber::set_default` rather than [`crate::init`] — the
/// global install is process-wide by design and is specifically for
/// `main()`-level use.
#[must_use = "dropping the ShutdownGuard immediately triggers telemetry flush; hold it for the process lifetime"]
pub struct ShutdownGuard {
    // Private fields land with Phase O.1. The opaque shape is stable;
    // the internals (TracerProvider, LoggerProvider handles, non_blocking
    // appender guard) may evolve.
    _private: (),
}

impl ShutdownGuard {
    /// Construct a no-op guard. Used internally when
    /// `OTEL_EXPORTER_OTLP_ENDPOINT` is unset.
    #[must_use]
    pub(crate) const fn noop() -> Self {
        Self { _private: () }
    }
}

impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        // Phase O.1 fills in:
        //   - force_flush on BatchSpanProcessor (5s timeout)
        //   - force_flush on BatchLogRecordProcessor (5s timeout)
        //   - shutdown on TracerProvider
        //   - shutdown on LoggerProvider
        //   - Drop the tracing_appender non_blocking guard last (flushes
        //     stdout fallback if that was active).
    }
}
