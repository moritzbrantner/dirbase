use std::{ffi::OsString, net::SocketAddr, path::PathBuf};

use clap::{CommandFactory, Parser, parser::ValueSource};

use crate::app::ResponseFormat;

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
  dirbase --folder ./data --schema ./schema.xsd

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
        help = "Use an explicit schema file instead of auto-detecting schema.json, schema.xsd, or schema.dbml."
    )]
    schema: Option<PathBuf>,
    #[arg(long, help = "Enable request logging.")]
    log: bool,
    #[arg(long, help = "Return JSON response bodies as XML.")]
    xml: bool,
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
pub(crate) struct Cli {
    pub(crate) path: Option<PathBuf>,
    pub(crate) folder: Option<PathBuf>,
    pub(crate) file: Option<PathBuf>,
    pub(crate) bind: SocketAddr,
    pub(crate) readonly: bool,
    pub(crate) schema: Option<PathBuf>,
    pub(crate) log: bool,
    pub(crate) response_format: ResponseFormat,
    pub(crate) logname: PathBuf,
    pub(crate) auth_token: Option<String>,
    pub(crate) cors_origin: Option<String>,
    pub(crate) max_body_bytes: usize,
    pub(crate) max_per_page: usize,
    pub(crate) max_sql_scan_rows: usize,
    pub(crate) max_sql_selected_rows: usize,
}

pub(crate) enum CliLoadError {
    CommandLine(clap::Error),
    Config(String),
}
pub(crate) fn load_cli() -> Result<Option<Cli>, CliLoadError> {
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
        response_format: if resolve_flag("xml", cli_matches, config_matches) {
            ResponseFormat::Xml
        } else {
            ResponseFormat::Json
        },
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

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, path::PathBuf};

    use super::{CliArgs, parse_config_args, resolve_cli};
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
    fn resolve_cli_loads_xml_from_config() {
        let resolved = resolve_test_cli(&[], &["--xml"]);
        assert_eq!(resolved.response_format, super::ResponseFormat::Xml);
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
    fn resolve_cli_command_line_xml_overrides_config_default() {
        let resolved = resolve_test_cli(&["--xml"], &[]);
        assert_eq!(resolved.response_format, super::ResponseFormat::Xml);
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
