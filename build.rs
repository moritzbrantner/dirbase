use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

const REBUILD_UI_ENV: &str = "DIRBASE_REBUILD_UI";

fn main() {
    if let Err(error) = ensure_overview_assets() {
        panic!("{error}");
    }
}

fn ensure_overview_assets() -> Result<(), String> {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|err| err.to_string())?);
    let ui_dir = manifest_dir.join("ui");
    let dist_dir = ui_dir.join("dist");
    let output_files = [dist_dir.join("overview.js"), dist_dir.join("overview.css")];

    println!("cargo:rerun-if-env-changed={REBUILD_UI_ENV}");
    for path in &output_files {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    if env::var_os(REBUILD_UI_ENV).is_none() {
        if outputs_missing(&output_files) {
            return Err(missing_assets_message(&ui_dir, &output_files));
        }
        return Ok(());
    }

    let mut input_files = vec![
        ui_dir.join("package.json"),
        ui_dir.join("bun.lock"),
        ui_dir.join("esbuild.config.mjs"),
        ui_dir.join("tsconfig.json"),
    ];

    collect_files(&ui_dir.join("src"), &mut input_files)
        .map_err(|err| format!("Failed to inspect UI sources: {err}"))?;

    for path in &input_files {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    fs::create_dir_all(&dist_dir)
        .map_err(|err| format!("Failed to create {}: {err}", dist_dir.display()))?;

    let output = Command::new("bun")
        .arg("run")
        .arg("build")
        .current_dir(&ui_dir)
        .output()
        .map_err(|err| match err.kind() {
            io::ErrorKind::NotFound => format!(
                "{REBUILD_UI_ENV}=1 was set, but Bun is not installed.\nInstall Bun and rerun `cargo build`, or unset {REBUILD_UI_ENV} to use the checked-in ui/dist assets."
            ),
            _ => format!("Failed to start Bun for overview asset build: {err}"),
        })?;

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let mut message =
            "Failed to rebuild embedded overview UI assets with `DIRBASE_REBUILD_UI=1 cargo build`."
                .to_string();
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
            "Overview asset rebuild completed without producing {} and {}.",
            output_files[0].display(),
            output_files[1].display()
        ));
    }

    Ok(())
}

fn missing_assets_message(ui_dir: &Path, outputs: &[PathBuf]) -> String {
    format!(
        "Embedded overview assets are missing.\nExpected files:\n- {}\n- {}\n\nThese checked-in files are required for default builds without Bun.\nMaintainers can regenerate them with:\n  cd {}\n  bun run build\nor:\n  {REBUILD_UI_ENV}=1 cargo build",
        outputs[0].display(),
        outputs[1].display(),
        ui_dir.display(),
    )
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
