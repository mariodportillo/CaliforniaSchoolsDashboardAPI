//! Local-only Axum application for selecting schools, running pulls, and exporting reports.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY,
    X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderValue, Response, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{RwLock, Semaphore};

use crate::client::ClientConfig;
use crate::{
    APP_VERSION, DashboardClient, ExportBundle, FetchSpec, ReportModel, SUPPORTED_YEARS,
    SchoolResolver, dashboard_year_id, directory_csv_bytes, directory_xlsx_bytes,
};

const INDEX_HTML: &str = include_str!("../assets/index.html");
const STYLES_CSS: &str = include_str!("../assets/styles.css");
const APP_JS: &str = include_str!("../assets/app.js");
const MAX_JOB_REQUESTS: usize = 100_000;
const MAX_SCHOOL_SEARCH_LIMIT: usize = 100;
const MAX_ROW_PREVIEW_LIMIT: usize = 5_000;
const MAX_RETAINED_JOBS: usize = 3;

static NEXT_JOB_ID: AtomicU64 = AtomicU64::new(1);

/// Shared configuration and state for a local dashboard server.
#[derive(Clone)]
pub struct WebState {
    inner: Arc<WebStateInner>,
}

struct WebStateInner {
    resolver: Arc<SchoolResolver>,
    jobs: RwLock<HashMap<String, JobRecord>>,
    job_gate: Semaphore,
    default_client_config: ClientConfig,
    base_url_override: Option<String>,
}

impl WebState {
    pub fn new(
        resolver: SchoolResolver,
        default_client_config: ClientConfig,
        base_url_override: Option<String>,
    ) -> Result<Self, String> {
        default_client_config
            .validate()
            .map_err(|error| error.to_string())?;
        let base_url_override = base_url_override
            .map(|url| url.trim().trim_end_matches('/').to_owned())
            .filter(|url| !url.is_empty());
        Ok(Self {
            inner: Arc::new(WebStateInner {
                resolver: Arc::new(resolver),
                jobs: RwLock::new(HashMap::new()),
                // One pull at a time keeps the configured concurrency/rate ceilings global.
                job_gate: Semaphore::new(1),
                default_client_config,
                base_url_override,
            }),
        })
    }

    pub fn school_count(&self) -> usize {
        self.inner.resolver.len()
    }
}

/// Builds the full application router. Assets are compiled into the binary.
pub fn router(state: WebState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/assets/styles.css", get(styles))
        .route("/assets/app.js", get(script))
        .route("/health", get(health))
        .route("/api/years", get(years))
        .route("/api/schools", get(search_schools))
        .route("/api/directory.csv", get(directory_csv))
        .route("/api/directory.xlsx", get(directory_xlsx))
        .route("/api/jobs", post(create_job))
        .route("/api/jobs/{id}", get(job_status))
        .route("/api/jobs/{id}/rows", get(job_rows))
        .route("/api/jobs/{id}/downloads/{format}", get(download))
        .fallback(not_found)
        .with_state(state)
}

async fn index() -> impl IntoResponse {
    secure_static_response("text/html; charset=utf-8", INDEX_HTML)
}

async fn styles() -> impl IntoResponse {
    static_response("text/css; charset=utf-8", STYLES_CSS)
}

async fn script() -> impl IntoResponse {
    static_response("text/javascript; charset=utf-8", APP_JS)
}

fn static_response(content_type: &'static str, content: &'static str) -> Response<Body> {
    let mut response = Response::new(Body::from(content));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    // Assets are compiled into the binary and change on every rebuild, so force
    // revalidation instead of caching a stale copy for an hour.
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}

fn secure_static_response(content_type: &'static str, content: &'static str) -> Response<Body> {
    let mut response = static_response(content_type, content);
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'",
        ),
    );
    response
        .headers_mut()
        .insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

async fn health(State(state): State<WebState>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": APP_VERSION,
        "school_count": state.school_count(),
        "directory_record_count": state.inner.resolver.directory_records().len(),
    }))
}

#[derive(Debug, Serialize)]
struct YearOption {
    year: u16,
    dashboard_id: u8,
}

async fn years() -> Json<Value> {
    let options: Vec<_> = SUPPORTED_YEARS
        .into_iter()
        .map(|year| YearOption {
            year,
            dashboard_id: dashboard_year_id(year).expect("supported year has an ID"),
        })
        .collect();
    Json(json!({ "years": options }))
}

#[derive(Debug, Deserialize)]
struct SchoolSearch {
    #[serde(default)]
    q: String,
    #[serde(default = "default_search_limit")]
    limit: usize,
}

const fn default_search_limit() -> usize {
    25
}

async fn search_schools(
    State(state): State<WebState>,
    Query(query): Query<SchoolSearch>,
) -> Json<Value> {
    let limit = query.limit.min(MAX_SCHOOL_SEARCH_LIMIT);
    Json(json!({
        "schools": state.inner.resolver.search(&query.q, limit),
        "query": query.q,
        "limit": limit,
    }))
}

async fn directory_csv(State(state): State<WebState>) -> Result<Response<Body>, ApiError> {
    let records = state.inner.resolver.directory_records().to_vec();
    let bytes = tokio::task::spawn_blocking(move || directory_csv_bytes(&records))
        .await
        .map_err(|error| ApiError::internal(format!("directory task failed: {error}")))?
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Ok(attachment_response(
        bytes,
        "text/csv; charset=utf-8",
        "cde-school-directory.csv",
    ))
}

async fn directory_xlsx(State(state): State<WebState>) -> Result<Response<Body>, ApiError> {
    let records = state.inner.resolver.directory_records().to_vec();
    let bytes = tokio::task::spawn_blocking(move || directory_xlsx_bytes(&records))
        .await
        .map_err(|error| ApiError::internal(format!("directory task failed: {error}")))?
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Ok(attachment_response(
        bytes,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "cde-school-directory.xlsx",
    ))
}

fn attachment_response(
    bytes: Vec<u8>,
    content_type: &'static str,
    filename: &'static str,
) -> Response<Body> {
    let mut response = Response::new(Body::from(bytes));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .expect("fixed attachment filename is a valid header"),
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateJobRequest {
    #[serde(default)]
    cds_codes: Vec<String>,
    #[serde(default)]
    all_schools: bool,
    years: Vec<u16>,
    #[serde(default)]
    settings: PullSettings,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PullSettings {
    concurrency: usize,
    requests_per_second: f64,
    timeout_seconds: u64,
}

impl Default for PullSettings {
    fn default() -> Self {
        Self {
            concurrency: 50,
            requests_per_second: 1_000.0,
            timeout_seconds: 10,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Default, Serialize)]
struct JobProgress {
    completed: usize,
    total: usize,
    succeeded: usize,
    failed: usize,
}

#[derive(Clone)]
struct JobArtifacts {
    csv: Vec<u8>,
    xlsx: Vec<u8>,
    html: Vec<u8>,
    pdf: Vec<u8>,
}

struct JobRecord {
    id: String,
    status: JobStatus,
    created_at_unix_ms: u128,
    progress: JobProgress,
    error: Option<String>,
    rows: Vec<Value>,
    row_count: usize,
    quality: Option<Value>,
    artifacts: Option<JobArtifacts>,
}

#[derive(Serialize)]
struct CreateJobResponse {
    id: String,
    status: JobStatus,
}

async fn create_job(
    State(state): State<WebState>,
    Json(request): Json<CreateJobRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let mut cds_codes: Vec<_> = request
        .cds_codes
        .into_iter()
        .map(|code| code.trim().to_owned())
        .filter(|code| !code.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let years: Vec<_> = request
        .years
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if request.all_schools && !cds_codes.is_empty() {
        return Err(ApiError::bad_request(
            "choose either explicit schools or all active schools, not both",
        ));
    }
    if !request.all_schools && cds_codes.is_empty() {
        return Err(ApiError::bad_request("choose at least one school"));
    }
    if years.is_empty() {
        return Err(ApiError::bad_request("choose at least one reporting year"));
    }
    cds_codes.sort();

    validate_settings(&request.settings)?;
    let total = if request.all_schools {
        state
            .school_count()
            .checked_mul(years.len())
            .ok_or_else(|| ApiError::bad_request("the requested job is too large"))?
    } else {
        cds_codes
            .len()
            .checked_mul(years.len())
            .ok_or_else(|| ApiError::bad_request("the requested job is too large"))?
    };
    if total > MAX_JOB_REQUESTS {
        return Err(ApiError::bad_request(format!(
            "the requested job has {total} pulls; the local safety limit is {MAX_JOB_REQUESTS}"
        )));
    }

    let mut specs = if request.all_schools {
        state.inner.resolver.all_fetch_specs(&years)
    } else {
        state.inner.resolver.fetch_specs(&cds_codes, &years)
    }
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if let Some(base_url) = &state.inner.base_url_override {
        apply_base_url(&mut specs, base_url);
    }

    let id = new_job_id();
    let mut jobs = state.inner.jobs.write().await;
    prune_finished_jobs(&mut jobs);
    jobs.insert(
        id.clone(),
        JobRecord {
            id: id.clone(),
            status: JobStatus::Queued,
            created_at_unix_ms: unix_time_millis(),
            progress: JobProgress {
                total,
                ..JobProgress::default()
            },
            error: None,
            rows: Vec::new(),
            row_count: 0,
            quality: None,
            artifacts: None,
        },
    );
    drop(jobs);

    let client_config = client_config_for(&state.inner.default_client_config, &request.settings);
    let task_state = state.clone();
    let task_id = id.clone();
    tokio::spawn(async move {
        run_job(task_state, task_id, specs, client_config).await;
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(CreateJobResponse {
            id,
            status: JobStatus::Queued,
        }),
    ))
}

fn validate_settings(settings: &PullSettings) -> Result<(), ApiError> {
    if !(1..=64).contains(&settings.concurrency) {
        return Err(ApiError::bad_request(
            "concurrency must be between 1 and 64",
        ));
    }
    if !settings.requests_per_second.is_finite()
        || !(1.0..=1000.0).contains(&settings.requests_per_second)
    {
        return Err(ApiError::bad_request(
            "requests_per_second must be between 1 and 1000",
        ));
    }
    if !(1..=120).contains(&settings.timeout_seconds) {
        return Err(ApiError::bad_request(
            "timeout_seconds must be between 1 and 120",
        ));
    }
    Ok(())
}

fn client_config_for(defaults: &ClientConfig, settings: &PullSettings) -> ClientConfig {
    ClientConfig {
        concurrency: settings.concurrency,
        max_requests_per_second: settings.requests_per_second,
        timeout: Duration::from_secs(settings.timeout_seconds),
        max_payload_bytes: defaults.max_payload_bytes,
    }
}

/// Rewrites only trusted, CLI-configured test endpoints. API callers cannot set this value.
pub fn apply_base_url(specs: &mut [FetchSpec], base_url: &str) {
    let base_url = base_url.trim_end_matches('/');
    for spec in specs {
        spec.url = format!(
            "{}/{}/{}/SummaryCards",
            base_url, spec.school.cds_code, spec.dashboard_year_id
        );
    }
}

async fn run_job(state: WebState, id: String, specs: Vec<FetchSpec>, config: ClientConfig) {
    let _job_permit = match state.inner.job_gate.acquire().await {
        Ok(permit) => permit,
        Err(_) => {
            fail_job(&state, &id, "job scheduler stopped".to_owned()).await;
            return;
        }
    };
    set_job_running(&state, &id).await;
    let chunk_size = config.concurrency.max(1);
    let client = match DashboardClient::new(config) {
        Ok(client) => client,
        Err(error) => {
            fail_job(&state, &id, error.to_string()).await;
            return;
        }
    };

    let mut outcomes = Vec::with_capacity(specs.len());
    for chunk in specs.chunks(chunk_size) {
        let chunk_outcomes = client.fetch_all(chunk).await;
        outcomes.extend(chunk_outcomes);
        let completed = outcomes.len();
        let succeeded = outcomes
            .iter()
            .filter(|outcome| outcome.is_success())
            .count();
        update_job_progress(&state, &id, completed, succeeded).await;
    }

    let report = ReportModel::from_outcomes(&outcomes);
    let exports = match ExportBundle::from_report(&report) {
        Ok(exports) => exports,
        Err(error) => {
            fail_job(&state, &id, format!("report export failed: {error}")).await;
            return;
        }
    };

    let row_count = report.rows.len();
    let preview_rows = report
        .rows
        .iter()
        .take(MAX_ROW_PREVIEW_LIMIT)
        .collect::<Vec<_>>();
    let rows = match serde_json::to_value(preview_rows) {
        Ok(Value::Array(rows)) => rows,
        Ok(_) => Vec::new(),
        Err(error) => {
            fail_job(&state, &id, format!("result serialization failed: {error}")).await;
            return;
        }
    };
    let quality = serde_json::to_value(&report.quality).ok();
    let artifacts = JobArtifacts {
        csv: exports.csv_bytes,
        xlsx: exports.xlsx_bytes,
        html: exports.html_string.into_bytes(),
        pdf: exports.pdf_bytes,
    };

    let mut jobs = state.inner.jobs.write().await;
    if let Some(job) = jobs.get_mut(&id) {
        job.status = JobStatus::Completed;
        job.rows = rows;
        job.row_count = row_count;
        job.quality = quality;
        job.artifacts = Some(artifacts);
        job.progress.completed = job.progress.total;
        job.progress.succeeded = outcomes
            .iter()
            .filter(|outcome| outcome.is_success())
            .count();
        job.progress.failed = job.progress.total.saturating_sub(job.progress.succeeded);
    }
}

fn prune_finished_jobs(jobs: &mut HashMap<String, JobRecord>) {
    while jobs.len() >= MAX_RETAINED_JOBS {
        let oldest = jobs
            .iter()
            .filter(|(_, job)| matches!(job.status, JobStatus::Completed | JobStatus::Failed))
            .min_by_key(|(_, job)| job.created_at_unix_ms)
            .map(|(id, _)| id.clone());
        let Some(oldest) = oldest else {
            break;
        };
        jobs.remove(&oldest);
    }
}

async fn set_job_running(state: &WebState, id: &str) {
    if let Some(job) = state.inner.jobs.write().await.get_mut(id) {
        job.status = JobStatus::Running;
    }
}

async fn update_job_progress(state: &WebState, id: &str, completed: usize, succeeded: usize) {
    if let Some(job) = state.inner.jobs.write().await.get_mut(id) {
        job.progress.completed = completed.min(job.progress.total);
        job.progress.succeeded = succeeded;
        job.progress.failed = completed.saturating_sub(succeeded);
    }
}

async fn fail_job(state: &WebState, id: &str, error: String) {
    if let Some(job) = state.inner.jobs.write().await.get_mut(id) {
        job.status = JobStatus::Failed;
        job.error = Some(error);
    }
}

#[derive(Serialize)]
struct JobStatusResponse {
    id: String,
    status: JobStatus,
    created_at_unix_ms: u128,
    progress: JobProgress,
    error: Option<String>,
    row_count: usize,
    quality: Option<Value>,
    downloads: Option<HashMap<&'static str, String>>,
}

async fn job_status(
    State(state): State<WebState>,
    Path(id): Path<String>,
) -> Result<Json<JobStatusResponse>, ApiError> {
    let jobs = state.inner.jobs.read().await;
    let job = jobs.get(&id).ok_or_else(ApiError::not_found)?;
    let downloads = job.artifacts.as_ref().map(|_| {
        ["csv", "xlsx", "html", "pdf"]
            .into_iter()
            .map(|format| (format, format!("/api/jobs/{}/downloads/{format}", job.id)))
            .collect()
    });
    Ok(Json(JobStatusResponse {
        id: job.id.clone(),
        status: job.status,
        created_at_unix_ms: job.created_at_unix_ms,
        progress: job.progress.clone(),
        error: job.error.clone(),
        row_count: job.row_count,
        quality: job.quality.clone(),
        downloads,
    }))
}

#[derive(Debug, Deserialize)]
struct RowQuery {
    #[serde(default = "default_row_limit")]
    limit: usize,
}

const fn default_row_limit() -> usize {
    500
}

async fn job_rows(
    State(state): State<WebState>,
    Path(id): Path<String>,
    Query(query): Query<RowQuery>,
) -> Result<Json<Value>, ApiError> {
    let jobs = state.inner.jobs.read().await;
    let job = jobs.get(&id).ok_or_else(ApiError::not_found)?;
    if job.status != JobStatus::Completed {
        return Err(ApiError::conflict("the report is not complete yet"));
    }
    let limit = query.limit.min(MAX_ROW_PREVIEW_LIMIT);
    let rows: Vec<_> = job.rows.iter().take(limit).cloned().collect();
    Ok(Json(json!({
        "rows": rows,
        "returned": rows.len(),
        "total": job.row_count,
        "truncated": rows.len() < job.row_count,
    })))
}

async fn download(
    State(state): State<WebState>,
    Path((id, format)): Path<(String, String)>,
) -> Result<Response<Body>, ApiError> {
    let jobs = state.inner.jobs.read().await;
    let job = jobs.get(&id).ok_or_else(ApiError::not_found)?;
    let artifacts = job
        .artifacts
        .as_ref()
        .ok_or_else(|| ApiError::conflict("the report is not complete yet"))?;
    let (body, content_type, extension) = match format.as_str() {
        "csv" => (artifacts.csv.clone(), "text/csv; charset=utf-8", "csv"),
        "xlsx" => (
            artifacts.xlsx.clone(),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "xlsx",
        ),
        "html" => (artifacts.html.clone(), "text/html; charset=utf-8", "html"),
        "pdf" => (artifacts.pdf.clone(), "application/pdf", "pdf"),
        _ => {
            return Err(ApiError::bad_request(
                "format must be csv, xlsx, html, or pdf",
            ));
        }
    };

    let mut response = Response::new(Body::from(body));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"california-dashboard-report.{extension}\""
        ))
        .expect("fixed report filename is a valid header"),
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok(response)
}

async fn not_found() -> ApiError {
    ApiError::not_found()
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }

    fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "resource not found".to_owned(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            [(CACHE_CONTROL, "no-store")],
            Json(json!({ "error": self.message })),
        )
            .into_response()
    }
}

fn new_job_id() -> String {
    let sequence = NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{sequence:x}", unix_time_millis())
}

fn unix_time_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SchoolRecord;

    #[test]
    fn base_url_override_preserves_identity_and_dashboard_year() {
        let mut specs = vec![FetchSpec {
            school: SchoolRecord {
                cds_code: "01000010000001".to_owned(),
                school_name: "Example".to_owned(),
                county: None,
                district: None,
                city: None,
            },
            year: 2025,
            dashboard_year_id: 11,
            url: "unused".to_owned(),
        }];
        apply_base_url(&mut specs, "http://127.0.0.1:9000/Reports/");
        assert_eq!(
            specs[0].url,
            "http://127.0.0.1:9000/Reports/01000010000001/11/SummaryCards"
        );
    }

    #[test]
    fn pull_setting_bounds_are_enforced() {
        let defaults = PullSettings::default();
        assert_eq!(defaults.concurrency, 50);
        assert_eq!(defaults.requests_per_second, 1_000.0);
        assert_eq!(defaults.timeout_seconds, 10);
        assert!(validate_settings(&defaults).is_ok());
        assert!(
            validate_settings(&PullSettings {
                concurrency: 0,
                ..PullSettings::default()
            })
            .is_err()
        );
        assert!(
            validate_settings(&PullSettings {
                requests_per_second: f64::NAN,
                ..PullSettings::default()
            })
            .is_err()
        );
    }

    #[test]
    fn compiled_ui_exposes_all_active_and_directory_workflows() {
        for required in [
            "id=\"all-active-schools\"",
            "href=\"/api/directory.csv\"",
            "href=\"/api/directory.xlsx\"",
            "id=\"download-csv\"",
            "id=\"download-xlsx\"",
            "id=\"download-html\"",
            "id=\"download-pdf\"",
        ] {
            assert!(
                INDEX_HTML.contains(required),
                "missing UI surface {required}"
            );
        }
        assert!(APP_JS.contains("all_schools: state.allSchools"));
    }
}
