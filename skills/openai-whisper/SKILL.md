---
name: openai-whisper
description: "Offline speech-to-text using Whisper CLI"
category: audio
status: active
---

# OpenAI Whisper (Offline STT)

Offline speech-to-text via the Whisper CLI.

## Tools Provided

- `stt`: Transcribe audio using Whisper CLI

## Requirements

- `whisper` CLI installed and in PATH
- `ffmpeg` installed and in PATH

## Usage

Example tool call:

```
{ "tool": "stt", "audio_path": "sample.wav", "model": "base", "output_dir": "stt_output", "format": "txt" }
```
