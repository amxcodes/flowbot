use anyhow::{anyhow, Result};
use serde_json::Value;
use tokio::process::Command;

#[derive(Clone)]
struct WhisperInvocation {
    program: String,
    prefix_args: Vec<String>,
    use_module: bool,
}

async fn resolve_whisper_invocation(args: &Value) -> Result<WhisperInvocation> {
    if let Some(bin) = args["whisper_bin"]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| std::env::var("NANOBOT_WHISPER_BIN").ok())
    {
        let mut check = Command::new(&bin);
        check.arg("--version");
        if check.output().await.is_ok() {
            return Ok(WhisperInvocation {
                program: bin,
                prefix_args: vec![],
                use_module: false,
            });
        }
    }

    let mut cli = Command::new("whisper");
    cli.arg("--version");
    if cli.output().await.is_ok() {
        return Ok(WhisperInvocation {
            program: "whisper".to_string(),
            prefix_args: vec![],
            use_module: false,
        });
    }

    for (program, prefix_args) in [
        ("python", vec![]),
        ("python3", vec![]),
        ("py", vec!["-3"]),
    ] {
        let mut cmd = Command::new(program);
        cmd.args(prefix_args.clone())
            .arg("-m")
            .arg("whisper")
            .arg("--help");

        if cmd
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Ok(WhisperInvocation {
                program: program.to_string(),
                prefix_args: prefix_args.into_iter().map(|s| s.to_string()).collect(),
                use_module: true,
            });
        }
    }

    Err(anyhow!(
        "Whisper is not installed. Run: nanobot setup --offline-models"
    ))
}

pub async fn execute_stt(args: &Value) -> Result<String> {
    let audio_path = args["audio_path"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing 'audio_path' field"))?;
    let audio_path = crate::tools::validate_path(audio_path)?;

    let model = args["model"]
        .as_str()
        .unwrap_or("base");
    let output_dir = args["output_dir"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "stt_output".to_string());
    let output_dir = crate::tools::validate_path(&output_dir)?;

    let format = args["format"]
        .as_str()
        .unwrap_or("txt");

    let whisper = resolve_whisper_invocation(args).await?;

    // Check ffmpeg availability
    let ffmpeg_status = Command::new("ffmpeg").arg("-version").output().await;
    if ffmpeg_status.is_err() {
        return Err(anyhow!("ffmpeg not found in PATH"));
    }

    let mut cmd = Command::new(&whisper.program);
    cmd.args(&whisper.prefix_args);
    if whisper.use_module {
        cmd.arg("-m").arg("whisper");
    }

    cmd.arg(audio_path.to_string_lossy().to_string())
        .arg("--model")
        .arg(model)
        .arg("--output_dir")
        .arg(output_dir.to_string_lossy().to_string())
        .arg("--output_format")
        .arg(format);

    let output = cmd.output().await?;
    if !output.status.success() {
        return Err(anyhow!(
            "Whisper failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    Ok(output_dir.to_string_lossy().to_string())
}
