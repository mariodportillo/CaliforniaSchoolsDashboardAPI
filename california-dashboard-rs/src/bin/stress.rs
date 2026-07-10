//! Local-only load harness for the request scheduler.
//!
//! This intentionally never contacts the public California Dashboard API.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use california_dashboard::{ClientConfig, DashboardClient, FetchSpec, SchoolRecord};
use clap::Parser;
use serde::Serialize;
use tokio::net::TcpListener;

#[derive(Debug, Parser)]
#[command(about = "Stress-test the Rust fetch engine against a local mock server")]
struct Arguments {
    /// Number of synthetic requests to complete.
    #[arg(long, default_value_t = 10_000)]
    requests: usize,
    /// Maximum concurrent requests.
    #[arg(long, default_value_t = 50)]
    concurrency: usize,
    /// Global request-start ceiling.
    #[arg(long, default_value_t = 1_000.0)]
    requests_per_second: f64,
    /// Mock response delay, used to create overlapping requests.
    #[arg(long, default_value_t = 20)]
    delay_ms: u64,
}

#[derive(Default)]
struct MockState {
    active: AtomicUsize,
    max_active: AtomicUsize,
    served: AtomicUsize,
    delay_ms: AtomicUsize,
}

#[derive(Debug, Serialize)]
struct StressSummary {
    requests: usize,
    completed: usize,
    failures: usize,
    elapsed_seconds: f64,
    observed_requests_per_second: f64,
    configured_concurrency: usize,
    maximum_in_flight: usize,
    server_requests: usize,
    result_order_preserved: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = Arguments::parse();
    ensure!(
        arguments.requests > 0,
        "--requests must be greater than zero"
    );

    let state = Arc::new(MockState::default());
    state
        .delay_ms
        .store(arguments.delay_ms as usize, Ordering::Relaxed);
    let app = Router::new()
        .fallback(mock_summary_cards)
        .with_state(state.clone());
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("bind local mock server")?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("local stress server remains available");
    });

    let school = SchoolRecord {
        cds_code: "19649071995901".to_owned(),
        school_name: "Synthetic Stress School".to_owned(),
        county: Some("Los Angeles".to_owned()),
        district: Some("Synthetic District".to_owned()),
        city: Some("Pomona".to_owned()),
    };
    let specs = (0..arguments.requests)
        .map(|index| FetchSpec {
            school: school.clone(),
            year: 2025,
            dashboard_year_id: 11,
            url: format!("http://{address}/Reports/{index}/11/SummaryCards"),
        })
        .collect::<Vec<_>>();

    let client = DashboardClient::new(ClientConfig {
        concurrency: arguments.concurrency,
        max_requests_per_second: arguments.requests_per_second,
        timeout: Duration::from_secs(5),
        max_payload_bytes: 1024 * 1024,
    })?;
    let started = Instant::now();
    let outcomes = client.fetch_all(&specs).await;
    let elapsed = started.elapsed();
    server.abort();

    let failures = outcomes
        .iter()
        .filter(|outcome| !outcome.is_success())
        .count();
    let order_preserved = outcomes
        .iter()
        .zip(&specs)
        .all(|(outcome, expected)| outcome.spec.url == expected.url);
    let summary = StressSummary {
        requests: arguments.requests,
        completed: outcomes.len(),
        failures,
        elapsed_seconds: elapsed.as_secs_f64(),
        observed_requests_per_second: outcomes.len() as f64 / elapsed.as_secs_f64(),
        configured_concurrency: arguments.concurrency,
        maximum_in_flight: state.max_active.load(Ordering::Relaxed),
        server_requests: state.served.load(Ordering::Relaxed),
        result_order_preserved: order_preserved,
    };

    ensure!(summary.completed == summary.requests, "dropped outcomes");
    ensure!(summary.failures == 0, "one or more requests failed");
    ensure!(
        summary.server_requests == summary.requests,
        "server count mismatch"
    );
    ensure!(summary.result_order_preserved, "result order changed");
    ensure!(
        summary.maximum_in_flight <= summary.configured_concurrency,
        "configured concurrency was exceeded"
    );

    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

async fn mock_summary_cards(State(state): State<Arc<MockState>>) -> Response {
    let active = state.active.fetch_add(1, Ordering::SeqCst) + 1;
    state.max_active.fetch_max(active, Ordering::SeqCst);
    state.served.fetch_add(1, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(
        state.delay_ms.load(Ordering::Relaxed) as u64,
    ))
    .await;
    state.active.fetch_sub(1, Ordering::SeqCst);

    (
        StatusCode::OK,
        [("content-type", "application/json")],
        r#"[{"indicatorId":2,"primary":{"cdsCode":"19649071995901","indicatorId":2,"status":2.5,"change":-1.6,"count":1619,"studentGroup":"ALL","schoolYearId":11,"isPrivateData":false},"secondary":{"cdsCode":"00000000000000","indicatorId":2,"status":2.9,"change":-0.4,"count":5961200,"studentGroup":"ALL","schoolYearId":11,"isPrivateData":false}}]"#,
    )
        .into_response()
}
