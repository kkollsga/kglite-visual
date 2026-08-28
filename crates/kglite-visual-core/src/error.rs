//! The one error type every consumer of this crate maps at its own boundary:
//! the CLI to an exit code + stderr line, the PyO3 wheel to a Python
//! exception.

/// Failures this crate can produce.
///
/// One variant per kglite entry point we actually call, and no more. There is
/// deliberately **no** `Save` variant: kglite-visual is a read-only viewer,
/// its kglite contact surface (plan D11) contains no write path, and a variant
/// nothing constructs is a claim the code contradicts.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// `.kgl` load failed. kglite's loaders report the `std::io::Error`
    /// family — including a corrupt or wrong-format file, which arrives as
    /// `InvalidData` rather than as a distinct type.
    #[error("could not load graph: {0}")]
    Load(#[from] std::io::Error),

    /// A Cypher execution failed: parse error, schema mismatch, timeout, or
    /// cancellation. `KgError` carries a `KgErrorCode` a binding can branch on.
    ///
    /// Boxed because `KgError` is ~128 bytes and this enum sits in the `Err`
    /// half of every load and query result on the hot path
    /// (`clippy::result_large_err`). The `From<KgError>` impl below keeps `?`
    /// working at the call sites, so the box is invisible to callers.
    #[error("query failed: {0}")]
    Query(#[source] Box<kglite::api::KgError>),
}

impl From<kglite::api::KgError> for CoreError {
    fn from(err: kglite::api::KgError) -> Self {
        Self::Query(Box::new(err))
    }
}
