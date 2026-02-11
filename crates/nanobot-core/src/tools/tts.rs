use anyhow::{anyhow, Result};
use serde_json::Value;
use tokio::process::Command;

fn runtime_env_vars() -> Result<(String, String)> {
    let runtime_dir = std::env::var("SHERPA_ONNX_RUNTIME_DIR")
        .ok()
        .filter(|v| std::path::Path::new(v).exists())
        .or_else(default_runtime_dir)
        .ok_or_else(|| anyhow!("SHERPA_ONNX_RUNTIME_DIR not set and default runtime not found"))?;
    let model_dir = std::env::var("SHERPA_ONNX_MODEL_DIR")
        .ok()
        .filter(|v| std::path::Path::new(v).exists())
        .or_else(default_model_dir)
        .ok_or_else(|| anyhow!("SHERPA_ONNX_MODEL_DIR not set and default model not found"))?;
    Ok((runtime_dir, model_dir))
}

fn default_runtime_dir() -> Option<String> {
    let home = dirs::home_dir()?;
    let path = home
        .join(".nanobot")
        .join("tools")
        .join("sherpa-onnx-tts")
        .join("runtime");
    path.exists().then(|| path.to_string_lossy().to_string())
}

fn default_model_dir() -> Option<String> {
    let home = dirs::home_dir()?;
    let path = home
        .join(".nanobot")
        .join("tools")
        .join("sherpa-onnx-tts")
        .join("model");
    path.exists().then(|| path.to_string_lossy().to_string())
}

fn resolve_runtime_bin(runtime_dir: &str) -> Result<String> {
    let mut path = std::path::PathBuf::from(runtime_dir);
    path.push("bin");
    path.push("sherpa-onnx-offline-tts");
    if cfg!(windows) {
        path.set_extension("exe");
    }
    if !path.exists() {
        return Err(anyhow!("sherpa-onnx-offline-tts binary not found in runtime dir"));
    }
    Ok(path.to_string_lossy().to_string())
}

fn resolve_model_file(model_dir: &str) -> Result<String> {
    if let Ok(model_file) = std::env::var("SHERPA_ONNX_MODEL_FILE") {
        return Ok(model_file);
    }

    let dir = std::path::Path::new(model_dir);
    let mut candidate: Option<String> = None;
    for entry in walkdir::WalkDir::new(dir).max_depth(6).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("onnx") {
            candidate = Some(path.to_string_lossy().to_string());
            break;
        }
    }

    candidate.ok_or_else(|| anyhow!("No .onnx model found in SHERPA_ONNX_MODEL_DIR"))
}

pub async fn execute_tts(args: &Value) -> Result<String> {
    let text = args["text"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing 'text' field"))?
        .to_string();

    let output_path = args["output_path"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
            format!("tts_output_{}.wav", stamp)
        });

    let output_path = crate::tools::validate_path(&output_path)?;
    let (runtime_dir, model_dir) = runtime_env_vars()?;
    let bin_path = resolve_runtime_bin(&runtime_dir)?;
    let model_file = resolve_model_file(&model_dir)?;

    let voice = args["voice"].as_str().map(|s| s.to_string());
    let model_override = args["model"]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| std::env::var("SHERPA_ONNX_MODEL_FILE").ok());

    let extra_args = args["extra_args"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut cmd = Command::new(bin_path);
    cmd.arg("--text").arg(text);
    cmd.arg("--output").arg(&output_path);
    let model_path = model_override.unwrap_or(model_file);
    cmd.arg("--model").arg(&model_path);

    let model_parent = std::path::Path::new(&model_path)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from(&model_dir));

    let tokens_path = if model_parent.join("tokens.txt").exists() {
        model_parent.join("tokens.txt")
    } else {
        std::path::Path::new(&model_dir).join("tokens.txt")
    };
    cmd.arg("--tokens").arg(tokens_path);

    let espeak_path = if model_parent.join("espeak-ng-data").exists() {
        model_parent.join("espeak-ng-data")
    } else {
        std::path::Path::new(&model_dir).join("espeak-ng-data")
    };
    if espeak_path.exists() {
        cmd.arg("--espeak-data").arg(espeak_path);
    }

    if let Some(voice) = voice {
        cmd.arg("--speaker").arg(voice);
    }

    if !extra_args.is_empty() {
        cmd.args(extra_args);
    }

    // Ensure runtime libs are available
    let lib_path = std::path::Path::new(&runtime_dir).join("lib");
    if lib_path.exists() {
        let lib_str = lib_path.to_string_lossy().to_string();
        #[cfg(target_os = "macos")]
        cmd.env("DYLD_LIBRARY_PATH", &lib_str);
        #[cfg(target_os = "linux")]
        cmd.env("LD_LIBRARY_PATH", &lib_str);
        let path_sep = if cfg!(windows) { ";" } else { ":" };
        cmd.env(
            "PATH",
            format!("{}{}{}", lib_str, path_sep, std::env::var("PATH").unwrap_or_default()),
        );
    }

    let output = cmd.output().await?;
    if !output.status.success() {
        return Err(anyhow!(
            "TTS failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(output_path.to_string_lossy().to_string())
}
