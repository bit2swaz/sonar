//! Lightweight axum HTTP server for the Sonar indexer.
//!
//! Exposes:
//!   `GET /account_history/:pubkey?from_slot=<u64>&to_slot=<u64>`
//!
//! Returns a JSON array of `u64` lamport balances for the given account
//! within the requested slot range, ordered by (slot ASC, write_version ASC).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use serde::Deserialize;
use sqlx::PgPool;
use tokio::net::TcpListener;
use tracing::{error, info};

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BalanceRangeParams {
    pub from_slot: u64,
    pub to_slot: u64,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

async fn account_history_handler(
    State(pool): State<PgPool>,
    Path(pubkey_b58): Path<String>,
    Query(params): Query<BalanceRangeParams>,
) -> Response {
    // Decode base-58 pubkey.
    let pubkey_bytes = match bs58::decode(&pubkey_b58).into_vec() {
        Ok(b) if b.len() == 32 => {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&b);
            arr
        },
        Ok(b) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("pubkey must decode to 32 bytes, got {}", b.len()),
            )
                .into_response();
        },
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid base-58 pubkey: {e}"),
            )
                .into_response();
        },
    };

    match crate::db::query_balances_in_range(&pool, &pubkey_bytes, params.from_slot, params.to_slot)
        .await
    {
        Ok(balances) => Json(balances).into_response(),
        Err(e) => {
            error!("query_balances_in_range failed: {e:#}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to query account history".to_string(),
            )
                .into_response()
        },
    }
}

// ---------------------------------------------------------------------------
// Server entry point
// ---------------------------------------------------------------------------

/// Build the axum `Router` (useful for testing without binding a port).
pub fn build_router(pool: PgPool) -> Router {
    Router::new()
        .route("/account_history/:pubkey", get(account_history_handler))
        .with_state(pool)
}

/// Start the indexer HTTP server on `0.0.0.0:{port}`.
///
/// This future runs indefinitely; cancel it with a `tokio::select!` or a
/// `CancellationToken` from the caller.
pub async fn start_server(pool: PgPool, port: u16) -> anyhow::Result<()> {
    let addr = format!("0.0.0.0:{port}");
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| anyhow::anyhow!("failed to bind HTTP server to {addr}: {e}"))?;

    info!("Indexer HTTP server listening on {addr}");

    axum::serve(listener, build_router(pool))
        .await
        .map_err(|e| anyhow::anyhow!("HTTP server error: {e}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt as _;

    use super::*;

    /// Builds a test router backed by an in-memory stub: no real DB needed.
    /// We cannot unit-test the full DB path without a real Postgres instance
    /// (covered by `db.rs` integration tests), so we just verify routing and
    /// input validation.
    #[tokio::test]
    async fn test_bad_pubkey_returns_400() {
        // We need a PgPool to build the router, but the handler will fail at
        // the bs58 decode step before ever touching the pool.  Use a dummy
        // connection string — the pool is never used in this test.
        let pool =
            sqlx::PgPool::connect_lazy("postgres://localhost/sonar_test").expect("lazy pool");
        let app = build_router(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/account_history/NOT_VALID_B58!!!?from_slot=0&to_slot=100")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_wrong_length_pubkey_returns_400() {
        // 3 bytes base-58 encoded — decodes fine but length != 32
        let short_b58 = bs58::encode(vec![1u8, 2u8, 3u8]).into_string();
        let pool =
            sqlx::PgPool::connect_lazy("postgres://localhost/sonar_test").expect("lazy pool");
        let app = build_router(pool);

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/account_history/{short_b58}?from_slot=0&to_slot=100"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_closed_pool_returns_500() {
        let pool =
            sqlx::PgPool::connect_lazy("postgres://localhost/sonar_test").expect("lazy pool");
        pool.close().await;
        let app = build_router(pool);
        let valid_pubkey = bs58::encode([7u8; 32]).into_string();

        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/account_history/{valid_pubkey}?from_slot=0&to_slot=100"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
