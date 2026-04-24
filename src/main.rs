use std::{
    collections::HashMap,
    ffi::OsString,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};

use app::{AppConfig, AppState, HealthState, MetricsStore};
use clap::{CommandFactory, Parser, parser::ValueSource};
use tokio::sync::RwLock;

mod app;
mod error;
mod graphql;
mod http;
mod mutation_service;
mod query;
mod relations;
mod schema;
mod sql;
mod storage;
mod watcher;

use http::routes::build_router;
use schema::{Schema, infer_schema_from_data_source, load_schema};
use storage::scan_resources;
use watcher::start_resource_watcher;

const CONFIG_FILE_NAME: &str = "dirbase.conf";
const DEFAULT_BIND_ADDR: &str = "127.0.0.1:4444";
const DEFAULT_LOGNAME: &str = "requests.log";
const DEFAULT_MAX_BODY_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_PER_PAGE: usize = 100;
const DEFAULT_MAX_SQL_SCAN_ROWS: usize = 50_000;
const DEFAULT_MAX_SQL_SELECTED_ROWS: usize = 1_000;
const CLI_HELP_AFTER: &str = "\
Examples:
  dirbase ./data
  dirbase ./db.json --bind 127.0.0.1:4444
  dirbase --folder ./data --port 5555
  dirbase --folder ./data --readonly
  dirbase --folder ./data --schema ./schema.dbml

Config file:
  If ./dirbase.conf exists, dirbase loads it automatically using the same CLI-style arguments.
  Explicit command-line arguments override dirbase.conf values.

Source selection:
  Use one of [PATH], --folder, or --file.
  [PATH] auto-detects file vs folder mode. Missing paths default to folder mode unless they end in .json.";

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Serve JSON resources from a folder or database file",
    next_line_help = true,
    after_help = CLI_HELP_AFTER
)]
struct CliArgs {
    #[arg(
        value_name = "PATH",
        conflicts_with_all = ["folder", "file"],
        help = "Path to a folder of *.json files or a single json-server-style database file."
    )]
    path: Option<PathBuf>,
    #[arg(
        short,
        long,
        conflicts_with_all = ["file", "path"],
        help = "Serve every *.json file in this folder as a resource."
    )]
    folder: Option<PathBuf>,
    #[arg(
        long,
        conflicts_with_all = ["folder", "path"],
        help = "Serve a single json-server-style database file."
    )]
    file: Option<PathBuf>,
    #[arg(
        short,
        long,
        help = "Listen address in HOST:PORT form.",
        long_help = "Listen address in HOST:PORT form. Use --port to override only the port while keeping the current host."
    )]
    bind: Option<SocketAddr>,
    #[arg(long, help = "Override only the listen port while keeping the current bind host.")]
    port: Option<u16>,
    #[arg(long, help = "Disable POST, PUT, PATCH, and DELETE routes.")]
    readonly: bool,
    #[arg(
        long,
        help = "Use an explicit schema file instead of auto-detecting schema.json or schema.dbml."
    )]
    schema: Option<PathBuf>,
    #[arg(long, help = "Enable request logging.")]
    log: bool,
    #[arg(long, help = "Write request logs to this file when --log is enabled.")]
    logname: Option<PathBuf>,
    #[arg(long, help = "Require this bearer token for application routes.")]
    auth_token: Option<String>,
    #[arg(long, help = "Allow CORS requests from this single origin.")]
    cors_origin: Option<String>,
    #[arg(long, help = "Reject request bodies larger than this many bytes.")]
    max_body_bytes: Option<usize>,
    #[arg(long, help = "Cap REST pagination to this many rows per page.")]
    max_per_page: Option<usize>,
    #[arg(long, help = "Cap how many rows SQL queries may scan before returning an error.")]
    max_sql_scan_rows: Option<usize>,
    #[arg(long, help = "Cap how many rows SQL queries may return.")]
    max_sql_selected_rows: Option<usize>,
}

#[derive(Debug)]
struct Cli {
    path: Option<PathBuf>,
    folder: Option<PathBuf>,
    file: Option<PathBuf>,
    bind: SocketAddr,
    readonly: bool,
    schema: Option<PathBuf>,
    log: bool,
    logname: PathBuf,
    auth_token: Option<String>,
    cors_origin: Option<String>,
    max_body_bytes: usize,
    max_per_page: usize,
    max_sql_scan_rows: usize,
    max_sql_selected_rows: usize,
}

enum CliLoadError {
    CommandLine(clap::Error),
    Config(String),
}

struct StartupSummary {
    source_kind: &'static str,
    source_path: String,
    resource_count: usize,
    schema_status: &'static str,
    mode: &'static str,
}

#[tokio::main]
async fn main() {
    let cli = match load_cli() {
        Ok(Some(cli)) => cli,
        Ok(None) => return,
        Err(CliLoadError::CommandLine(err)) => err.exit(),
        Err(CliLoadError::Config(message)) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    let _guard = if cli.log {
        let file_appender = tracing_appender::rolling::never(
            cli.logname.parent().unwrap_or(std::path::Path::new(".")),
            cli.logname.file_name().and_then(|n| n.to_str()).unwrap_or("requests.log"),
        );
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::fmt().with_writer(non_blocking).init();
        Some(guard)
    } else {
        tracing_subscriber::fmt::init();
        None
    };

    let data_source = resolve_data_source(&cli).await;

    let schema_root = match &data_source {
        app::DataSource::Folder(folder) => folder.clone(),
        app::DataSource::File(file) => {
            file.parent().map(|parent| parent.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
        }
    };

    let declared_schema = match load_schema(&schema_root, cli.schema.as_deref()) {
        Ok(schema) => schema,
        Err(err) => {
            eprintln!("Failed to load schema: {err}");
            std::process::exit(1);
        }
    };

    let initial_resources = scan_resources(&data_source).unwrap_or_default();
    let (inferred_schema, health) =
        match infer_schema_from_data_source(&data_source, &initial_resources) {
            Ok(schema) => (schema, Arc::new(HealthState::new(true, None))),
            Err(err) => {
                eprintln!("Failed to infer schema: {err}");
                (Schema::default(), Arc::new(HealthState::new(false, Some(err))))
            }
        };
    let startup_summary = StartupSummary {
        source_kind: data_source_kind_label(&data_source),
        source_path: data_source_path_label(&data_source),
        resource_count: initial_resources.len(),
        schema_status: schema_status_label(&declared_schema, &inferred_schema),
        mode: if cli.readonly { "readonly" } else { "read-write" },
    };
    let config = Arc::new(AppConfig {
        readonly: cli.readonly,
        enable_log: cli.log,
        auth_token: cli.auth_token.clone(),
        cors_origin: cli.cors_origin.clone(),
        max_body_bytes: cli.max_body_bytes,
        max_per_page: cli.max_per_page,
        max_sql_scan_rows: cli.max_sql_scan_rows,
        max_sql_selected_rows: cli.max_sql_selected_rows,
    });
    let metrics = Arc::new(MetricsStore::default());
    let (event_bus, _) = tokio::sync::broadcast::channel(256);
    let state = AppState {
        data_source: Arc::new(data_source),
        config,
        resources: Arc::new(RwLock::new(initial_resources)),
        resource_cache: Arc::new(RwLock::new(HashMap::new())),
        resource_locks: Arc::new(RwLock::new(HashMap::new())),
        schema_store: Arc::new(std::sync::RwLock::new(
            app::SchemaStore::new(declared_schema, inferred_schema).unwrap_or_else(|err| {
                eprintln!("Failed to build schema: {err}");
                std::process::exit(1);
            }),
        )),
        graphql_store: Arc::new(RwLock::new(app::GraphqlStore::default())),
        metrics,
        health,
        event_bus,
    };

    start_resource_watcher(
        state.data_source.clone(),
        state.resources.clone(),
        state.resource_cache.clone(),
        state.schema_store.clone(),
        state.graphql_store.clone(),
        state.health.clone(),
        state.clone(),
    );

    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind(cli.bind).await.expect("binding server listener");
    let listen_addr = listener.local_addr().expect("reading server listener address");
    let browser_url = browser_url_for(listen_addr);
    tracing::info!(readonly = cli.readonly, "Readonly mode");
    tracing::info!(listen_addr = %listen_addr, browser_url = %browser_url, "Server started");
    print_startup_summary(&browser_url, &cli, &startup_summary);
    axum::serve(listener, app).await.expect("running server");
}

fn load_cli() -> Result<Option<Cli>, CliLoadError> {
    let args: Vec<OsString> = std::env::args_os().collect();
    let config_path = std::env::current_dir()
        .map_err(|err| {
            CliLoadError::Config(format!(
                "Failed to inspect current directory for {CONFIG_FILE_NAME}: {err}"
            ))
        })?
        .join(CONFIG_FILE_NAME);
    let config_tokens = load_config_tokens(&config_path)?;

    if args.len() == 1 && config_tokens.is_none() {
        let mut command = CliArgs::command();
        command.print_help().expect("print CLI help");
        println!();
        return Ok(None);
    }

    let cli_matches =
        CliArgs::command().try_get_matches_from(args).map_err(CliLoadError::CommandLine)?;
    let config_matches = match config_tokens {
        Some(config_args) => {
            Some(CliArgs::command().try_get_matches_from(config_args).map_err(|err| {
                CliLoadError::Config(format!(
                    "Failed to parse {CONFIG_FILE_NAME}: {}",
                    err.render().ansi()
                ))
            })?)
        }
        None => None,
    };

    Ok(Some(resolve_cli(&cli_matches, config_matches.as_ref())))
}

fn load_config_tokens(path: &std::path::Path) -> Result<Option<Vec<OsString>>, CliLoadError> {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let mut args = vec![OsString::from("dirbase")];
            args.extend(
                parse_config_args(&contents)
                    .map_err(|err| {
                        CliLoadError::Config(format!("Failed to parse {}: {err}", path.display()))
                    })?
                    .into_iter()
                    .map(OsString::from),
            );
            Ok(Some(args))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(CliLoadError::Config(format!("Failed to read {}: {err}", path.display()))),
    }
}

fn parse_config_args(contents: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = contents.chars();
    let mut quote = None;

    while let Some(ch) = chars.next() {
        if let Some(active_quote) = quote {
            match ch {
                '\\' => current.push(
                    chars
                        .next()
                        .ok_or_else(|| "Trailing escape sequence in quoted value".to_string())?,
                ),
                _ if ch == active_quote => quote = None,
                _ => current.push(ch),
            }
            continue;
        }

        match ch {
            '"' | '\'' => quote = Some(ch),
            '\\' => current.push(
                chars
                    .next()
                    .ok_or_else(|| "Trailing escape sequence in config file".to_string())?,
            ),
            '#' if current.is_empty() => {
                for comment_char in chars.by_ref() {
                    if comment_char == '\n' {
                        break;
                    }
                }
            }
            _ if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }

    if quote.is_some() {
        return Err("Unterminated quoted value in config file".to_string());
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    Ok(tokens)
}

fn resolve_cli(cli_matches: &clap::ArgMatches, config_matches: Option<&clap::ArgMatches>) -> Cli {
    let (path, folder, file) = resolve_data_source_args(cli_matches, config_matches);

    Cli {
        path,
        folder,
        file,
        bind: resolve_bind_addr(cli_matches, config_matches),
        readonly: resolve_flag("readonly", cli_matches, config_matches),
        schema: resolve_value("schema", cli_matches, config_matches),
        log: resolve_flag("log", cli_matches, config_matches),
        logname: resolve_value("logname", cli_matches, config_matches)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_LOGNAME)),
        auth_token: resolve_value("auth_token", cli_matches, config_matches),
        cors_origin: resolve_value("cors_origin", cli_matches, config_matches),
        max_body_bytes: resolve_value("max_body_bytes", cli_matches, config_matches)
            .unwrap_or(DEFAULT_MAX_BODY_BYTES),
        max_per_page: resolve_value("max_per_page", cli_matches, config_matches)
            .unwrap_or(DEFAULT_MAX_PER_PAGE),
        max_sql_scan_rows: resolve_value("max_sql_scan_rows", cli_matches, config_matches)
            .unwrap_or(DEFAULT_MAX_SQL_SCAN_ROWS),
        max_sql_selected_rows: resolve_value("max_sql_selected_rows", cli_matches, config_matches)
            .unwrap_or(DEFAULT_MAX_SQL_SELECTED_ROWS),
    }
}

fn resolve_bind_addr(
    cli_matches: &clap::ArgMatches,
    config_matches: Option<&clap::ArgMatches>,
) -> SocketAddr {
    let mut bind = config_matches
        .filter(|matches| matches.value_source("bind") == Some(ValueSource::CommandLine))
        .and_then(|matches| matches.get_one::<SocketAddr>("bind").copied())
        .unwrap_or_else(default_bind_addr);

    if let Some(port) = config_matches
        .filter(|matches| matches.value_source("port") == Some(ValueSource::CommandLine))
        .and_then(|matches| matches.get_one::<u16>("port").copied())
    {
        bind.set_port(port);
    }

    if let Some(cli_bind) = cli_matches
        .value_source("bind")
        .filter(|source| *source == ValueSource::CommandLine)
        .and_then(|_| cli_matches.get_one::<SocketAddr>("bind").copied())
    {
        bind = cli_bind;
    }

    if let Some(cli_port) = cli_matches
        .value_source("port")
        .filter(|source| *source == ValueSource::CommandLine)
        .and_then(|_| cli_matches.get_one::<u16>("port").copied())
    {
        bind.set_port(cli_port);
    }

    bind
}

fn resolve_data_source_args(
    cli_matches: &clap::ArgMatches,
    config_matches: Option<&clap::ArgMatches>,
) -> (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>) {
    let cli_has_source = ["path", "folder", "file"]
        .iter()
        .any(|id| cli_matches.value_source(id) == Some(ValueSource::CommandLine));
    if cli_has_source {
        return (
            cli_matches.get_one::<PathBuf>("path").cloned(),
            cli_matches.get_one::<PathBuf>("folder").cloned(),
            cli_matches.get_one::<PathBuf>("file").cloned(),
        );
    }

    if let Some(config_matches) = config_matches {
        let config_has_source = ["path", "folder", "file"]
            .iter()
            .any(|id| config_matches.value_source(id) == Some(ValueSource::CommandLine));
        if config_has_source {
            return (
                config_matches.get_one::<PathBuf>("path").cloned(),
                config_matches.get_one::<PathBuf>("folder").cloned(),
                config_matches.get_one::<PathBuf>("file").cloned(),
            );
        }
    }

    (None, None, None)
}

fn resolve_value<T: Clone + Send + Sync + 'static>(
    id: &str,
    cli_matches: &clap::ArgMatches,
    config_matches: Option<&clap::ArgMatches>,
) -> Option<T> {
    if cli_matches.value_source(id) == Some(ValueSource::CommandLine) {
        return cli_matches.get_one::<T>(id).cloned();
    }

    config_matches.and_then(|matches| {
        (matches.value_source(id) == Some(ValueSource::CommandLine))
            .then(|| matches.get_one::<T>(id).cloned())
            .flatten()
    })
}

fn resolve_flag(
    id: &str,
    cli_matches: &clap::ArgMatches,
    config_matches: Option<&clap::ArgMatches>,
) -> bool {
    if cli_matches.value_source(id) == Some(ValueSource::CommandLine) {
        return cli_matches.get_flag(id);
    }

    config_matches
        .filter(|matches| matches.value_source(id) == Some(ValueSource::CommandLine))
        .map(|matches| matches.get_flag(id))
        .unwrap_or(false)
}

fn default_bind_addr() -> SocketAddr {
    DEFAULT_BIND_ADDR.parse().expect("valid default bind address")
}

fn browser_url_for(addr: SocketAddr) -> String {
    let browser_addr = match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), addr.port())
        }
        IpAddr::V6(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), addr.port())
        }
        _ => addr,
    };

    format!("http://{browser_addr}/")
}

fn data_source_kind_label(data_source: &app::DataSource) -> &'static str {
    match data_source {
        app::DataSource::Folder(_) => "folder",
        app::DataSource::File(_) => "file",
    }
}

fn data_source_path_label(data_source: &app::DataSource) -> String {
    match data_source {
        app::DataSource::Folder(path) | app::DataSource::File(path) => path.display().to_string(),
    }
}

fn schema_status_label(
    declared_schema: &Option<schema::DeclaredSchema>,
    inferred_schema: &Schema,
) -> &'static str {
    if declared_schema.as_ref().is_some_and(|schema| !schema.tables.is_empty()) {
        "loaded"
    } else if !inferred_schema.tables.is_empty() {
        "inferred"
    } else {
        "none"
    }
}

fn print_startup_summary(browser_url: &str, cli: &Cli, summary: &StartupSummary) {
    eprintln!("Open {browser_url}");
    eprintln!("Source: {} {}", summary.source_kind, summary.source_path);
    eprintln!("Resources: {}", summary.resource_count);
    eprintln!("Schema: {}", summary.schema_status);
    eprintln!("Mode: {}", summary.mode);
    if cli.auth_token.is_some() {
        eprintln!("Auth: bearer token enabled");
    }
    if let Some(origin) = &cli.cors_origin {
        eprintln!("CORS: {origin}");
    }
}

async fn resolve_data_source(cli: &Cli) -> app::DataSource {
    if let Some(file) = cli.file.clone() {
        if let Err(err) = tokio::fs::try_exists(&file).await {
            eprintln!("Failed to inspect data file {}: {err}", file.display());
            std::process::exit(1);
        }
        return app::DataSource::File(file);
    }

    if let Some(folder) = cli.folder.clone() {
        ensure_folder_exists(&folder).await;
        return app::DataSource::Folder(folder);
    }

    if let Some(path) = cli.path.clone() {
        match tokio::fs::metadata(&path).await {
            Ok(metadata) if metadata.is_file() => return app::DataSource::File(path),
            Ok(metadata) if metadata.is_dir() => return app::DataSource::Folder(path),
            Ok(_) => {
                eprintln!("Path {} is neither a regular file nor a directory", path.display());
                std::process::exit(1);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                    return app::DataSource::File(path);
                }
                ensure_folder_exists(&path).await;
                return app::DataSource::Folder(path);
            }
            Err(err) => {
                eprintln!("Failed to inspect path {}: {err}", path.display());
                std::process::exit(1);
            }
        }
    }

    let folder = PathBuf::from("./data");
    ensure_folder_exists(&folder).await;
    app::DataSource::Folder(folder)
}

async fn ensure_folder_exists(folder: &std::path::Path) {
    if let Err(err) = tokio::fs::create_dir_all(folder).await {
        eprintln!("Failed to create data folder {}: {err}", folder.display());
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, path::PathBuf};

    use super::{CliArgs, browser_url_for, parse_config_args, resolve_cli};
    use clap::CommandFactory;

    fn resolve_test_cli(cli_args: &[&str], config_args: &[&str]) -> super::Cli {
        let cli_matches = matches_for(cli_args);
        let config_matches = (!config_args.is_empty()).then(|| matches_for(config_args));
        resolve_cli(&cli_matches, config_matches.as_ref())
    }

    fn matches_for(args: &[&str]) -> clap::ArgMatches {
        let argv = std::iter::once("dirbase").chain(args.iter().copied()).collect::<Vec<_>>();
        CliArgs::command().try_get_matches_from(argv).expect("matches")
    }

    #[test]
    fn browser_url_preserves_specific_bind_addresses() {
        let addr = "127.0.0.1:4444".parse().expect("socket addr");
        assert_eq!(browser_url_for(addr), "http://127.0.0.1:4444/");
    }

    #[test]
    fn browser_url_maps_unspecified_ipv4_to_loopback() {
        let addr = "0.0.0.0:4444".parse().expect("socket addr");
        assert_eq!(browser_url_for(addr), "http://127.0.0.1:4444/");
    }

    #[test]
    fn browser_url_maps_unspecified_ipv6_to_loopback() {
        let addr = "[::]:4444".parse().expect("socket addr");
        assert_eq!(browser_url_for(addr), "http://[::1]:4444/");
    }

    #[test]
    fn config_parser_supports_quotes_comments_and_escapes() {
        let parsed = parse_config_args(
            "--folder \"my data\" # comment\n--auth-token a\\ b\n'file name.json'\n",
        )
        .expect("config args");

        assert_eq!(parsed, vec!["--folder", "my data", "--auth-token", "a b", "file name.json"]);
    }

    #[test]
    fn resolve_cli_prefers_command_line_values_over_config() {
        let resolved = resolve_test_cli(
            &["--bind", "127.0.0.1:9999", "--folder", "cli"],
            &["--bind", "127.0.0.1:4444", "--file", "config.json"],
        );

        assert_eq!(resolved.bind, "127.0.0.1:9999".parse().expect("socket addr"));
        assert_eq!(resolved.folder, Some(PathBuf::from("cli")));
        assert_eq!(resolved.file, None);
    }

    #[test]
    fn resolve_cli_loads_path_from_config() {
        let resolved = resolve_test_cli(&[], &["./config-data"]);
        assert_eq!(resolved.path, Some(PathBuf::from("./config-data")));
    }

    #[test]
    fn resolve_cli_loads_folder_from_config() {
        let resolved = resolve_test_cli(&[], &["--folder", "config-folder"]);
        assert_eq!(resolved.folder, Some(PathBuf::from("config-folder")));
    }

    #[test]
    fn resolve_cli_loads_file_from_config() {
        let resolved = resolve_test_cli(&[], &["--file", "config.json"]);
        assert_eq!(resolved.file, Some(PathBuf::from("config.json")));
    }

    #[test]
    fn resolve_cli_loads_bind_from_config() {
        let resolved = resolve_test_cli(&[], &["--bind", "0.0.0.0:4444"]);
        assert_eq!(resolved.bind, "0.0.0.0:4444".parse::<SocketAddr>().expect("socket addr"));
    }

    #[test]
    fn resolve_cli_loads_port_from_config() {
        let resolved = resolve_test_cli(&[], &["--port", "4555"]);
        assert_eq!(resolved.bind, "127.0.0.1:4555".parse::<SocketAddr>().expect("socket addr"));
    }

    #[test]
    fn resolve_cli_loads_readonly_from_config() {
        let resolved = resolve_test_cli(&[], &["--readonly"]);
        assert!(resolved.readonly);
    }

    #[test]
    fn resolve_cli_loads_schema_from_config() {
        let resolved = resolve_test_cli(&[], &["--schema", "schema.dbml"]);
        assert_eq!(resolved.schema, Some(PathBuf::from("schema.dbml")));
    }

    #[test]
    fn resolve_cli_loads_log_from_config() {
        let resolved = resolve_test_cli(&[], &["--log"]);
        assert!(resolved.log);
    }

    #[test]
    fn resolve_cli_loads_logname_from_config() {
        let resolved = resolve_test_cli(&[], &["--logname", "dirbase.log"]);
        assert_eq!(resolved.logname, PathBuf::from("dirbase.log"));
    }

    #[test]
    fn resolve_cli_loads_auth_token_from_config() {
        let resolved = resolve_test_cli(&[], &["--auth-token", "secret"]);
        assert_eq!(resolved.auth_token.as_deref(), Some("secret"));
    }

    #[test]
    fn resolve_cli_loads_cors_origin_from_config() {
        let resolved = resolve_test_cli(&[], &["--cors-origin", "http://localhost:3000"]);
        assert_eq!(resolved.cors_origin.as_deref(), Some("http://localhost:3000"));
    }

    #[test]
    fn resolve_cli_loads_max_body_bytes_from_config() {
        let resolved = resolve_test_cli(&[], &["--max-body-bytes", "2048"]);
        assert_eq!(resolved.max_body_bytes, 2048);
    }

    #[test]
    fn resolve_cli_loads_max_per_page_from_config() {
        let resolved = resolve_test_cli(&[], &["--max-per-page", "7"]);
        assert_eq!(resolved.max_per_page, 7);
    }

    #[test]
    fn resolve_cli_loads_max_sql_scan_rows_from_config() {
        let resolved = resolve_test_cli(&[], &["--max-sql-scan-rows", "12"]);
        assert_eq!(resolved.max_sql_scan_rows, 12);
    }

    #[test]
    fn resolve_cli_loads_max_sql_selected_rows_from_config() {
        let resolved = resolve_test_cli(&[], &["--max-sql-selected-rows", "3"]);
        assert_eq!(resolved.max_sql_selected_rows, 3);
    }

    #[test]
    fn resolve_cli_command_line_path_overrides_config_source() {
        let resolved = resolve_test_cli(&["./cli-data"], &["--folder", "config-folder"]);
        assert_eq!(resolved.path, Some(PathBuf::from("./cli-data")));
        assert_eq!(resolved.folder, None);
    }

    #[test]
    fn resolve_cli_command_line_folder_overrides_config_source() {
        let resolved = resolve_test_cli(&["--folder", "cli-folder"], &["--file", "config.json"]);
        assert_eq!(resolved.folder, Some(PathBuf::from("cli-folder")));
        assert_eq!(resolved.file, None);
    }

    #[test]
    fn resolve_cli_command_line_file_overrides_config_source() {
        let resolved = resolve_test_cli(&["--file", "cli.json"], &["./config-data"]);
        assert_eq!(resolved.file, Some(PathBuf::from("cli.json")));
        assert_eq!(resolved.path, None);
    }

    #[test]
    fn resolve_cli_command_line_bind_overrides_config_bind() {
        let resolved = resolve_test_cli(&["--bind", "127.0.0.1:9999"], &["--bind", "0.0.0.0:4444"]);
        assert_eq!(resolved.bind, "127.0.0.1:9999".parse::<SocketAddr>().expect("socket addr"));
    }

    #[test]
    fn resolve_cli_command_line_port_overrides_config_bind_port() {
        let resolved = resolve_test_cli(&["--port", "9999"], &["--bind", "0.0.0.0:4444"]);
        assert_eq!(resolved.bind, "0.0.0.0:9999".parse::<SocketAddr>().expect("socket addr"));
    }

    #[test]
    fn resolve_cli_command_line_bind_overrides_config_port() {
        let resolved = resolve_test_cli(&["--bind", "0.0.0.0:9999"], &["--port", "4444"]);
        assert_eq!(resolved.bind, "0.0.0.0:9999".parse::<SocketAddr>().expect("socket addr"));
    }

    #[test]
    fn resolve_cli_command_line_schema_overrides_config_schema() {
        let resolved = resolve_test_cli(&["--schema", "cli.dbml"], &["--schema", "config.dbml"]);
        assert_eq!(resolved.schema, Some(PathBuf::from("cli.dbml")));
    }

    #[test]
    fn resolve_cli_command_line_logname_overrides_config_logname() {
        let resolved = resolve_test_cli(&["--logname", "cli.log"], &["--logname", "config.log"]);
        assert_eq!(resolved.logname, PathBuf::from("cli.log"));
    }

    #[test]
    fn resolve_cli_command_line_auth_token_overrides_config_auth_token() {
        let resolved =
            resolve_test_cli(&["--auth-token", "cli-token"], &["--auth-token", "config-token"]);
        assert_eq!(resolved.auth_token.as_deref(), Some("cli-token"));
    }

    #[test]
    fn resolve_cli_command_line_cors_origin_overrides_config_cors_origin() {
        let resolved = resolve_test_cli(
            &["--cors-origin", "http://localhost:4000"],
            &["--cors-origin", "http://localhost:3000"],
        );
        assert_eq!(resolved.cors_origin.as_deref(), Some("http://localhost:4000"));
    }

    #[test]
    fn resolve_cli_command_line_max_body_bytes_overrides_config_max_body_bytes() {
        let resolved =
            resolve_test_cli(&["--max-body-bytes", "4096"], &["--max-body-bytes", "2048"]);
        assert_eq!(resolved.max_body_bytes, 4096);
    }

    #[test]
    fn resolve_cli_command_line_max_per_page_overrides_config_max_per_page() {
        let resolved = resolve_test_cli(&["--max-per-page", "11"], &["--max-per-page", "7"]);
        assert_eq!(resolved.max_per_page, 11);
    }

    #[test]
    fn resolve_cli_command_line_max_sql_scan_rows_overrides_config_max_sql_scan_rows() {
        let resolved =
            resolve_test_cli(&["--max-sql-scan-rows", "20"], &["--max-sql-scan-rows", "12"]);
        assert_eq!(resolved.max_sql_scan_rows, 20);
    }

    #[test]
    fn resolve_cli_command_line_max_sql_selected_rows_overrides_config_max_sql_selected_rows() {
        let resolved =
            resolve_test_cli(&["--max-sql-selected-rows", "9"], &["--max-sql-selected-rows", "3"]);
        assert_eq!(resolved.max_sql_selected_rows, 9);
    }
}
