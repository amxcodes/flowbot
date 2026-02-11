use anyhow::{anyhow, bail, Context, Result};
use console::style;
use dialoguer::{theme::ColorfulTheme, MultiSelect};
use futures::StreamExt;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Clone, Copy)]
struct WhisperModel {
    id: &'static str,
    size: &'static str,
}

const WHISPER_MODELS: &[WhisperModel] = &[
    WhisperModel { id: "tiny", size: "~75 MB" },
    WhisperModel { id: "base", size: "~145 MB" },
    WhisperModel { id: "small", size: "~480 MB" },
    WhisperModel { id: "medium", size: "~1.5 GB" },
    WhisperModel { id: "large-v3", size: "~3.0 GB" },
    WhisperModel { id: "turbo", size: "~1.6 GB" },
];

const SHERPA_TTS_MODEL_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/vits-piper-en_US-lessac-high.tar.bz2";

pub async fn run_offline_models_installer() -> Result<()> {
    println!();
    println!("{}", style("Offline Speech Model Installer").bold().cyan());
    println!("Detected platform: {}", style(platform_label()).bold());

    let sherpa_supported = default_sherpa_runtime_url().is_some();
    if !sherpa_supported {
        println!(
            "{}",
            style("Sherpa TTS auto-install is not available on this architecture yet. STT will still work.")
                .yellow()
        );
    }

    let options = vec![
        "OpenAI Whisper STT models (tiny/base/small/medium/large-v3/turbo)",
        if sherpa_supported {
            "Sherpa-ONNX TTS runtime + en_US-lessac-high voice"
        } else {
            "Sherpa-ONNX TTS runtime + en_US-lessac-high voice (auto-install unavailable on this device)"
        },
    ];

    let selected = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose offline components to install now")
        .items(&options)
        .interact()?;

    if selected.is_empty() {
        println!("{}", style("No offline components selected.").yellow());
        return Ok(());
    }

    let mut failures: Vec<String> = Vec::new();

    if selected.contains(&0) {
        if let Err(err) = install_whisper_models_interactive().await {
            failures.push(format!("Whisper STT setup failed: {}", err));
        }
    }

    if selected.contains(&1) {
        if sherpa_supported {
            if let Err(err) = install_sherpa_tts_stack_interactive().await {
                failures.push(format!("Sherpa TTS setup failed: {}", err));
            }
        } else {
            println!();
            println!("{}", style("Skipping Sherpa TTS install on this architecture.").yellow());
        }
    }

    println!();
    if failures.is_empty() {
        println!("{}", style("✅ Offline model installation step complete.").green().bold());
    } else {
        println!("{}", style("⚠️  Offline setup completed with warnings:").yellow().bold());
        for issue in &failures {
            println!("  - {}", issue);
        }
    }
    println!("You can re-run this anytime with: {}", style("nanobot setup --offline-models").green());
    Ok(())
}

async fn install_whisper_models_interactive() -> Result<()> {
    println!();
    println!("{}", style("Whisper STT Models").bold().cyan());

    ensure_whisper_python_package().await?;
    ensure_ffmpeg_available().await;

    let labels: Vec<String> = WHISPER_MODELS
        .iter()
        .map(|m| format!("{} ({})", m.id, m.size))
        .collect();

    let selected = MultiSelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Select Whisper models to pre-download")
        .items(&labels)
        .interact()?;

    if selected.is_empty() {
        println!("{}", style("No Whisper models selected.").yellow());
        return Ok(());
    }

    let py = detect_python()
        .await?
        .ok_or_else(|| anyhow!("Python was not found in PATH (tried: python, python3, py -3)"))?;

    for idx in selected {
        let model = WHISPER_MODELS[idx];
        println!("Downloading Whisper model '{}'...", model.id);
        run_python_code(&py, &format!("import whisper; whisper.load_model({:?})", model.id)).await?;
        println!("  {} {}", style("✓ Cached model:").green(), model.id);
    }

    if !command_available("ffmpeg").await {
        println!();
        println!("{}", style("⚠️  ffmpeg not found in PATH.").yellow());
        println!("Whisper model files are cached, but STT transcription needs ffmpeg at runtime.");
        println!("Install ffmpeg manually, then use tool: {{ \"tool\": \"stt\", ... }}");
    }

    Ok(())
}

async fn ensure_whisper_python_package() -> Result<()> {
    if command_available("whisper").await {
        return Ok(());
    }

    println!("{}", style("whisper CLI not found in PATH.").yellow());
    println!("Installing openai-whisper automatically...");

    let py = match detect_python().await? {
        Some(py) => py,
        None => {
            println!("{}", style("Python not found in PATH. Attempting automatic install...").yellow());
            if try_install_python().await {
                detect_python().await?.ok_or_else(|| anyhow!(python_manual_help()))?
            } else {
                return Err(anyhow!(python_manual_help()));
            }
        }
    };

    let mut cmd = Command::new(&py.program);
    cmd.args(&py.prefix_args)
        .arg("-m")
        .arg("pip")
        .arg("install")
        .arg("-U")
        .arg("openai-whisper");

    let status = cmd.status().await.context("Failed to run pip install")?;
    if !status.success() {
        let mut ensure_pip = Command::new(&py.program);
        ensure_pip
            .args(&py.prefix_args)
            .arg("-m")
            .arg("ensurepip")
            .arg("--upgrade");
        let _ = ensure_pip.status().await;

        let mut retry = Command::new(&py.program);
        retry
            .args(&py.prefix_args)
            .arg("-m")
            .arg("pip")
            .arg("install")
            .arg("-U")
            .arg("openai-whisper");
        let retry_status = retry.status().await.context("Failed to retry pip install")?;
        if !retry_status.success() {
            bail!("Failed to install openai-whisper via pip");
        }
    }

    println!("{}", style("✓ Installed openai-whisper").green());
    Ok(())
}

async fn install_sherpa_tts_stack_interactive() -> Result<()> {
    println!();
    println!("{}", style("Sherpa-ONNX TTS Stack").bold().cyan());

    let runtime_url = std::env::var("NANOBOT_SHERPA_RUNTIME_URL")
        .ok()
        .or_else(|| default_sherpa_runtime_url().map(|s| s.to_string()))
        .ok_or_else(|| {
            anyhow!(
                "No default Sherpa runtime URL for this platform. Set NANOBOT_SHERPA_RUNTIME_URL and retry."
            )
        })?;

    let model_url = std::env::var("NANOBOT_SHERPA_MODEL_URL")
        .unwrap_or_else(|_| SHERPA_TTS_MODEL_URL.to_string());

    if !command_available("tar").await {
        bail!("tar command is required to extract .tar.bz2 archives");
    }

    let home = dirs::home_dir().ok_or_else(|| anyhow!("Could not resolve home directory"))?;
    let install_root = home.join(".nanobot").join("tools").join("sherpa-onnx-tts");
    let runtime_dir = install_root.join("runtime");
    let model_dir = install_root.join("model");

    tokio::fs::create_dir_all(&install_root).await?;

    let temp_root = std::env::temp_dir().join(format!("nanobot-offline-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&temp_root).await?;

    let runtime_archive = temp_root.join("runtime.tar.bz2");
    let model_archive = temp_root.join("model.tar.bz2");
    let runtime_extract = temp_root.join("runtime_extract");
    let model_extract = temp_root.join("model_extract");

    tokio::fs::create_dir_all(&runtime_extract).await?;
    tokio::fs::create_dir_all(&model_extract).await?;

    println!("Downloading Sherpa runtime...");
    download_file(&runtime_url, &runtime_archive).await?;
    println!("Downloading Sherpa model...");
    download_file(&model_url, &model_archive).await?;

    println!("Extracting runtime archive...");
    extract_tar_bz2(&runtime_archive, &runtime_extract).await?;
    println!("Extracting model archive...");
    extract_tar_bz2(&model_archive, &model_extract).await?;

    let runtime_root = find_runtime_root(&runtime_extract)
        .ok_or_else(|| anyhow!("Could not find sherpa runtime root (bin/lib) in extracted files"))?;
    let model_root = find_model_root(&model_extract)
        .ok_or_else(|| anyhow!("Could not find model root (.onnx + tokens.txt) in extracted files"))?;

    if runtime_dir.exists() {
        tokio::fs::remove_dir_all(&runtime_dir).await?;
    }
    if model_dir.exists() {
        tokio::fs::remove_dir_all(&model_dir).await?;
    }

    copy_dir_all(&runtime_root, &runtime_dir)?;
    copy_dir_all(&model_root, &model_dir)?;

    let _ = tokio::fs::remove_dir_all(&temp_root).await;

    println!("{} {}", style("✓ Runtime installed:").green(), runtime_dir.display());
    println!("{} {}", style("✓ Model installed:").green(), model_dir.display());
    println!("TTS tool auto-detects these paths if env vars are not set.");
    Ok(())
}

async fn command_available(command: &str) -> bool {
    let output = Command::new(command).arg("--version").output().await;
    output.map(|o| o.status.success()).unwrap_or(false)
}

async fn try_install_python() -> bool {
    #[cfg(target_os = "windows")]
    {
        if command_available("winget").await {
            let status = Command::new("winget")
                .arg("install")
                .arg("--id")
                .arg("Python.Python.3.12")
                .arg("-e")
                .arg("--accept-source-agreements")
                .arg("--accept-package-agreements")
                .status()
                .await;
            return status.map(|s| s.success()).unwrap_or(false);
        }
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        if command_available("brew").await {
            let status = Command::new("brew").arg("install").arg("python").status().await;
            return status.map(|s| s.success()).unwrap_or(false);
        }
        return false;
    }

    #[cfg(target_os = "linux")]
    {
        if command_available("apt-get").await {
            let status = Command::new("sh")
                .arg("-c")
                .arg("sudo apt-get update && sudo apt-get install -y python3 python3-pip")
                .status()
                .await;
            return status.map(|s| s.success()).unwrap_or(false);
        }
        if command_available("dnf").await {
            let status = Command::new("sh")
                .arg("-c")
                .arg("sudo dnf install -y python3 python3-pip")
                .status()
                .await;
            return status.map(|s| s.success()).unwrap_or(false);
        }
        if command_available("yum").await {
            let status = Command::new("sh")
                .arg("-c")
                .arg("sudo yum install -y python3 python3-pip")
                .status()
                .await;
            return status.map(|s| s.success()).unwrap_or(false);
        }
        false
    }
}

fn python_manual_help() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "Python is required for Whisper STT. Install Python 3 from https://www.python.org/downloads/ or run: winget install --id Python.Python.3.12 -e"
    }
    #[cfg(target_os = "macos")]
    {
        "Python is required for Whisper STT. Install with Homebrew: brew install python"
    }
    #[cfg(target_os = "linux")]
    {
        "Python is required for Whisper STT. Install python3 + pip with your package manager (apt/dnf/yum)."
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    "Python is required for Whisper STT."
}

fn platform_label() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

async fn ensure_ffmpeg_available() {
    if command_available("ffmpeg").await {
        return;
    }

    println!("{}", style("ffmpeg not found in PATH. Attempting automatic install...").yellow());
    if try_install_ffmpeg().await {
        if command_available("ffmpeg").await {
            println!("{}", style("✓ Installed ffmpeg").green());
            return;
        }
    }

    println!("{}", style("⚠️  Could not auto-install ffmpeg.").yellow());
    println!("Install it manually, then STT will work immediately.");
}

async fn try_install_ffmpeg() -> bool {
    #[cfg(target_os = "windows")]
    {
        if command_available("winget").await {
            let status = Command::new("winget")
                .arg("install")
                .arg("--id")
                .arg("Gyan.FFmpeg")
                .arg("-e")
                .arg("--accept-source-agreements")
                .arg("--accept-package-agreements")
                .status()
                .await;
            return status.map(|s| s.success()).unwrap_or(false);
        }
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        if command_available("brew").await {
            let status = Command::new("brew").arg("install").arg("ffmpeg").status().await;
            return status.map(|s| s.success()).unwrap_or(false);
        }
        return false;
    }

    #[cfg(target_os = "linux")]
    {
        if command_available("apt-get").await {
            let status = Command::new("sh")
                .arg("-c")
                .arg("sudo apt-get update && sudo apt-get install -y ffmpeg")
                .status()
                .await;
            return status.map(|s| s.success()).unwrap_or(false);
        }
        if command_available("dnf").await {
            let status = Command::new("sh")
                .arg("-c")
                .arg("sudo dnf install -y ffmpeg")
                .status()
                .await;
            return status.map(|s| s.success()).unwrap_or(false);
        }
        if command_available("yum").await {
            let status = Command::new("sh")
                .arg("-c")
                .arg("sudo yum install -y ffmpeg")
                .status()
                .await;
            return status.map(|s| s.success()).unwrap_or(false);
        }
        false
    }
}

#[derive(Clone)]
struct PythonCommand {
    program: String,
    prefix_args: Vec<String>,
}

async fn detect_python() -> Result<Option<PythonCommand>> {
    let candidates = vec![
        PythonCommand {
            program: "python".to_string(),
            prefix_args: vec![],
        },
        PythonCommand {
            program: "python3".to_string(),
            prefix_args: vec![],
        },
        PythonCommand {
            program: "py".to_string(),
            prefix_args: vec!["-3".to_string()],
        },
    ];

    for candidate in candidates {
        let mut cmd = Command::new(&candidate.program);
        cmd.args(&candidate.prefix_args).arg("--version");
        if cmd.output().await.is_ok() {
            return Ok(Some(candidate));
        }
    }

    Ok(None)
}

async fn run_python_code(py: &PythonCommand, code: &str) -> Result<()> {
    let mut cmd = Command::new(&py.program);
    cmd.args(&py.prefix_args).arg("-c").arg(code);
    let output = cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Python command failed: {}", stderr.trim());
    }
    Ok(())
}

async fn download_file(url: &str, destination: &Path) -> Result<()> {
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Failed to start download: {}", url))?;

    if !response.status().is_success() {
        bail!("Download failed ({}): {}", response.status(), url);
    }

    let mut file = tokio::fs::File::create(destination).await?;
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
    }

    file.flush().await?;
    Ok(())
}

async fn extract_tar_bz2(archive_path: &Path, destination: &Path) -> Result<()> {
    let status = Command::new("tar")
        .arg("-xjf")
        .arg(archive_path)
        .arg("-C")
        .arg(destination)
        .status()
        .await
        .with_context(|| format!("Failed to run tar on {}", archive_path.display()))?;

    if !status.success() {
        bail!("tar extraction failed for {}", archive_path.display());
    }

    Ok(())
}

fn find_runtime_root(base: &Path) -> Option<PathBuf> {
    let binary_name = if cfg!(windows) {
        "sherpa-onnx-offline-tts.exe"
    } else {
        "sherpa-onnx-offline-tts"
    };

    for entry in walkdir::WalkDir::new(base).max_depth(6).into_iter().flatten() {
        if entry.file_type().is_file() && entry.file_name().to_string_lossy() == binary_name {
            let bin_dir = entry.path().parent()?;
            return bin_dir.parent().map(|p| p.to_path_buf());
        }
    }

    None
}

fn find_model_root(base: &Path) -> Option<PathBuf> {
    for entry in walkdir::WalkDir::new(base).max_depth(6).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let is_onnx = path.extension().and_then(|s| s.to_str()) == Some("onnx");
        if !is_onnx {
            continue;
        }

        let parent = path.parent()?;
        if parent.join("tokens.txt").exists() {
            return Some(parent.to_path_buf());
        }
    }
    None
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).with_context(|| {
                format!("Failed to copy {} to {}", from.display(), to.display())
            })?;
        }
    }
    Ok(())
}

fn default_sherpa_runtime_url() -> Option<&'static str> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Some(
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.12.23/sherpa-onnx-v1.12.23-win-x64-shared.tar.bz2",
        )
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Some(
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.12.23/sherpa-onnx-v1.12.23-linux-x64-shared.tar.bz2",
        )
    }

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        Some(
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.12.23/sherpa-onnx-v1.12.23-linux-arm64-shared.tar.bz2",
        )
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Some(
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.12.23/sherpa-onnx-v1.12.23-osx-x64-shared.tar.bz2",
        )
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Some(
            "https://github.com/k2-fsa/sherpa-onnx/releases/download/v1.12.23/sherpa-onnx-v1.12.23-osx-arm64-shared.tar.bz2",
        )
    }

    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
    )))]
    {
        None
    }
}
