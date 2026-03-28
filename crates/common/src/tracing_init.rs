//! Tracing subscriber initialisation.

use tracing_subscriber::{fmt, EnvFilter};

/// Initialise the global tracing subscriber.
///
/// - Uses the `log_level` argument as the default filter (e.g. `"info"`), but
///   the `RUST_LOG` environment variable takes precedence.
/// - In release builds the output is JSON; in debug builds it is the
///   human-readable pretty format.
pub fn init_tracing(log_level: &str) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    #[cfg(not(debug_assertions))]
    {
        fmt().json().with_env_filter(filter).init();
    }
    #[cfg(debug_assertions)]
    {
        fmt().pretty().with_env_filter(filter).init();
    }
}
