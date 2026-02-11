---
name: sherpa-onnx-tts
description: "Offline text-to-speech using Sherpa-ONNX"
category: audio
status: active
---

# Sherpa-ONNX TTS

Offline text-to-speech using the Sherpa-ONNX runtime.

## Tools Provided

- `tts`: Generate a WAV file from text using the local Sherpa-ONNX runtime

## Setup

Set environment variables:

- `SHERPA_ONNX_RUNTIME_DIR`: path to runtime/ directory
- `SHERPA_ONNX_MODEL_DIR`: path to model/ directory
- optional `SHERPA_ONNX_MODEL_FILE`: explicit .onnx file

## Usage

Example tool call:

```
{ "tool": "tts", "text": "Hello", "output_path": "hello.wav" }
```
