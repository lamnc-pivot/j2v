#!/usr/bin/env python3
"""
Persistent faster-whisper worker.

Usage: python3 transcribe_worker.py <model_dir>

Protocol: JSON lines over stdin/stdout
    request: {"type": "append_chunk", "audio_path": "/abs/path.wav"}
    request: {"type": "flush"}
  request: {"type": "shutdown"}

    response: {"success": true, "results": [{"text": "...", "language": "ja", "language_probability": 0.99}]}
  response: {"success": false, "error": "..."}
"""

import json
import os
import sys
import tempfile
import wave

import numpy as np

TARGET_SAMPLE_RATE = 16000
VAD_THRESHOLD = 0.45
MIN_SPEECH_DURATION_MS = 220
MIN_SILENCE_DURATION_MS = 420
SPEECH_PAD_MS = 160
MAX_SPEECH_DURATION_S = 9.0
FINALIZE_HOLDBACK_MS = 280
IDLE_TAIL_KEEP_MS = 800
HARD_MIN_RMS_DBFS = -75.0
LOW_SIGNAL_ACCEPT_DBFS = -60.0
LOW_SIGNAL_MIN_LANG_PROB = 0.92
AUTO_GAIN_TARGET_DBFS = -28.0
AUTO_GAIN_MAX_DB = 44.0


def emit(payload: dict) -> None:
    print(json.dumps(payload, ensure_ascii=False), flush=True)


def has_model_files(path: str) -> bool:
    return os.path.exists(os.path.join(path, "config.json")) and os.path.exists(
        os.path.join(path, "model.bin")
    )


def resolve_model_dir(model_dir: str) -> str | None:
    if has_model_files(model_dir):
        return model_dir

    try:
        for entry in os.scandir(model_dir):
            if entry.is_dir() and has_model_files(entry.path):
                return entry.path
    except FileNotFoundError:
        return None

    return None


def decode_once(model, audio_path: str, language: str | None, vad_filter: bool) -> dict:
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
        kwargs["vad_parameters"] = {
            "min_silence_duration_ms": 320,
            "speech_pad_ms": 220,
        }

    segments, info = model.transcribe(**kwargs)
    accepted: list[str] = []

    for seg in segments:
        text = (seg.text or "").strip()
        if not text:
            continue

        no_speech_prob = getattr(seg, "no_speech_prob", None)
        avg_logprob = getattr(seg, "avg_logprob", None)

        if no_speech_prob is not None and no_speech_prob > 0.6:
            continue
        if avg_logprob is not None and avg_logprob < -1.2:
            continue

        accepted.append(text)

    return {
        "text": "".join(accepted).strip(),
        "language": info.language,
        "language_probability": float(info.language_probability),
    }


def calculate_rms_dbfs(samples: np.ndarray) -> float:
    if samples.size == 0:
        return -100.0

    rms = float(np.sqrt(np.mean(np.square(samples, dtype=np.float32))))
    if rms <= np.finfo(np.float32).eps:
        return -100.0
    return float(20.0 * np.log10(rms))


def apply_auto_gain(samples: np.ndarray, target_dbfs: float, max_gain_db: float) -> tuple[np.ndarray, float, float]:
    if samples.size == 0:
        return samples, 0.0, -100.0

    before_dbfs = calculate_rms_dbfs(samples)
    if before_dbfs <= -99.0:
        return samples, 0.0, before_dbfs

    desired_gain_db = min(max(target_dbfs - before_dbfs, 0.0), max_gain_db)
    if desired_gain_db <= 0.01:
        return samples, 0.0, before_dbfs

    peak_norm = float(np.max(np.abs(samples))) if samples.size else 0.0
    max_peak_norm = 0.89
    peak_limited_gain_db = max(20.0 * np.log10(max_peak_norm / peak_norm), 0.0) if peak_norm > 0.0 else desired_gain_db
    gain_db = min(desired_gain_db, peak_limited_gain_db)
    if gain_db <= 0.01:
        return samples, 0.0, before_dbfs

    gain = float(10.0 ** (gain_db / 20.0))
    amplified = np.clip(samples * gain, -1.0, 1.0).astype(np.float32)
    after_dbfs = calculate_rms_dbfs(amplified)
    return amplified, gain_db, after_dbfs


def normalize_text_for_dedup(text: str) -> str:
    punctuation = {"。", "、", ".", ",", "!", "?", "！", "？"}
    return "".join(ch for ch in text.lower() if (not ch.isspace()) and ch not in punctuation)


def should_reject_low_signal_result(text: str, language: str, language_probability: float, rms_dbfs: float) -> bool:
    if rms_dbfs >= LOW_SIGNAL_ACCEPT_DBFS:
        return False
    if language != "ja":
        return True
    if language_probability < LOW_SIGNAL_MIN_LANG_PROB:
        return True
    return len(normalize_text_for_dedup(text)) <= 1


def read_wav_samples(audio_path: str) -> np.ndarray:
    with wave.open(audio_path, "rb") as wav_file:
        channels = wav_file.getnchannels()
        sample_rate = wav_file.getframerate()
        sample_width = wav_file.getsampwidth()
        frames = wav_file.readframes(wav_file.getnframes())

    if sample_width != 2:
        raise RuntimeError(f"Expected 16-bit PCM WAV, got sample width {sample_width}")
    if sample_rate != TARGET_SAMPLE_RATE:
        raise RuntimeError(f"Expected {TARGET_SAMPLE_RATE}Hz WAV, got {sample_rate}Hz")

    samples = np.frombuffer(frames, dtype=np.int16).astype(np.float32) / 32768.0
    if channels == 2:
        samples = samples.reshape(-1, 2).mean(axis=1)
    elif channels != 1:
        raise RuntimeError(f"Unsupported channel count: {channels}")
    return samples.astype(np.float32)


def write_temp_wav(samples: np.ndarray) -> str:
    pcm = np.clip(samples, -1.0, 1.0)
    pcm = (pcm * 32767.0).astype(np.int16)

    with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tmp_file:
        output_path = tmp_file.name

    with wave.open(output_path, "wb") as wav_file:
        wav_file.setnchannels(1)
        wav_file.setsampwidth(2)
        wav_file.setframerate(TARGET_SAMPLE_RATE)
        wav_file.writeframes(pcm.tobytes())

    return output_path


def transcribe_segment(model, samples: np.ndarray) -> dict | None:
    if samples.size == 0:
        return None

    rms_dbfs = calculate_rms_dbfs(samples)
    if rms_dbfs < HARD_MIN_RMS_DBFS:
        return None

    normalized, _, normalized_rms_dbfs = apply_auto_gain(samples, AUTO_GAIN_TARGET_DBFS, AUTO_GAIN_MAX_DB)
    wav_path = write_temp_wav(normalized)
    try:
        result = decode_once(model, wav_path, language="ja", vad_filter=False)
        if not result["text"]:
            result = decode_once(model, wav_path, language="ja", vad_filter=True)
    finally:
        if os.path.exists(wav_path):
            os.remove(wav_path)

    if not result["text"].strip():
        return None
    if should_reject_low_signal_result(
        result["text"],
        result["language"],
        result["language_probability"],
        normalized_rms_dbfs,
    ):
        return None
    return result


def process_pending(model, get_speech_timestamps, vad_options, pending_audio: np.ndarray, flush: bool) -> tuple[list[dict], np.ndarray]:
    if pending_audio.size == 0:
        return [], pending_audio

    speech_timestamps = get_speech_timestamps(
        pending_audio,
        vad_options=vad_options,
        sampling_rate=TARGET_SAMPLE_RATE,
    )

    if not speech_timestamps:
        if flush:
            return [], np.array([], dtype=np.float32)

        idle_keep = int(TARGET_SAMPLE_RATE * IDLE_TAIL_KEEP_MS / 1000)
        if pending_audio.size > idle_keep:
            pending_audio = pending_audio[-idle_keep:]
        return [], pending_audio

    commit_limit = pending_audio.size if flush else max(
        0,
        pending_audio.size - int(TARGET_SAMPLE_RATE * FINALIZE_HOLDBACK_MS / 1000),
    )
    finalized = [ts for ts in speech_timestamps if ts["end"] <= commit_limit]
    if flush and not finalized:
        finalized = speech_timestamps

    results: list[dict] = []
    processed_until = 0
    for timestamp in finalized:
        start = int(timestamp["start"])
        end = int(timestamp["end"])
        if end <= start:
            continue

        result = transcribe_segment(model, pending_audio[start:end])
        processed_until = max(processed_until, end)
        if result is not None:
            results.append(result)

    if processed_until > 0:
        pending_audio = pending_audio[processed_until:]
    elif not flush:
        keep_before_speech = int(TARGET_SAMPLE_RATE * SPEECH_PAD_MS / 1000)
        first_start = int(speech_timestamps[0]["start"])
        if first_start > keep_before_speech:
            pending_audio = pending_audio[first_start - keep_before_speech :]
    else:
        pending_audio = np.array([], dtype=np.float32)

    return results, pending_audio


def main() -> int:
    if len(sys.argv) < 2:
        emit({"success": False, "error": "Usage: transcribe_worker.py <model_dir>"})
        return 1

    model_dir = sys.argv[1]
    if not os.path.exists(model_dir):
        emit({"success": False, "error": f"Model directory not found: {model_dir}"})
        return 1

    resolved_model_dir = resolve_model_dir(model_dir)
    if resolved_model_dir is None:
        emit({
            "success": False,
            "error": f"No valid faster-whisper model files found under: {model_dir}",
        })
        return 1

    try:
        from faster_whisper import WhisperModel
        from faster_whisper.vad import VadOptions, get_speech_timestamps, get_vad_model

        model = WhisperModel(resolved_model_dir, device="cpu", compute_type="int8")
        get_vad_model()
        vad_options = VadOptions(
            threshold=VAD_THRESHOLD,
            min_speech_duration_ms=MIN_SPEECH_DURATION_MS,
            min_silence_duration_ms=MIN_SILENCE_DURATION_MS,
            speech_pad_ms=SPEECH_PAD_MS,
            max_speech_duration_s=MAX_SPEECH_DURATION_S,
        )
        pending_audio = np.array([], dtype=np.float32)
        emit({"success": True})
    except ImportError:
        emit({
            "success": False,
            "error": "faster-whisper with VAD dependencies is not installed. Run: pip install faster-whisper onnxruntime",
        })
        return 1
    except Exception as exc:
        emit({"success": False, "error": str(exc)})
        return 1

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue

        try:
            request = json.loads(line)
        except json.JSONDecodeError as exc:
            emit({"success": False, "error": f"Invalid JSON request: {exc}"})
            continue

        request_type = request.get("type")
        if request_type == "shutdown":
            emit({"success": True})
            return 0

        try:
            if request_type == "append_chunk":
                audio_path = request.get("audio_path")
                if not isinstance(audio_path, str) or not audio_path:
                    emit({"success": False, "error": "audio_path is required"})
                    continue

                if not os.path.exists(audio_path):
                    emit({"success": False, "error": f"Audio file not found: {audio_path}"})
                    continue

                chunk_audio = read_wav_samples(audio_path)
                pending_audio = np.concatenate((pending_audio, chunk_audio)).astype(np.float32)
                results, pending_audio = process_pending(
                    model,
                    get_speech_timestamps,
                    vad_options,
                    pending_audio,
                    flush=False,
                )
                emit({"success": True, "results": results})
                continue

            if request_type == "flush":
                results, pending_audio = process_pending(
                    model,
                    get_speech_timestamps,
                    vad_options,
                    pending_audio,
                    flush=True,
                )
                emit({"success": True, "results": results})
                continue

            emit({"success": False, "error": f"Unknown request type: {request_type}"})
        except Exception as exc:
            emit({"success": False, "error": str(exc)})

    return 0


if __name__ == "__main__":
    sys.exit(main())