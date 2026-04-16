use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

fn main() {
    if let Err(error) = build_overview_assets() {
        panic!("{error}");
    }
}

fn build_overview_assets() -> Result<(), String> {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|err| err.to_string())?);
    let ui_dir = manifest_dir.join("ui");
    let dist_dir = ui_dir.join("dist");
    let output_files = [dist_dir.join("overview.js"), dist_dir.join("overview.css")];
    let mut input_files = vec![
        ui_dir.join("package.json"),
        ui_dir.join("bun.lock"),
        ui_dir.join("esbuild.config.mjs"),
        ui_dir.join("tsconfig.json"),
    ];

    collect_files(&ui_dir.join("src"), &mut input_files)
        .map_err(|err| format!("Failed to inspect UI sources: {err}"))?;

    println!("cargo:rerun-if-changed={}", ui_dir.join("src").display());
    for path in &input_files {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let needs_build = outputs_missing(&output_files) || outputs_stale(&input_files, &output_files)?;
    if !needs_build {
        return Ok(());
    }

    fs::create_dir_all(&dist_dir)
        .map_err(|err| format!("Failed to create {}: {err}", dist_dir.display()))?;

    let output = Command::new("bun")
        .arg("run")
        .arg("build")
        .current_dir(&ui_dir)
        .output()
        .map_err(|err| match err.kind() {
            io::ErrorKind::NotFound => {
                "Bun is required to build the embedded overview UI assets. Install Bun and rerun `cargo build` or `cargo test`.".to_string()
            }
            _ => format!("Failed to start Bun for overview asset build: {err}"),
        })?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let mut message =
            "Failed to build embedded overview UI assets with `bun run build`.".to_string();
        if !stdout.is_empty() {
            message.push_str("\nstdout:\n");
            message.push_str(&stdout);
        }
        if !stderr.is_empty() {
            message.push_str("\nstderr:\n");
            message.push_str(&stderr);
        }
        return Err(message);
    }

    if outputs_missing(&output_files) {
        return Err(format!(
            "Overview asset build completed without producing {} and {}.",
            output_files[0].display(),
            output_files[1].display()
        ));
    }

    Ok(())
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_files(&path, files)?;
        } else {
            files.push(path);
        }
    }

    Ok(())
}

fn outputs_missing(outputs: &[PathBuf]) -> bool {
    outputs.iter().any(|path| !path.exists())
}

fn outputs_stale(inputs: &[PathBuf], outputs: &[PathBuf]) -> Result<bool, String> {
    let latest_input = latest_modified(inputs)?;
    let earliest_output = earliest_modified(outputs)?;
    Ok(earliest_output < latest_input)
}

fn latest_modified(paths: &[PathBuf]) -> Result<SystemTime, String> {
    let mut latest = None;
    for path in paths {
        let modified = modified_time(path)?;
        latest = Some(match latest {
            Some(current) if current > modified => current,
            _ => modified,
        });
    }
    latest.ok_or_else(|| "No UI input files found for overview asset build".to_string())
}

fn earliest_modified(paths: &[PathBuf]) -> Result<SystemTime, String> {
    let mut earliest = None;
    for path in paths {
        let modified = modified_time(path)?;
        earliest = Some(match earliest {
            Some(current) if current < modified => current,
            _ => modified,
        });
    }
    earliest.ok_or_else(|| "No UI output files found for overview asset build".to_string())
}

fn modified_time(path: &Path) -> Result<SystemTime, String> {
    fs::metadata(path)
        .map_err(|err| format!("Failed to read metadata for {}: {err}", path.display()))?
        .modified()
        .map_err(|err| format!("Failed to read modified time for {}: {err}", path.display()))
}
