use serde::{Deserialize, Serialize};
use std::{
    env,
    ffi::OsString,
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State};

const SETTINGS_FILE: &str = "settings.json";
const CONFIG_FILE: &str = "config.yaml";
const DOWNLOAD_EVENT: &str = "download-event";
const PROGRESS_PREFIX: &str = "__BOOSTY_PROGRESS__";
const LAUNCHER_FILE: &str = "boosty_launcher.py";
const BOOSTY_LAUNCHER: &str = include_str!("../boosty_launcher.py");
const MANAGED_PYTHON: &str = "3.12";
const UV_VERSION: &str = "0.8.4";
const MIN_PYTHON_MINOR: u32 = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettings {
    username: String,
    auth_header: String,
    cookie: String,
    destination_directory: String,
    post_url: String,
    preferred_video_quality: String,
    content_types: Vec<String>,
    request_delay_seconds: f64,
    skip_all_failures: bool,
}

impl AppSettings {
    fn defaults(app: &AppHandle) -> Self {
        let destination = app
            .path()
            .download_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("Boosty Downloads");

        Self {
            username: String::new(),
            auth_header: String::new(),
            cookie: String::new(),
            destination_directory: destination.to_string_lossy().into_owned(),
            post_url: String::new(),
            preferred_video_quality: "medium".into(),
            content_types: vec![
                "post_content".into(),
                "boosty_videos".into(),
                "external_videos".into(),
                "files".into(),
                "audio".into(),
            ],
            request_delay_seconds: 2.5,
            skip_all_failures: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatus {
    installed: bool,
    version: Option<String>,
    running: bool,
}

#[derive(Debug, Clone, Serialize)]
struct DownloadEvent {
    kind: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ProgressPayload {
    label: String,
    percent: Option<f64>,
    #[serde(default)]
    detail: String,
    #[serde(default = "default_progress_active")]
    active: bool,
}

fn default_progress_active() -> bool {
    true
}

#[derive(Default)]
struct ProcessState {
    child: Option<Child>,
    cancelled: bool,
}

struct DownloaderState(Mutex<ProcessState>);

impl Default for DownloaderState {
    fn default() -> Self {
        Self(Mutex::new(ProcessState::default()))
    }
}

#[derive(Clone)]
struct CommandSpec {
    program: String,
    prefix_args: Vec<String>,
}

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    Ok(directory)
}

fn runtime_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app_data_dir(app)?.join("downloader");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    Ok(directory)
}

fn venv_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(runtime_dir(app)?.join("venv"))
}

fn managed_binary(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(if cfg!(windows) {
        venv_dir(app)?.join("Scripts").join("boosty-downloader.exe")
    } else {
        venv_dir(app)?.join("bin").join("boosty-downloader")
    })
}

fn managed_python(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(if cfg!(windows) {
        venv_dir(app)?.join("Scripts").join("python.exe")
    } else {
        venv_dir(app)?.join("bin").join("python")
    })
}

fn prepare_download_command(app: &AppHandle, spec: CommandSpec) -> Result<CommandSpec, String> {
    let managed_binary = managed_binary(app)?;
    if spec.program != managed_binary.to_string_lossy() {
        return Ok(spec);
    }

    let launcher_path = runtime_dir(app)?.join(LAUNCHER_FILE);
    write_private(&launcher_path, BOOSTY_LAUNCHER)?;
    Ok(CommandSpec {
        program: managed_python(app)?.to_string_lossy().into_owned(),
        prefix_args: vec![launcher_path.to_string_lossy().into_owned()],
    })
}

fn certifi_bundle(app: &AppHandle) -> Option<PathBuf> {
    let output = Command::new(managed_python(app).ok()?)
        .args(["-c", "import certifi; print(certifi.where())"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    path.is_file().then_some(path)
}

fn json_scalar(value: &str) -> Result<String, String> {
    serde_json::to_string(value).map_err(|error| error.to_string())
}

fn write_private(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), String> {
    fs::write(path, contents).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn save_config(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    let data_dir = app_data_dir(app)?;
    let serialized = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    write_private(&data_dir.join(SETTINGS_FILE), serialized)?;

    let target = json_scalar(&settings.destination_directory)?;
    let cookie = json_scalar(settings.cookie.trim())?;
    let auth_header = json_scalar(settings.auth_header.trim())?;
    let yaml = format!(
        "auth:\n  cookie: {cookie}\n  auth_header: {auth_header}\ndownloading_settings:\n  target_directory: {target}\n"
    );
    let config_path = runtime_dir(app)?.join(CONFIG_FILE);
    write_private(&config_path, yaml)
}

fn validate_settings(settings: &AppSettings) -> Result<(), String> {
    if settings.username.trim().is_empty() {
        return Err("Укажите имя автора из адреса boosty.to/имя".into());
    }
    if settings.auth_header.trim().is_empty() || settings.cookie.trim().is_empty() {
        return Err("Добавьте Authorization и Cookie из авторизованной сессии Boosty".into());
    }
    if settings.destination_directory.trim().is_empty() {
        return Err("Выберите папку для загрузок".into());
    }
    if settings.content_types.is_empty() {
        return Err("Выберите хотя бы один тип контента".into());
    }
    if settings.request_delay_seconds < 1.0 {
        return Err("Задержка между запросами должна быть не меньше 1 секунды".into());
    }
    Ok(())
}

fn command_works(spec: &CommandSpec) -> Option<String> {
    let output = Command::new(&spec.program)
        .args(&spec.prefix_args)
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Some(if version.is_empty() {
        "установлен".into()
    } else {
        version
    })
}

fn find_downloader(app: &AppHandle) -> Option<(CommandSpec, String)> {
    let managed_binary = managed_binary(app).ok()?;
    let candidates = vec![
        CommandSpec {
            program: managed_binary.to_string_lossy().into_owned(),
            prefix_args: vec![],
        },
        CommandSpec {
            program: "boosty-downloader".into(),
            prefix_args: vec![],
        },
    ];

    candidates
        .into_iter()
        .find_map(|candidate| command_works(&candidate).map(|version| (candidate, version)))
}

fn command_output(program: impl AsRef<Path>, args: &[&str]) -> Result<std::process::Output, String> {
    Command::new(program.as_ref())
        .args(args)
        .output()
        .map_err(|error| format!("Не удалось запустить {}: {error}", program.as_ref().display()))
}

fn command_succeeded(program: impl AsRef<Path>, args: &[&str], context: &str) -> Result<String, String> {
    let output = command_output(program, args)?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        Ok(if stdout.is_empty() { stderr } else { stdout })
    } else {
        Err(format!(
            "{context}\n{}",
            if stderr.is_empty() { stdout } else { stderr }
        ))
    }
}

fn parse_python_version(raw: &str) -> Option<(u32, u32)> {
    let mut parts = raw
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

fn python_version(program: &str) -> Option<(u32, u32)> {
    let output = Command::new(program)
        .args(["-c", "import sys; print(f'{sys.version_info.major}.{sys.version_info.minor}')"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_python_version(String::from_utf8_lossy(&output.stdout).trim())
}

fn is_compatible_python(program: &str) -> bool {
    python_version(program).is_some_and(|(major, minor)| major == 3 && minor >= MIN_PYTHON_MINOR)
}

fn find_compatible_python() -> Option<String> {
    [
        "python3.13",
        "python3.12",
        "python3.11",
        "python3.10",
        "python3",
        "python",
        "py",
    ]
    .into_iter()
    .find(|candidate| is_compatible_python(candidate))
    .map(str::to_string)
}

fn venv_python_compatible(venv: &Path) -> bool {
    let python = if cfg!(windows) {
        venv.join("Scripts").join("python.exe")
    } else {
        venv.join("bin").join("python")
    };
    python.is_file() && is_compatible_python(&python.to_string_lossy())
}

fn remove_path(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|error| error.to_string())
    } else {
        fs::remove_file(path).map_err(|error| error.to_string())
    }
}

fn uv_binary(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(runtime_dir(app)?.join(if cfg!(windows) { "uv.exe" } else { "uv" }))
}

fn uv_download_target() -> Result<(&'static str, &'static str), String> {
    let arch = env::consts::ARCH;
    match (env::consts::OS, arch) {
        ("macos", "aarch64") => Ok(("uv-aarch64-apple-darwin.tar.gz", "uv")),
        ("macos", "x86_64") => Ok(("uv-x86_64-apple-darwin.tar.gz", "uv")),
        ("linux", "aarch64") => Ok(("uv-aarch64-unknown-linux-gnu.tar.gz", "uv")),
        ("linux", "x86_64") => Ok(("uv-x86_64-unknown-linux-gnu.tar.gz", "uv")),
        ("windows", "x86_64") => Ok(("uv-x86_64-pc-windows-msvc.zip", "uv.exe")),
        ("windows", "aarch64") => Ok(("uv-aarch64-pc-windows-msvc.zip", "uv.exe")),
        _ => Err(format!(
            "Автоустановка Python не поддерживается для {}-{arch}. Установите Python 3.10+ вручную.",
            env::consts::OS
        )),
    }
}

fn download_file(url: &str, destination: &Path) -> Result<(), String> {
    if cfg!(windows) {
        command_succeeded(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                &format!(
                    "Invoke-WebRequest -UseBasicParsing -Uri {} -OutFile {}",
                    serde_json::to_string(url).map_err(|error| error.to_string())?,
                    serde_json::to_string(&destination.to_string_lossy())
                        .map_err(|error| error.to_string())?
                ),
            ],
            "Не удалось скачать вспомогательный установщик",
        )?;
    } else {
        command_succeeded(
            "curl",
            &["-fsSL", url, "-o", &destination.to_string_lossy()],
            "Не удалось скачать вспомогательный установщик",
        )?;
    }
    Ok(())
}

fn extract_uv_archive(archive: &Path, extract_dir: &Path, binary_name: &str) -> Result<PathBuf, String> {
    fs::create_dir_all(extract_dir).map_err(|error| error.to_string())?;
    if cfg!(windows) {
        command_succeeded(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                &format!(
                    "Expand-Archive -Force -Path {} -DestinationPath {}",
                    serde_json::to_string(&archive.to_string_lossy())
                        .map_err(|error| error.to_string())?,
                    serde_json::to_string(&extract_dir.to_string_lossy())
                        .map_err(|error| error.to_string())?
                ),
            ],
            "Не удалось распаковать вспомогательный установщик",
        )?;
    } else {
        command_succeeded(
            "tar",
            &[
                "-xzf",
                &archive.to_string_lossy(),
                "-C",
                &extract_dir.to_string_lossy(),
            ],
            "Не удалось распаковать вспомогательный установщик",
        )?;
    }

    let nested = extract_dir.join(binary_name);
    if nested.is_file() {
        return Ok(nested);
    }

    for entry in fs::read_dir(extract_dir).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let candidate = entry.path().join(binary_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err("В архиве установщика не найден исполняемый файл uv".into())
}

fn ensure_uv(app: &AppHandle) -> Result<PathBuf, String> {
    let binary = uv_binary(app)?;
    if binary.is_file()
        && command_output(&binary, &["--version"])
            .map(|output| output.status.success())
            .unwrap_or(false)
    {
        return Ok(binary);
    }

    emit(app, "log", "Скачиваю встроенный установщик Python…");
    let (archive_name, binary_name) = uv_download_target()?;
    let tools_dir = runtime_dir(app)?.join("tools");
    let extract_dir = tools_dir.join("uv-extract");
    remove_path(&extract_dir)?;
    fs::create_dir_all(&tools_dir).map_err(|error| error.to_string())?;

    let archive = tools_dir.join(archive_name);
    let url = format!(
        "https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/{archive_name}"
    );
    download_file(&url, &archive)?;
    let extracted = extract_uv_archive(&archive, &extract_dir, binary_name)?;
    remove_path(&binary)?;
    fs::copy(&extracted, &binary).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))
            .map_err(|error| error.to_string())?;
    }
    let _ = fs::remove_file(&archive);
    remove_path(&extract_dir)?;
    Ok(binary)
}

fn install_with_uv(app: &AppHandle, venv: &Path) -> Result<(), String> {
    let uv = ensure_uv(app)?;
    remove_path(venv)?;
    emit(
        app,
        "log",
        format!("Готовлю изолированный Python {MANAGED_PYTHON}…"),
    );
    command_succeeded(
        &uv,
        &[
            "venv",
            "--python",
            MANAGED_PYTHON,
            "--seed",
            &venv.to_string_lossy(),
        ],
        "Не удалось создать окружение Python",
    )?;

    let python = managed_python(app)?;
    emit(app, "log", "Устанавливаю boosty-downloader…");
    command_succeeded(
        &uv,
        &[
            "pip",
            "install",
            "--python",
            &python.to_string_lossy(),
            "--upgrade",
            "boosty-downloader",
            "certifi",
        ],
        "Не удалось установить boosty-downloader",
    )?;
    Ok(())
}

fn install_with_system_python(app: &AppHandle, python: &str, venv: &Path) -> Result<(), String> {
    if !venv_python_compatible(venv) {
        remove_path(venv)?;
    }
    if !venv.exists() {
        emit(app, "log", "Создаю изолированное окружение Python…");
        command_succeeded(
            python,
            &["-m", "venv", &venv.to_string_lossy()],
            "Не удалось создать окружение Python",
        )?;
    }

    let venv_python = managed_python(app)?;
    emit(app, "log", "Обновляю pip…");
    command_succeeded(
        &venv_python,
        &["-m", "pip", "install", "--upgrade", "pip", "setuptools", "wheel"],
        "Не удалось обновить pip",
    )?;
    emit(app, "log", "Устанавливаю boosty-downloader…");
    command_succeeded(
        &venv_python,
        &[
            "-m",
            "pip",
            "install",
            "--upgrade",
            "boosty-downloader",
            "certifi",
        ],
        "Не удалось установить boosty-downloader",
    )?;
    Ok(())
}

fn emit(app: &AppHandle, kind: &str, message: impl Into<String>) {
    let _ = app.emit(
        DOWNLOAD_EVENT,
        DownloadEvent {
            kind: kind.into(),
            message: message.into(),
            percent: None,
            label: None,
            detail: None,
            active: None,
        },
    );
}

fn emit_progress(app: &AppHandle, payload: ProgressPayload) {
    let message = if payload.active {
        match (payload.percent, payload.detail.as_str()) {
            (Some(percent), detail) if !detail.is_empty() => {
                format!("{} — {:.0}% · {}", payload.label, percent, detail)
            }
            (Some(percent), _) => format!("{} — {:.0}%", payload.label, percent),
            (_, detail) if !detail.is_empty() => format!("{} — {}", payload.label, detail),
            _ => payload.label.clone(),
        }
    } else {
        String::new()
    };
    let _ = app.emit(
        DOWNLOAD_EVENT,
        DownloadEvent {
            kind: "progress".into(),
            message,
            percent: payload.percent,
            label: Some(payload.label),
            detail: Some(payload.detail),
            active: Some(payload.active),
        },
    );
}

fn forward_output<R: Read + Send + 'static>(reader: R, app: AppHandle, kind: &'static str) {
    thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if kind == "log" {
                if let Some(raw) = trimmed.strip_prefix(PROGRESS_PREFIX) {
                    if let Ok(payload) = serde_json::from_str::<ProgressPayload>(raw) {
                        emit_progress(&app, payload);
                        continue;
                    }
                }
            }
            emit(&app, kind, line);
        }
    });
}

fn build_download_args(spec: &CommandSpec, settings: &AppSettings) -> Vec<String> {
    let mut args = spec.prefix_args.clone();
    args.extend([
        "download".into(),
        "--username".into(),
        settings.username.trim().into(),
        "--preferred-video-quality".into(),
        settings.preferred_video_quality.clone(),
        "--request-delay-seconds".into(),
        settings.request_delay_seconds.to_string(),
        "--destination-directory".into(),
        settings.destination_directory.clone(),
    ]);
    if !settings.post_url.trim().is_empty() {
        args.extend(["--post-url".into(), settings.post_url.trim().into()]);
    }
    for content_type in &settings.content_types {
        args.extend(["--content-type-filter".into(), content_type.clone()]);
    }
    if settings.skip_all_failures {
        args.push("--skip-all-failures".into());
    }
    args
}

fn build_process_command(
    spec: &CommandSpec,
    args: &[String],
    working_directory: &Path,
    ca_bundle: Option<&Path>,
) -> Command {
    let mut command = Command::new(&spec.program);
    command
        .args(args)
        .current_dir(working_directory)
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(executable_path) = executable_search_path() {
        command.env("PATH", executable_path);
    }
    if let Some(ca_bundle) = ca_bundle {
        command.env("SSL_CERT_FILE", ca_bundle);
    }
    command
}

fn executable_search_path() -> Option<OsString> {
    let mut directories = Vec::<PathBuf>::new();

    #[cfg(target_os = "macos")]
    directories.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/local/bin"),
    ]);

    if let Some(current_path) = env::var_os("PATH") {
        directories.extend(env::split_paths(&current_path));
    }
    directories.dedup();
    env::join_paths(directories).ok()
}

#[tauri::command]
fn load_settings(app: AppHandle) -> Result<AppSettings, String> {
    let path = app_data_dir(&app)?.join(SETTINGS_FILE);
    if !path.exists() {
        return Ok(AppSettings::defaults(&app));
    }
    let data = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&data).map_err(|error| error.to_string())
}

#[tauri::command]
fn save_settings(app: AppHandle, settings: AppSettings) -> Result<(), String> {
    save_config(&app, &settings)
}

#[tauri::command]
fn runtime_status(app: AppHandle, state: State<'_, DownloaderState>) -> RuntimeStatus {
    let installed = find_downloader(&app);
    let managed_program = managed_binary(&app).ok();
    let has_required_ca = installed.as_ref().is_none_or(|(spec, _)| {
        managed_program.as_ref().is_none_or(|path| {
            spec.program != path.to_string_lossy() || certifi_bundle(&app).is_some()
        })
    });
    RuntimeStatus {
        installed: installed.is_some() && has_required_ca,
        version: installed.map(|(_, version)| version),
        running: state
            .0
            .lock()
            .map(|value| value.child.is_some())
            .unwrap_or(false),
    }
}

#[tauri::command]
fn install_downloader(app: AppHandle) -> Result<String, String> {
    let venv = venv_dir(&app)?;
    emit(&app, "log", "Готовлю Boosty Downloader…");

    let system_python = find_compatible_python();
    let result = match system_python.as_deref() {
        Some(python) => match install_with_system_python(&app, python, &venv) {
            Ok(()) => Ok(()),
            Err(error) => {
                emit(
                    &app,
                    "log",
                    "Системный Python не подошёл, пробую встроенную установку…",
                );
                install_with_uv(&app, &venv).map_err(|fallback| {
                    format!("{error}\n\nРезервная установка тоже не удалась:\n{fallback}")
                })
            }
        },
        None => {
            emit(
                &app,
                "log",
                "Подходящий Python не найден — ставлю изолированный runtime…",
            );
            install_with_uv(&app, &venv)
        }
    };

    result?;

    if find_downloader(&app).is_none() || certifi_bundle(&app).is_none() {
        return Err(
            "Установка завершилась, но Boosty Downloader не найден. Нажмите «Установить» ещё раз."
                .into(),
        );
    }

    Ok("Boosty Downloader готов к работе".into())
}

#[tauri::command]
fn start_download(
    app: AppHandle,
    state: State<'_, DownloaderState>,
    settings: AppSettings,
) -> Result<(), String> {
    validate_settings(&settings)?;
    save_config(&app, &settings)?;
    fs::create_dir_all(Path::new(&settings.destination_directory))
        .map_err(|error| format!("Не удалось создать папку загрузок: {error}"))?;

    let (detected_spec, _) = find_downloader(&app).ok_or_else(|| {
        "Boosty Downloader не установлен. Нажмите «Установить» в верхней части окна.".to_string()
    })?;

    let mut guard = state
        .0
        .lock()
        .map_err(|_| "Не удалось получить состояние процесса")?;
    if guard.child.is_some() {
        return Err("Загрузка уже выполняется".into());
    }

    let managed_program = managed_binary(&app)?;
    let ca_bundle = if detected_spec.program == managed_program.to_string_lossy() {
        Some(certifi_bundle(&app).ok_or_else(|| {
            "Не найден набор доверенных сертификатов. Нажмите «Установить» для восстановления компонентов."
                .to_string()
        })?)
    } else {
        None
    };

    let spec = prepare_download_command(&app, detected_spec)?;
    let args = build_download_args(&spec, &settings);
    let mut child = build_process_command(&spec, &args, &runtime_dir(&app)?, ca_bundle.as_deref())
        .spawn()
        .map_err(|error| format!("Не удалось запустить Boosty Downloader: {error}"))?;

    if let Some(stdout) = child.stdout.take() {
        forward_output(stdout, app.clone(), "log");
    }
    if let Some(stderr) = child.stderr.take() {
        forward_output(stderr, app.clone(), "error");
    }

    guard.child = Some(child);
    guard.cancelled = false;
    drop(guard);
    emit(&app, "started", "Загрузка запущена");

    let monitor_app = app.clone();
    thread::spawn(move || loop {
        let finished = {
            let state = monitor_app.state::<DownloaderState>();
            let mut process = match state.0.lock() {
                Ok(process) => process,
                Err(_) => return,
            };
            let Some(child) = process.child.as_mut() else {
                return;
            };
            match child.try_wait() {
                Ok(Some(status)) => {
                    let cancelled = process.cancelled;
                    process.child = None;
                    process.cancelled = false;
                    Some((status.success(), status.code(), cancelled))
                }
                Ok(None) => None,
                Err(error) => {
                    process.child = None;
                    emit(&monitor_app, "error", format!("Ошибка процесса: {error}"));
                    return;
                }
            }
        };

        if let Some((success, code, cancelled)) = finished {
            if cancelled {
                emit(&monitor_app, "cancelled", "Загрузка остановлена");
            } else if success {
                emit(&monitor_app, "completed", "Загрузка завершена");
            } else {
                emit(
                    &monitor_app,
                    "failed",
                    format!("Загрузка завершилась с кодом {}", code.unwrap_or(-1)),
                );
            }
            return;
        }
        thread::sleep(Duration::from_millis(250));
    });

    Ok(())
}

#[tauri::command]
fn stop_download(state: State<'_, DownloaderState>) -> Result<(), String> {
    let mut process = state
        .0
        .lock()
        .map_err(|_| "Не удалось получить состояние процесса")?;
    let child = process
        .child
        .as_mut()
        .ok_or_else(|| "Активной загрузки нет".to_string())?;
    child
        .kill()
        .map_err(|error| format!("Не удалось остановить процесс: {error}"))?;
    process.cancelled = true;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(DownloaderState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            load_settings,
            save_settings,
            runtime_status,
            install_downloader,
            start_download,
            stop_download
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_settings() -> AppSettings {
        AppSettings {
            username: " creator ".into(),
            auth_header: "Bearer token".into(),
            cookie: "auth=value".into(),
            destination_directory: "/tmp/downloads".into(),
            post_url: "https://boosty.to/creator/posts/id".into(),
            preferred_video_quality: "high".into(),
            content_types: vec!["post_content".into(), "files".into()],
            request_delay_seconds: 3.0,
            skip_all_failures: true,
        }
    }

    #[test]
    fn builds_supported_cli_arguments() {
        let args = build_download_args(
            &CommandSpec {
                program: "boosty-downloader".into(),
                prefix_args: vec![],
            },
            &valid_settings(),
        );

        assert_eq!(args[0], "download");
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--username", "creator"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--preferred-video-quality", "high"]));
        assert_eq!(
            args.iter()
                .filter(|arg| *arg == "--content-type-filter")
                .count(),
            2
        );
        assert!(args.contains(&"--skip-all-failures".to_string()));
    }

    #[test]
    fn rejects_missing_credentials() {
        let mut settings = valid_settings();
        settings.cookie.clear();
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn json_strings_are_valid_yaml_scalars() {
        assert_eq!(json_scalar("a'b\nline").unwrap(), "\"a'b\\nline\"");
    }

    #[test]
    fn managed_process_uses_explicit_ca_bundle() {
        let spec = CommandSpec {
            program: "boosty-downloader".into(),
            prefix_args: vec![],
        };
        let ca_bundle = Path::new("/app/venv/certifi/cacert.pem");
        let command = build_process_command(&spec, &[], Path::new("/app"), Some(ca_bundle));
        let ssl_cert_file = command
            .get_envs()
            .find(|(key, _)| *key == "SSL_CERT_FILE")
            .and_then(|(_, value)| value);

        assert_eq!(ssl_cert_file, Some(ca_bundle.as_os_str()));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn managed_process_can_find_homebrew_media_tools() {
        let spec = CommandSpec {
            program: "boosty-downloader".into(),
            prefix_args: vec![],
        };
        let command = build_process_command(&spec, &[], Path::new("/app"), None);
        let executable_path = command
            .get_envs()
            .find(|(key, _)| *key == "PATH")
            .and_then(|(_, value)| value)
            .expect("the child process must receive an explicit PATH");

        assert!(std::env::split_paths(executable_path)
            .any(|path| path == Path::new("/opt/homebrew/bin")));
    }

    #[test]
    fn parses_python_version_strings() {
        assert_eq!(parse_python_version("3.12"), Some((3, 12)));
        assert_eq!(parse_python_version("Python 3.9.6"), Some((3, 9)));
        assert_eq!(parse_python_version("nope"), None);
    }

    #[test]
    fn embeds_vimeo_compatibility_launcher() {
        assert!(BOOSTY_LAUNCHER.contains("player.vimeo.com"));
        assert!(BOOSTY_LAUNCHER.contains("boosty_referer"));
        assert!(BOOSTY_LAUNCHER.contains(PROGRESS_PREFIX));
        assert!(BOOSTY_LAUNCHER.contains("DesktopProgressReporter"));
    }
}
