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
