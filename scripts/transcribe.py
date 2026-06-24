#!/usr/bin/env python3
"""
Transcribe audio using faster-whisper.

Usage: python3 transcribe.py <audio_wav_path> <model_dir>

Output: JSON to stdout
  {"success": true, "text": "...", "language": "ja", "language_probability": 0.99}
  {"success": false, "error": "..."}
"""

import sys
import json
import os


def _decode_once(model, audio_path: str, language: str | None, vad_filter: bool) -> dict:
    """Run one decode pass and return text + language metadata."""
    kwargs = {
        "audio": audio_path,
        "beam_size": 5,
        "temperature": 0.0,
        "condition_on_previous_text": False,
        "vad_filter": vad_filter,
    }

    if language:
        kwargs["language"] = language

    if vad_filter:
        # Keep this lenient for headset microphones.
        kwargs["vad_parameters"] = {
            "min_silence_duration_ms": 200,
            "speech_pad_ms": 400,
        }

    segments, info = model.transcribe(**kwargs)

    accepted = []
    for seg in segments:
        text = (seg.text or "").strip()
        if not text:
            continue

        no_speech_prob = getattr(seg, "no_speech_prob", None)
        avg_logprob = getattr(seg, "avg_logprob", None)

        # Filter low-confidence segments to reduce hallucinations on weak audio.
        if no_speech_prob is not None and no_speech_prob > 0.6:
            continue
        if avg_logprob is not None and avg_logprob < -1.2:
            continue

        accepted.append(text)

    text = "".join(accepted).strip()

    return {
        "text": text,
        "language": info.language,
        "language_probability": float(info.language_probability),
    }


def transcribe(audio_path: str, model_dir: str) -> dict:
    if not os.path.exists(audio_path):
        return {"success": False, "error": f"Audio file not found: {audio_path}"}

    if not os.path.exists(model_dir):
        return {"success": False, "error": f"Model directory not found: {model_dir}"}

    resolved_model_dir = resolve_model_dir(model_dir)
    if resolved_model_dir is None:
        return {
            "success": False,
            "error": f"No valid faster-whisper model files found under: {model_dir}",
        }

    try:
        from faster_whisper import WhisperModel

        model = WhisperModel(resolved_model_dir, device="cpu", compute_type="int8")

        # Pass 1: Japanese + lenient VAD
        result = _decode_once(model, audio_path, language="ja", vad_filter=True)

        # Pass 2: Japanese + no VAD (for low-volume / compressed Bluetooth voice)
        if not result["text"]:
            result = _decode_once(model, audio_path, language="ja", vad_filter=False)

        return {
            "success": True,
            "text": result["text"],
            "language": result["language"],
            "language_probability": result["language_probability"],
        }

    except ImportError:
        return {
            "success": False,
            "error": "faster-whisper is not installed. Run: pip install faster-whisper",
        }
    except Exception as e:
        return {"success": False, "error": str(e)}


def resolve_model_dir(model_dir: str) -> str | None:
    """Return a directory that contains real faster-whisper model files."""
    if has_model_files(model_dir):
        return model_dir

    try:
        for entry in os.scandir(model_dir):
            if entry.is_dir() and has_model_files(entry.path):
                return entry.path
    except FileNotFoundError:
        return None

    return None


def has_model_files(path: str) -> bool:
    return os.path.exists(os.path.join(path, "config.json")) and os.path.exists(
        os.path.join(path, "model.bin")
    )


if __name__ == "__main__":
    if len(sys.argv) < 3:
        result = {
            "success": False,
            "error": "Usage: transcribe.py <audio_wav_path> <model_dir>",
        }
        print(json.dumps(result))
        sys.exit(1)

    audio_path = sys.argv[1]
    model_dir = sys.argv[2]

    result = transcribe(audio_path, model_dir)
    print(json.dumps(result, ensure_ascii=False))
    sys.exit(0 if result["success"] else 1)
