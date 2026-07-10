use axum::Router;
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use california_dashboard::client::ClientError;
use california_dashboard::model::{FetchSpec, FetchStatus, SchoolRecord, parse_indicators};
use california_dashboard::resolver::{MatchKind, ResolverError, SchoolResolver};
use california_dashboard::years::{SUPPORTED_YEARS, dashboard_year_id, summary_cards_url};
use california_dashboard::{ClientConfig, DashboardClient};
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;

#[test]
fn parses_array_and_singleton_without_fabricating_missing_values() {
    let array = parse_indicators(include_bytes!("fixtures/summary_array.json")).unwrap();
    assert_eq!(array.len(), 2);
    assert_eq!(array[0].category, "SUSPENSION_RATE");
    assert_eq!(array[0].primary.as_ref().unwrap().status, Some(2.5));
    assert_eq!(array[0].secondary.as_ref().unwrap().status, Some(2.9));
    let private = array[1].primary.as_ref().unwrap();
    assert_eq!(private.is_private_data, Some(true));
    assert_eq!(private.status, None);
    assert_eq!(private.count, None);
    assert!(array[1].secondary.is_none());

    let singleton = parse_indicators(include_bytes!("fixtures/summary_singleton.json")).unwrap();
    assert_eq!(singleton.len(), 1);
    assert_eq!(singleton[0].indicator_id, 8);
    assert_eq!(singleton[0].category, "SCIENCE");
}

#[test]
fn year_mapping_and_url_are_exact() {
    assert_eq!(
        SUPPORTED_YEARS,
        [2017, 2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025]
    );
    assert_eq!(
        SUPPORTED_YEARS.map(|year| dashboard_year_id(year).unwrap()),
        [3, 4, 5, 6, 7, 8, 9, 10, 11]
    );
    assert_eq!(dashboard_year_id(2026), None);
    assert_eq!(
        summary_cards_url("19649071995901", 2025).unwrap(),
        "https://api.caschooldashboard.org/Reports/19649071995901/11/SummaryCards"
    );
    assert!(summary_cards_url("123", 2025).is_err());
}

#[test]
fn resolver_retains_duplicate_names_and_reports_ambiguity() {
    let (_directory, resolver) = fixture_resolver();
    assert_eq!(resolver.len(), 5);
    assert_eq!(resolver.directory_records().len(), 6);
    assert!(
        resolver
            .directory_records()
            .iter()
            .any(|record| record.school == "Closed School" && record.status == "Closed")
    );
    let all_map = resolver.build_all_schools_map(&[2024, 2025]).unwrap();
    assert!(all_map.contains_key("Lincoln Elementary (00000000000001)"));
    assert!(all_map.contains_key("Lincoln Elementary (00000000000002)"));
    assert_eq!(all_map["Pomona High School"], [2024, 2025]);
    let specs = resolver
        .fetch_specs_for_queries(
            &["Lincoln Elementary (00000000000001)".to_owned()],
            &[2024, 2025],
        )
        .unwrap();
    assert_eq!(specs.len(), 2);
    assert!(
        specs
            .iter()
            .all(|spec| spec.school.cds_code == "00000000000001")
    );
    assert_eq!(
        resolver
            .school_by_cds("00000000000002")
            .unwrap()
            .county
            .as_deref(),
        Some("Beta")
    );

    let selected = resolver.resolve("00000000000002").unwrap();
    assert_eq!(selected.kind, MatchKind::ExactCds);
    assert_eq!(selected.school.cds_code, "00000000000002");

    let ambiguous = resolver.resolve("Lincoln Elementary").unwrap_err();
    match ambiguous {
        ResolverError::Ambiguous { candidates, .. } => {
            assert_eq!(candidates.len(), 2);
            assert_eq!(candidates[0].cds_code, "00000000000001");
            assert_eq!(candidates[1].cds_code, "00000000000002");
        }
        other => panic!("expected ambiguity, got {other:?}"),
    }
}

#[test]
fn resolver_matching_and_search_are_deterministic() {
    let (_directory, resolver) = fixture_resolver();
    assert!(matches!(
        resolver.resolve("Pomo"),
        Err(ResolverError::NotFound(_))
    ));

    let substring = resolver.resolve("Pomona High").unwrap();
    assert_eq!(substring.kind, MatchKind::Substring);
    assert_eq!(substring.school.cds_code, "00000000000003");

    let fuzzy = resolver.resolve("Ponderosa Acadmy").unwrap();
    assert_eq!(fuzzy.kind, MatchKind::Levenshtein);
    assert_eq!(fuzzy.distance, Some(1));

    assert_eq!(resolver.search("north district", 10).len(), 1);
    assert_eq!(resolver.search("beta", 10).len(), 1);
    assert_eq!(resolver.search("riverside", 10).len(), 2);

    let specs = resolver
        .fetch_specs(
            &["00000000000003".to_owned(), "00000000000004".to_owned()],
            &[2024, 2025],
        )
        .unwrap();
    assert_eq!(specs.len(), 4);
    assert_eq!(specs[0].school.cds_code, "00000000000003");
    assert_eq!(specs[0].year, 2024);
    assert_eq!(specs[1].year, 2025);
    assert_eq!(specs[2].school.cds_code, "00000000000004");
}

#[test]
fn invalid_client_configuration_is_rejected() {
    let error = match DashboardClient::new(ClientConfig {
        concurrency: 0,
        ..ClientConfig::default()
    }) {
        Ok(_) => panic!("zero concurrency unexpectedly accepted"),
        Err(error) => error,
    };
    assert!(matches!(error, ClientError::InvalidConfiguration(_)));
}

#[tokio::test]
async fn client_retries_documented_statuses_and_preserves_metadata() {
    let server = MockServer::start().await;
    let client = DashboardClient::new(test_client_config(4)).unwrap();
    let spec = mock_spec(format!("{}/retry", server.base_url), "Original Name");
    let outcome = client.fetch(&spec).await;

    assert!(outcome.is_success());
    assert_eq!(outcome.spec.school.school_name, "Original Name");
    assert_eq!(outcome.provenance.source_url, spec.url);
    assert_eq!(outcome.provenance.attempts, 4);
    assert_eq!(outcome.provenance.http_status, Some(200));
    assert!(outcome.provenance.payload_sha256.is_some());
    assert_eq!(server.state.count("retry"), 4);
    assert_eq!(
        outcome.card.as_ref().unwrap().provenance,
        outcome.provenance
    );
}

#[tokio::test]
async fn client_enforces_concurrency_order_and_payload_limit_under_load() {
    let server = MockServer::start().await;
    let client = DashboardClient::new(ClientConfig {
        max_payload_bytes: 256,
        ..test_client_config(7)
    })
    .unwrap();

    let specs: Vec<_> = (0..140)
        .map(|index| {
            mock_spec(
                format!("{}/slow-{index}", server.base_url),
                &format!("School {index:03}"),
            )
        })
        .collect();
    let outcomes = client.fetch_all(&specs).await;
    assert_eq!(outcomes.len(), specs.len());
    assert!(outcomes.iter().all(|outcome| outcome.is_success()));
    for (expected, actual) in specs.iter().zip(&outcomes) {
        assert_eq!(actual.spec, *expected);
    }
    assert!(server.state.max_active.load(Ordering::SeqCst) <= 7);
    assert!(server.state.max_active.load(Ordering::SeqCst) > 1);

    let oversized = client
        .fetch(&mock_spec(
            format!("{}/large", server.base_url),
            "Large Payload",
        ))
        .await;
    assert!(matches!(
        oversized.status,
        FetchStatus::PayloadTooLarge {
            limit_bytes: 256,
            ..
        }
    ));
    assert!(oversized.card.is_none());
}

fn fixture_resolver() -> (TempDir, SchoolResolver) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("schools.db");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE schools (
                CDSCode TEXT, School TEXT, StatusType TEXT,
                County TEXT, District TEXT, City TEXT
             );",
        )
        .unwrap();
    let rows = [
        (
            "00000000000001",
            "Lincoln Elementary",
            "Alpha",
            "North District",
            "Riverside",
        ),
        (
            "00000000000002",
            "Lincoln Elementary",
            "Beta",
            "South District",
            "Riverside",
        ),
        (
            "00000000000003",
            "Pomona High School",
            "Los Angeles",
            "Pomona Unified",
            "Pomona",
        ),
        (
            "00000000000004",
            "Ponderosa Academy",
            "Orange",
            "Coast Unified",
            "Irvine",
        ),
        (
            "00000000000005",
            "Closed School",
            "Orange",
            "Coast Unified",
            "Irvine",
        ),
        (
            "00000000000006",
            "Active Academy",
            "San Diego",
            "Mesa District",
            "San Diego",
        ),
    ];
    for (index, row) in rows.iter().enumerate() {
        let status = if index == 4 { "Closed" } else { "Active" };
        connection
            .execute(
                "INSERT INTO schools VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![row.0, row.1, status, row.2, row.3, row.4],
            )
            .unwrap();
    }
    drop(connection);
    let resolver = SchoolResolver::open(&path).unwrap();
    (directory, resolver)
}

fn mock_spec(url: String, school_name: &str) -> FetchSpec {
    FetchSpec {
        school: SchoolRecord {
            cds_code: "19649071995901".to_owned(),
            school_name: school_name.to_owned(),
            county: Some("Los Angeles".to_owned()),
            district: Some("Pomona Unified".to_owned()),
            city: Some("Pomona".to_owned()),
        },
        year: 2025,
        dashboard_year_id: 11,
        url,
    }
}

fn test_client_config(concurrency: usize) -> ClientConfig {
    ClientConfig {
        concurrency,
        max_requests_per_second: 1_000_000_000.0,
        timeout: Duration::from_secs(3),
        max_payload_bytes: 4 * 1024,
    }
}

#[derive(Default)]
struct MockState {
    counts: Mutex<HashMap<String, usize>>,
    active: AtomicUsize,
    max_active: AtomicUsize,
}

impl MockState {
    fn increment(&self, key: &str) -> usize {
        let mut counts = self.counts.lock().unwrap();
        let count = counts.entry(key.to_owned()).or_default();
        *count += 1;
        *count
    }

    fn count(&self, key: &str) -> usize {
        *self.counts.lock().unwrap().get(key).unwrap_or(&0)
    }
}

struct MockServer {
    base_url: String,
    state: Arc<MockState>,
}

impl MockServer {
    async fn start() -> Self {
        let state = Arc::new(MockState::default());
        let app = Router::new()
            .route("/{case}", get(mock_handler))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            base_url: format!("http://{address}"),
            state,
        }
    }
}

async fn mock_handler(
    State(state): State<Arc<MockState>>,
    AxumPath(case): AxumPath<String>,
) -> Response {
    let active = state.active.fetch_add(1, Ordering::SeqCst) + 1;
    state.max_active.fetch_max(active, Ordering::SeqCst);
    let count = state.increment(&case);

    let response = if case == "retry" && count < 4 {
        let status = match count {
            1 => StatusCode::REQUEST_TIMEOUT,
            2 => StatusCode::TOO_MANY_REQUESTS,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        };
        let mut response = (status, "retry").into_response();
        response
            .headers_mut()
            .insert("retry-after", HeaderValue::from_static("0"));
        response
    } else if case == "large" {
        vec![b'x'; 2_048].into_response()
    } else {
        if case.starts_with("slow-") {
            let index = case.trim_start_matches("slow-").parse::<u64>().unwrap_or(0);
            tokio::time::sleep(Duration::from_millis(2 + index % 9)).await;
        }
        (
            StatusCode::OK,
            "{\"indicatorId\":2,\"primary\":{\"status\":2.5,\"isPrivateData\":false}}",
        )
            .into_response()
    };

    state.active.fetch_sub(1, Ordering::SeqCst);
    response
}
