//! RAII shutdown handle returned by [`crate::init`].

use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::trace::SdkTracerProvider;

/// Opaque RAII handle. Hold this in `main()` for the lifetime of the
/// process; its [`Drop`] impl is where the batch processors get flushed and
/// the providers shut down.
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
    tracer_provider: Option<SdkTracerProvider>,
    logger_provider: Option<SdkLoggerProvider>,
}

impl ShutdownGuard {
    /// Construct a no-op guard (no OTLP — stdout-only or tests).
    pub(crate) const fn noop() -> Self {
        Self {
            tracer_provider: None,
            logger_provider: None,
        }
    }

    /// Hold clones of the OTLP providers for flush/shutdown on drop.
    #[allow(clippy::missing_const_for_fn)] // not const: SDK handles are not const-constructible
    pub(crate) fn new_otlp(
        tracer_provider: SdkTracerProvider,
        logger_provider: SdkLoggerProvider,
    ) -> Self {
        Self {
            tracer_provider: Some(tracer_provider),
            logger_provider: Some(logger_provider),
        }
    }
}

impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        if let Some(tp) = self.tracer_provider.take() {
            let _ignored = tp.force_flush();
            let _ignored = tp.shutdown();
        }
        if let Some(lp) = self.logger_provider.take() {
            let _ignored = lp.force_flush();
            let _ignored = lp.shutdown();
        }
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
        let guard = ShutdownGuard::noop();
        drop(guard);
    }

    #[test]
    fn guard_is_send() {
        fn assert_send<T: Send>(_: &T) {}
        let guard = ShutdownGuard::noop();
        assert_send(&guard);
    }

    #[test]
    fn guard_is_sync() {
        fn assert_sync<T: Sync>(_: &T) {}
        let guard = ShutdownGuard::noop();
        assert_sync(&guard);
    }
}
