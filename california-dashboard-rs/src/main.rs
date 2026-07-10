use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use california_dashboard::web::{WebState, apply_base_url, router};
use california_dashboard::{
    ClientConfig, DashboardClient, ExportBundle, ReportModel, SchoolResolver, directory_csv_bytes,
    directory_xlsx_bytes, import_school_csv, validate_school_database,
};
use clap::{Args, Parser, Subcommand};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "california-dashboard",
    version,
    about = "Pull and responsibly report California School Dashboard data"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the private local dashboard interface.
    Serve(ServeArgs),
    /// Pull selected schools and write CSV, Excel, HTML, and PDF exports.
    Pull(PullArgs),
    /// Build or replace the SQLite school cache from CDE's CSV file.
    ImportSchools(ImportArgs),
    /// Compare the SQLite school cache with its source CSV.
    ValidateSchools(ValidateArgs),
    /// Export the complete CDE-style school and district directory as CSV and Excel.
    ExportDirectory(DirectoryArgs),
}

#[derive(Debug, Args)]
struct ServeArgs {
    /// School SQLite database. If omitted, current and parent directories are searched.
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,
    /// Interface to listen on. The loopback default keeps the UI local.
    #[arg(long, default_value_t = IpAddr::V4(Ipv4Addr::LOCALHOST))]
    host: IpAddr,
    /// TCP port for the local interface.
    #[arg(long, default_value_t = 8787)]
    port: u16,
    /// Override the Dashboard Reports base URL (intended for local integration tests).
    #[arg(long, env = "CALIFORNIA_DASHBOARD_BASE_URL")]
    base_url: Option<String>,
    #[command(flatten)]
    client: ClientArgs,
}

#[derive(Debug, Args)]
struct PullArgs {
    /// School SQLite database. If omitted, current and parent directories are searched.
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,
    /// Exact 14-digit CDS code. Repeat this option to select multiple schools.
    #[arg(
        long = "school-cds",
        value_name = "CDS",
        action = clap::ArgAction::Append,
        required_unless_present = "all",
        conflicts_with = "all"
    )]
    school_cds: Vec<String>,
    /// Pull every active school in the local database.
    #[arg(long)]
    all: bool,
    /// Dashboard years, repeated or comma-delimited.
    #[arg(long, value_delimiter = ',', default_value = "2021,2022,2023,2024")]
    years: Vec<u16>,
    /// Directory in which all four exports will be written.
    #[arg(long, default_value = "dashboard-output")]
    output: PathBuf,
    /// File stem shared by the four exports.
    #[arg(long, default_value = "california-dashboard-report")]
    stem: String,
    /// Override the Dashboard Reports base URL (intended for local integration tests).
    #[arg(long, env = "CALIFORNIA_DASHBOARD_BASE_URL")]
    base_url: Option<String>,
    #[command(flatten)]
    client: ClientArgs,
}

#[derive(Debug, Clone, Args)]
struct ClientArgs {
    /// Maximum concurrent HTTP requests.
    #[arg(long, default_value_t = 50, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..=64))]
    concurrency: usize,
    /// Global request-start ceiling per second.
    #[arg(long, default_value_t = 1000.0)]
    requests_per_second: f64,
    /// Per-request timeout in seconds.
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..=120))]
    timeout_seconds: u64,
    /// Maximum accepted response size in mebibytes.
    #[arg(long, default_value_t = 16, value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..=256))]
    max_payload_mib: usize,
}

impl ClientArgs {
    fn config(&self) -> Result<ClientConfig> {
        ensure!(
            self.requests_per_second.is_finite() && self.requests_per_second > 0.0,
            "--requests-per-second must be finite and greater than zero"
        );
        let config = ClientConfig {
            concurrency: self.concurrency,
            max_requests_per_second: self.requests_per_second,
            timeout: Duration::from_secs(self.timeout_seconds),
            max_payload_bytes: self
                .max_payload_mib
                .checked_mul(1024 * 1024)
                .context("--max-payload-mib is too large")?,
        };
        config.validate()?;
        Ok(config)
    }
}

#[derive(Debug, Args)]
struct ImportArgs {
    /// CDE public-school CSV source.
    #[arg(long, default_value = "pubschls.csv")]
    csv: PathBuf,
    /// SQLite cache to create or replace.
    #[arg(long, default_value = "pubschls.db")]
    db: PathBuf,
}

#[derive(Debug, Args)]
struct ValidateArgs {
    /// CDE public-school CSV source.
    #[arg(long, default_value = "pubschls.csv")]
    csv: PathBuf,
    /// SQLite cache to compare.
    #[arg(long, default_value = "pubschls.db")]
    db: PathBuf,
    /// Print counts instead of field-level JSON details.
    #[arg(long)]
    summary: bool,
}

#[derive(Debug, Args)]
struct DirectoryArgs {
    /// School SQLite database. If omitted, current and parent directories are searched.
    #[arg(long, value_name = "PATH")]
    db: Option<PathBuf>,
    /// Directory in which the CSV and Excel files will be written.
    #[arg(long, default_value = "dashboard-output")]
    output: PathBuf,
    /// File stem shared by the two directory exports.
    #[arg(long, default_value = "cde-school-directory")]
    stem: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    match Cli::parse().command {
        Command::Serve(arguments) => serve(arguments).await,
        Command::Pull(arguments) => pull(arguments).await,
        Command::ImportSchools(arguments) => import_schools(arguments),
        Command::ValidateSchools(arguments) => validate_schools(arguments),
        Command::ExportDirectory(arguments) => export_directory(arguments).await,
    }
}

async fn serve(arguments: ServeArgs) -> Result<()> {
    let database = discover_database(arguments.db)?;
    let resolver = SchoolResolver::open(&database)
        .with_context(|| format!("open school database {}", database.display()))?;
    let state = WebState::new(resolver, arguments.client.config()?, arguments.base_url)
        .map_err(anyhow::Error::msg)?;
    let address = SocketAddr::new(arguments.host, arguments.port);
    let listener = TcpListener::bind(address)
        .await
        .with_context(|| format!("listen on {address}"))?;
    let actual_address = listener.local_addr()?;

    println!("California Dashboard Workbench is ready at http://{actual_address}");
    println!("Using school database: {}", database.display());
    if !actual_address.ip().is_loopback() {
        eprintln!(
            "Warning: this server is reachable beyond this computer; use 127.0.0.1 for local-only access."
        );
    }

    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("local web server stopped unexpectedly")
}

async fn pull(arguments: PullArgs) -> Result<()> {
    validate_stem(&arguments.stem)?;
    let database = discover_database(arguments.db)?;
    let resolver = SchoolResolver::open(&database)
        .with_context(|| format!("open school database {}", database.display()))?;
    let years: Vec<_> = arguments
        .years
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut specs = if arguments.all {
        resolver.all_fetch_specs(&years)?
    } else {
        let cds_codes: Vec<_> = arguments
            .school_cds
            .into_iter()
            .map(|code| code.trim().to_owned())
            .filter(|code| !code.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        ensure!(!cds_codes.is_empty(), "provide at least one --school-cds");
        resolver.fetch_specs(&cds_codes, &years)?
    };
    if let Some(base_url) = arguments.base_url.as_deref() {
        apply_base_url(&mut specs, base_url);
    }

    println!("Pulling {} school/year requests…", specs.len());
    let client = DashboardClient::new(arguments.client.config()?)?;
    let outcomes = client.fetch_all(&specs).await;
    let succeeded = outcomes
        .iter()
        .filter(|outcome| outcome.is_success())
        .count();
    let report = ReportModel::from_outcomes(&outcomes);
    let exports = ExportBundle::from_report(&report)?;
    write_exports(&arguments.output, &arguments.stem, &exports).await?;

    println!(
        "Completed {}/{} requests; wrote CSV, Excel, HTML, and PDF to {}",
        succeeded,
        outcomes.len(),
        arguments.output.display()
    );
    if succeeded < outcomes.len() {
        eprintln!(
            "{} request(s) failed. Failure provenance is preserved in the report and fetch log.",
            outcomes.len() - succeeded
        );
    }
    Ok(())
}

async fn write_exports(directory: &Path, stem: &str, exports: &ExportBundle) -> Result<()> {
    tokio::fs::create_dir_all(directory)
        .await
        .with_context(|| format!("create export directory {}", directory.display()))?;
    let files = [
        ("csv", exports.csv_bytes.as_slice()),
        ("xlsx", exports.xlsx_bytes.as_slice()),
        ("html", exports.html_string.as_bytes()),
        ("pdf", exports.pdf_bytes.as_slice()),
    ];
    for (extension, bytes) in files {
        let path = directory.join(format!("{stem}.{extension}"));
        atomic_write(&path, bytes).await?;
    }
    Ok(())
}

async fn export_directory(arguments: DirectoryArgs) -> Result<()> {
    validate_stem(&arguments.stem)?;
    let database = discover_database(arguments.db)?;
    let resolver = SchoolResolver::open(&database)
        .with_context(|| format!("open school database {}", database.display()))?;
    let csv = directory_csv_bytes(resolver.directory_records())?;
    let xlsx = directory_xlsx_bytes(resolver.directory_records())?;
    tokio::fs::create_dir_all(&arguments.output)
        .await
        .with_context(|| format!("create export directory {}", arguments.output.display()))?;
    atomic_write(
        &arguments.output.join(format!("{}.csv", arguments.stem)),
        &csv,
    )
    .await?;
    atomic_write(
        &arguments.output.join(format!("{}.xlsx", arguments.stem)),
        &xlsx,
    )
    .await?;
    println!(
        "Wrote {} school/district records as CSV and Excel to {}",
        resolver.directory_records().len(),
        arguments.output.display()
    );
    Ok(())
}

async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    let temporary = path.with_extension(format!("{extension}.tmp-{}", std::process::id()));
    tokio::fs::write(&temporary, bytes)
        .await
        .with_context(|| format!("write temporary export {}", temporary.display()))?;
    tokio::fs::rename(&temporary, path)
        .await
        .with_context(|| format!("publish export {}", path.display()))?;
    Ok(())
}

fn import_schools(arguments: ImportArgs) -> Result<()> {
    let summary = import_school_csv(&arguments.csv, &arguments.db).with_context(|| {
        format!(
            "import {} into {}",
            arguments.csv.display(),
            arguments.db.display()
        )
    })?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn validate_schools(arguments: ValidateArgs) -> Result<()> {
    let validation =
        validate_school_database(&arguments.csv, &arguments.db).with_context(|| {
            format!(
                "validate {} against {}",
                arguments.db.display(),
                arguments.csv.display()
            )
        })?;
    if arguments.summary {
        println!(
            "CSV rows: {}; database rows: {}; missing from database: {}; only in database: {}; changed: {}; duplicate CSV CDS: {}; duplicate database CDS: {}",
            validation.csv_row_count,
            validation.database_row_count,
            validation.only_in_csv.len(),
            validation.only_in_database.len(),
            validation.changed_rows.len(),
            validation.duplicate_csv_cds_codes.len(),
            validation.duplicate_database_cds_codes.len(),
        );
    } else {
        println!("{}", serde_json::to_string_pretty(&validation)?);
    }
    if !validation.is_exact_match() {
        bail!(
            "school database validation found {} issue(s)",
            validation.issue_count()
        );
    }
    Ok(())
}

fn discover_database(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        ensure!(
            path.is_file(),
            "school database not found: {}",
            path.display()
        );
        return Ok(path);
    }
    let current = std::env::current_dir().context("determine current directory")?;
    let mut candidates = vec![current.join("pubschls.db")];
    if let Some(parent) = current.parent() {
        candidates.push(parent.join("pubschls.db"));
    }
    let manifest_parent = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|parent| parent.join("pubschls.db"));
    if let Some(candidate) = manifest_parent
        && !candidates.contains(&candidate)
    {
        candidates.push(candidate);
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .context("pubschls.db was not found in the current or parent directory; pass --db PATH")
}

fn validate_stem(stem: &str) -> Result<()> {
    ensure!(!stem.trim().is_empty(), "--stem cannot be empty");
    ensure!(
        Path::new(stem).file_name().is_some_and(|name| name == stem),
        "--stem must be a file name, not a path"
    );
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_stem_cannot_escape_output_directory() {
        assert!(validate_stem("school-report").is_ok());
        assert!(validate_stem("../school-report").is_err());
        assert!(validate_stem("").is_err());
    }
}
