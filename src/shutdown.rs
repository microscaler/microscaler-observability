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
    /// `OTEL_EXPORTER_OTLP_ENDPOINT` is unset; Phase O.1 makes this the
    /// production path for unconfigured services.
    ///
    /// In v0.0.1 this is only called from tests (see the `tests` module).
    /// The `#[cfg(test)]` gate avoids a `dead_code` clippy warning until
    /// Phase O.1 wires it into [`crate::init`]; remove the gate in that
    /// same change.
    //
    // No `#[must_use]` here because the return type `Self = ShutdownGuard`
    // is already `#[must_use = "…"]` on the struct — clippy's
    // `double_must_use` lint would fire otherwise.
    #[cfg(test)]
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

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::missing_panics_doc
    )]

    use super::*;

    #[test]
    fn noop_constructs_and_drops_cleanly() {
        // v0.0.1 scaffold: the no-op guard is the only kind that can be
        // constructed (init() panics). Verify it drops without panicking
        // or leaking anything observable.
        let guard = ShutdownGuard::noop();
        drop(guard);
        // If drop panicked, `cargo test` would fail. No-op expected.
    }

    #[test]
    fn guard_is_send() {
        // Guard will live in main() for the process lifetime; must be
        // movable to another thread if the host spawns workers and moves
        // ownership around.
        fn assert_send<T: Send>(_: &T) {}
        let guard = ShutdownGuard::noop();
        assert_send(&guard);
    }

    #[test]
    fn guard_is_sync() {
        // In v0.0.1 the guard is trivially Sync (zero-sized fields). Once
        // Phase O.1 adds `Arc<TracerProvider>` etc., those providers are
        // Sync by contract in opentelemetry_sdk 0.29. This test is a
        // compile-time tripwire for that property.
        fn assert_sync<T: Sync>(_: &T) {}
        let guard = ShutdownGuard::noop();
        assert_sync(&guard);
    }
}
