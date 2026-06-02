use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
};

use crate::{
    app,
    cli::Cli,
    schema::{self, Schema},
};

pub(crate) struct StartupSummary {
    pub(crate) source_kind: &'static str,
    pub(crate) source_path: String,
    pub(crate) resource_count: usize,
    pub(crate) schema_status: &'static str,
    pub(crate) mode: &'static str,
    pub(crate) clone_source: Option<String>,
}
pub(crate) fn browser_url_for(addr: SocketAddr) -> String {
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

pub(crate) fn data_source_kind_label(data_source: &app::DataSource) -> &'static str {
    match data_source {
        app::DataSource::Folder(_) => "folder",
        app::DataSource::File(_) => "file",
    }
}

pub(crate) fn data_source_path_label(data_source: &app::DataSource) -> String {
    match data_source {
        app::DataSource::Folder(path) | app::DataSource::File(path) => path.display().to_string(),
    }
}

pub(crate) fn schema_status_label(
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

pub(crate) fn print_startup_summary(browser_url: &str, cli: &Cli, summary: &StartupSummary) {
    eprintln!("Open {browser_url}");
    eprintln!("Source: {} {}", summary.source_kind, summary.source_path);
    eprintln!("Resources: {}", summary.resource_count);
    eprintln!("Schema: {}", summary.schema_status);
    eprintln!("Mode: {}", summary.mode);
    if let Some(clone_source) = &summary.clone_source {
        eprintln!("Clone source: {clone_source}");
    }
    if cli.auth_token.is_some() {
        eprintln!("Auth: bearer token enabled");
    }
    if let Some(origin) = &cli.cors_origin {
        eprintln!("CORS: {origin}");
    }
    if cli.protect_ops && cli.auth_token.is_some() {
        eprintln!("Ops auth: protected");
    } else if cli.protect_ops {
        eprintln!("Ops auth: disabled; no auth token configured");
    }
}

pub(crate) async fn resolve_data_source(cli: &Cli) -> app::DataSource {
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
    use super::browser_url_for;

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
}
