use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const UV_VERSION: &str = "0.8.4";

fn main() {
    prepare_bundled_uv().unwrap_or_else(|error| {
        panic!("Failed to prepare bundled uv: {error}");
    });
    tauri_build::build();
}

fn prepare_bundled_uv() -> Result<(), String> {
    let target = env::var("TARGET").map_err(|error| error.to_string())?;
    let (archive_name, binary_name) = uv_artifact_for_target(&target)?;
    let resources = PathBuf::from(env::var("CARGO_MANIFEST_DIR").map_err(|error| error.to_string())?)
        .join("resources");
    fs::create_dir_all(&resources).map_err(|error| error.to_string())?;

    let destination = resources.join(if target.contains("windows") {
        "uv.exe"
    } else {
        "uv"
    });

    let marker = resources.join(format!(".uv-{UV_VERSION}-{target}"));
    if destination.is_file() && marker.is_file() {
        println!("cargo:rerun-if-changed={}", destination.display());
        println!("cargo:rerun-if-changed={}", marker.display());
        return Ok(());
    }

    let staging = resources.join("uv-staging");
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|error| error.to_string())?;

    let archive = staging.join(archive_name);
    let url = format!(
        "https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/{archive_name}"
    );
    download(&url, &archive)?;
    let extracted = extract_archive(&archive, &staging, binary_name)?;
    if destination.exists() {
        fs::remove_file(&destination).map_err(|error| error.to_string())?;
    }
    fs::copy(&extracted, &destination).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&destination, fs::Permissions::from_mode(0o755))
            .map_err(|error| error.to_string())?;
    }

    for entry in fs::read_dir(&resources).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(".uv-") && entry.path() != marker {
            let _ = fs::remove_file(entry.path());
        }
    }
    fs::write(&marker, UV_VERSION).map_err(|error| error.to_string())?;
    let _ = fs::remove_dir_all(&staging);

    println!("cargo:rerun-if-changed={}", destination.display());
    println!("cargo:rerun-if-changed={}", marker.display());
    Ok(())
}

fn uv_artifact_for_target(target: &str) -> Result<(&'static str, &'static str), String> {
    match target {
        "aarch64-apple-darwin" => Ok(("uv-aarch64-apple-darwin.tar.gz", "uv")),
        "x86_64-apple-darwin" => Ok(("uv-x86_64-apple-darwin.tar.gz", "uv")),
        "aarch64-unknown-linux-gnu" => Ok(("uv-aarch64-unknown-linux-gnu.tar.gz", "uv")),
        "x86_64-unknown-linux-gnu" => Ok(("uv-x86_64-unknown-linux-gnu.tar.gz", "uv")),
        "x86_64-pc-windows-msvc" => Ok(("uv-x86_64-pc-windows-msvc.zip", "uv.exe")),
        "aarch64-pc-windows-msvc" => Ok(("uv-aarch64-pc-windows-msvc.zip", "uv.exe")),
        other => Err(format!("unsupported target for bundled uv: {other}")),
    }
}

fn download(url: &str, destination: &Path) -> Result<(), String> {
    let status = Command::new("curl")
        .args(["-fsSL", url, "-o"])
        .arg(destination)
        .status()
        .map_err(|error| format!("curl failed: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("curl exited with {status} while downloading {url}"))
    }
}

fn extract_archive(archive: &Path, extract_dir: &Path, binary_name: &str) -> Result<PathBuf, String> {
    if archive.extension().and_then(|value| value.to_str()) == Some("zip") {
        #[cfg(target_os = "windows")]
        {
            let status = Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    &format!(
                        "Expand-Archive -Force -Path {} -DestinationPath {}",
                        archive.display(),
                        extract_dir.display()
                    ),
                ])
                .status()
                .map_err(|error| error.to_string())?;
            if !status.success() {
                return Err("failed to unzip uv archive".into());
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let status = Command::new("unzip")
                .args(["-o", "-d"])
                .arg(extract_dir)
                .arg(archive)
                .status()
                .map_err(|error| error.to_string())?;
            if !status.success() {
                return Err("failed to unzip uv archive".into());
            }
        }
    } else {
        let status = Command::new("tar")
            .args(["-xzf"])
            .arg(archive)
            .arg("-C")
            .arg(extract_dir)
            .status()
            .map_err(|error| error.to_string())?;
        if !status.success() {
            return Err("failed to extract uv archive".into());
        }
    }

    let direct = extract_dir.join(binary_name);
    if direct.is_file() {
        return Ok(direct);
    }
    for entry in fs::read_dir(extract_dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let candidate = entry.path().join(binary_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("uv binary missing from archive".into())
}
