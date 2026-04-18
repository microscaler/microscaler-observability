//! Error types for [`crate::init`] and friends.

use thiserror::Error;

/// Any error produced while constructing or shutting down the observability
/// adapter.
#[derive(Debug, Error)]
pub enum ObservabilityError {
    /// [`crate::init`] was called more than once in a single process.
    #[error("microscaler-observability was already initialized; init() must be called exactly once from main()")]
    AlreadyInitialized,

    /// The `OTEL_EXPORTER_OTLP_ENDPOINT` environment variable was malformed
    /// (e.g. not a valid URL, or a URL whose scheme/host/port made no sense
    /// for the chosen protocol).
    #[error("invalid OTLP endpoint `{value}`: {reason}")]
    InvalidEndpoint {
        /// The offending env-var value.
        value: String,
        /// Human-readable explanation.
        reason: String,
    },

    /// The OTLP exporter could not be constructed (TLS error, transport
    /// failure, etc.).
    #[error("OTLP exporter construction failed: {0}")]
    ExporterConstruction(String),

    /// Another `tracing` subscriber has already claimed the global slot.
    /// Typically means either [`crate::init`] was called twice or some
    /// other code-path in the process called
    /// `tracing::subscriber::set_global_default` first.
    #[error("a tracing subscriber is already globally installed")]
    SubscriberAlreadyInstalled,

    /// Failure while flushing the batch processors on shutdown.
    #[error("observability shutdown failed: {0}")]
    Shutdown(String),
}

/// Convenience alias for `Result<T, ObservabilityError>`.
pub type ObservabilityResult<T> = Result<T, ObservabilityError>;

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
    fn already_initialized_display_is_actionable() {
        let err = ObservabilityError::AlreadyInitialized;
        let rendered = err.to_string();
        assert!(
            rendered.contains("already initialized"),
            "AlreadyInitialized message must be human-readable: {rendered}"
        );
        assert!(
            rendered.contains("main()"),
            "AlreadyInitialized must tell the reader where init() is meant to be called: {rendered}"
        );
    }

    #[test]
    fn invalid_endpoint_display_includes_value_and_reason() {
        let err = ObservabilityError::InvalidEndpoint {
            value: "not a url".to_string(),
            reason: "missing scheme".to_string(),
        };
        let rendered = err.to_string();
        assert!(rendered.contains("not a url"));
        assert!(rendered.contains("missing scheme"));
    }

    #[test]
    fn exporter_construction_propagates_inner_reason() {
        let err = ObservabilityError::ExporterConstruction("TLS handshake failed".to_string());
        assert!(err.to_string().contains("TLS handshake failed"));
    }

    #[test]
    fn error_implements_std_error() {
        // Compile-time assertion that ObservabilityError is a first-class
        // std::error::Error, so callers can `? into Box<dyn Error>`.
        fn takes_error<E: std::error::Error>(_: &E) {}
        takes_error(&ObservabilityError::AlreadyInitialized);
    }

    #[test]
    fn observability_result_compiles_as_result_alias() {
        let ok: ObservabilityResult<i32> = Ok(42);
        let err: ObservabilityResult<i32> = Err(ObservabilityError::AlreadyInitialized);
        assert!(matches!(ok, Ok(42)));
        assert!(err.is_err());
    }
}
