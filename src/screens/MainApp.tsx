import React, { useCallback, useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import TranscriptionDisplay from "../components/TranscriptionDisplay";

type CaptureBackendMode =
  | "auto"
  | "loopback-driver"
  | "screen-capture-kit"
  | "wasapi-loopback";

const getDefaultBackend = (): CaptureBackendMode => {
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes("mac")) {
    return "screen-capture-kit";
  }
  if (ua.includes("windows")) {
    return "wasapi-loopback";
  }
  return "auto";
};

const EVENT_POLL_INTERVAL_MS = 100;
const DURATION_TICK_INTERVAL_MS = 1000;
const MAX_PENDING_TRANSLATIONS = 200;

/** Streaming event types from backend */
interface StreamingEvent {
  type:
    | "Started"
    | "ChunkReceived"
    | "Stopped"
    | "Error"
    | "TranscriptionReady"
    | "TranscriptionError"
    | "TranslationReady"
    | "TranslationError";
  sample_rate?: number;
  channels?: number;
  chunk?: {
    samples: number[];
    sample_count: number;
    timestamp_ms: number;
  };
  duration_secs?: number;
  sample_count?: number;
  file_path?: string | null;
  message?: string;
  sequence_id?: number;
  /** TranscriptionReady fields */
  text?: string;
  language?: string;
  language_probability?: number;
  /** TranslationReady fields */
  source_text?: string;
  translated_text?: string;
  model?: string;
}

/**
 * Main application screen with streaming audio capture controls and real-time transcription display.
 * 
 * Features:
 * - Start/stop streaming system-audio recording (output loopback)
 * - Real-time transcription display (Japanese/Vietnamese)
 * - Display recording duration and status
 * - Text-to-speech playback (placeholder)
 */
const MainApp: React.FC = () => {
  const [isRecording, setIsRecording] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [japaneseText, setJapaneseText] = useState("");
  const [vietnameseText, setVietnameseText] = useState("");
  const [recordingDuration, setRecordingDuration] = useState(0);
  const [isTranscribing, setIsTranscribing] = useState(false);
  const [whisperAvailable, setWhisperAvailable] = useState<boolean | null>(null);
  const [captureBackend, setCaptureBackend] =
    useState<CaptureBackendMode>(getDefaultBackend());
  const pollingIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const durationIntervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const startTimeRef = useRef<number | null>(null);
  const chunkCountRef = useRef(0);
  const lastTranscriptNormRef = useRef("");
  const pendingTranslationsBySeqRef = useRef<Map<number, string>>(new Map());
  const skippedTranslationSeqsRef = useRef<Set<number>>(new Set());
  const nextExpectedTranslationSeqRef = useRef(1);

  const normalizeTranscript = useCallback((text: string) => {
    return text
      .toLowerCase()
      .replace(/[\s\u3000]/g, "")
      .replace(/[。、,.!?！？]/g, "");
  }, []);

  const clearTimers = useCallback(() => {
    if (pollingIntervalRef.current) {
      clearInterval(pollingIntervalRef.current);
      pollingIntervalRef.current = null;
    }
    if (durationIntervalRef.current) {
      clearInterval(durationIntervalRef.current);
      durationIntervalRef.current = null;
    }
  }, []);

  const resetTranslationBuffers = useCallback(() => {
    pendingTranslationsBySeqRef.current.clear();
    skippedTranslationSeqsRef.current.clear();
    nextExpectedTranslationSeqRef.current = 1;
  }, []);

  const flushTranslationBuffer = useCallback(() => {
    const parts: string[] = [];
    const pending = pendingTranslationsBySeqRef.current;
    const skipped = skippedTranslationSeqsRef.current;

    while (
      pending.has(nextExpectedTranslationSeqRef.current) ||
      skipped.has(nextExpectedTranslationSeqRef.current)
    ) {
      const seq = nextExpectedTranslationSeqRef.current;

      if (skipped.has(seq)) {
        skipped.delete(seq);
        nextExpectedTranslationSeqRef.current += 1;
        continue;
      }

      const text = pending.get(seq);
      pending.delete(seq);
      nextExpectedTranslationSeqRef.current += 1;

      if (text && text.trim()) {
        parts.push(text.trim());
      }
    }

    if (parts.length > 0) {
      const chunk = parts.join("\n");
      setVietnameseText((prev) => (prev ? `${prev}\n${chunk}` : chunk));
    }
  }, []);

  const enforcePendingTranslationLimit = useCallback(() => {
    const pending = pendingTranslationsBySeqRef.current;
    const skipped = skippedTranslationSeqsRef.current;

    if (pending.size <= MAX_PENDING_TRANSLATIONS) {
      return;
    }

    // Drop far-future entries first and mark them skipped to avoid blocking flush order.
    const keysDesc = Array.from(pending.keys()).sort((a, b) => b - a);
    while (pending.size > MAX_PENDING_TRANSLATIONS && keysDesc.length > 0) {
      const dropSeq = keysDesc.shift();
      if (typeof dropSeq !== "number") {
        break;
      }
      pending.delete(dropSeq);
      skipped.add(dropSeq);
    }
  }, []);

  /** Check whisper model availability on mount */
  useEffect(() => {
    invoke<{ id: string; installed: boolean }[]>("check_model_status")
      .then((models) => {
        const whisper = models.find((m) => m.id === "faster-whisper");
        setWhisperAvailable(whisper?.installed ?? false);
      })
      .catch(() => setWhisperAvailable(false));
  }, []);

  /** Poll for streaming events from backend */
  const pollStreamingEvents = useCallback(async () => {
    try {
      const events = await invoke<StreamingEvent[]>("get_streaming_events", {
        maxEvents: 10,
      });

      for (const event of events) {
        switch (event.type) {
          case "Started":
            console.log(
              `📍 Recording started: ${event.sample_rate}Hz, ${event.channels}ch`
            );
            break;

          case "ChunkReceived":
            if (event.chunk) {
              chunkCountRef.current++;
              console.log(
                `📦 Chunk ${chunkCountRef.current}: ${event.chunk.sample_count} samples @ ${event.chunk.timestamp_ms}ms`
              );
            }
            break;

          case "Stopped":
            console.log(
              `⏹️ Recording stopped: ${event.duration_secs}s, ${event.sample_count} samples`
            );
            console.log(`📁 Saved to: ${event.file_path}`);
            setRecordingDuration(0);
            setIsTranscribing(false);
            // Post-recording transcription fallback (if no streaming transcription ran)
            if (event.file_path && !whisperAvailable) {
              console.log("Whisper model not available – skipping post-transcription");
            }
            break;

          case "Error":
            console.error(`❌ Recording error: ${event.message}`);
            setError(event.message || "Recording error");
            break;

          case "TranscriptionReady":
            setIsTranscribing(false);
            if (event.text && event.text.trim()) {
              const normalized = normalizeTranscript(event.text);
              if (normalized && normalized === lastTranscriptNormRef.current) {
                console.log("ℹ️ Duplicate transcript skipped on UI");
                break;
              }

              lastTranscriptNormRef.current = normalized;
              console.log(
                `📝 Transcription #${event.sequence_id ?? "?"}: ${event.text}`
              );
              // Append new text segment (each result is a ~3s window)
              setJapaneseText((prev) =>
                prev ? `${prev}\n${event.text}` : event.text!
              );
            } else {
              console.log("ℹ️ Whisper window processed but no speech text detected");
            }
            break;

          case "TranscriptionError":
            console.warn(`⚠️ Transcription error: ${event.message}`);
            setIsTranscribing(false);
            setError(event.message || "Transcription error");
            break;

          case "TranslationReady":
            if (!event.translated_text?.trim()) {
              break;
            }

            const translated = event.translated_text.trim();
            const sequenceId = event.sequence_id;
            if (typeof sequenceId === "number") {
              console.log(
                `🌐 Translation #${sequenceId} (${event.model || "qwen"}): ${translated}`
              );
              pendingTranslationsBySeqRef.current.set(sequenceId, translated);
              enforcePendingTranslationLimit();
              flushTranslationBuffer();
            } else {
              console.log(`🌐 Translation (${event.model || "qwen"}): ${translated}`);
              setVietnameseText((prev) =>
                prev ? `${prev}\n${translated}` : translated
              );
            }
            break;

          case "TranslationError":
            console.warn(
              `⚠️ Translation error #${event.sequence_id ?? "?"}: ${event.message}`
            );
            if (typeof event.sequence_id === "number") {
              skippedTranslationSeqsRef.current.add(event.sequence_id);
              flushTranslationBuffer();
            }
            setError(event.message || "Translation error");
            break;
        }
      }
    } catch (err) {
      console.error("Failed to poll events:", err);
      const message = err instanceof Error ? err.message : String(err);
      setError(`Failed to poll streaming events: ${message}`);
    }
  }, [
    enforcePendingTranslationLimit,
    flushTranslationBuffer,
    normalizeTranscript,
    whisperAvailable,
  ]);

  /** Handle start/stop recording toggle */
  const handleStartRecording = useCallback(async () => {
    try {
      setError(null);
      setIsLoading(true);

      if (!isRecording) {
        // Use Whisper-enabled capture if model is available, otherwise plain streaming
        const command = whisperAvailable === false
          ? "start_streaming_capture"
          : "start_streaming_capture_with_transcription";
        console.log(`Starting capture (${command}) in system-audio mode`);
        await invoke(command, {
          inputDeviceName: null,
          captureBackend,
        });
        setIsRecording(true);
        setJapaneseText("");
        setVietnameseText("");
        resetTranslationBuffers();
        lastTranscriptNormRef.current = "";
        chunkCountRef.current = 0;
        startTimeRef.current = Date.now();
        if (command === "start_streaming_capture_with_transcription") {
          setIsTranscribing(true);
        }

        // Start polling for events
        pollingIntervalRef.current = setInterval(
          pollStreamingEvents,
          EVENT_POLL_INTERVAL_MS
        );

        // Start duration counter
        durationIntervalRef.current = setInterval(() => {
          if (startTimeRef.current) {
            const elapsed = (Date.now() - startTimeRef.current) / 1000;
            setRecordingDuration(Math.floor(elapsed));
          }
        }, DURATION_TICK_INTERVAL_MS);
      } else {
        // Stop streaming recording
        console.log("Stopping streaming audio capture");

        await invoke("stop_streaming_capture");
        // Fetch any final events emitted during stop/flush.
        await pollStreamingEvents();

        // Clean up polling after final flush.
        clearTimers();

        setIsRecording(false);
        setIsTranscribing(false);
      }
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      console.error("Audio capture error:", errorMessage);
      setError(errorMessage);
      setIsRecording(false);

      // Clean up intervals on error
      clearTimers();
    } finally {
      setIsLoading(false);
    }
  }, [
    clearTimers,
    DURATION_TICK_INTERVAL_MS,
    EVENT_POLL_INTERVAL_MS,
    captureBackend,
    isRecording,
    pollStreamingEvents,
    resetTranslationBuffers,
    whisperAvailable,
  ]);

  /** Cleanup on unmount */
  useEffect(() => {
    return () => {
      clearTimers();
    };
  }, [clearTimers]);

  /** Placeholder for text-to-speech feature */
  const handleTTS = useCallback(() => {
    if (vietnameseText) {
      console.log("Playing text-to-speech");
      // TODO: Integrate MeloTTS for audio playback
    }
  }, [vietnameseText]);

  return (
    <div className="screen main-app-screen">
      <div className="main-app-container">
        <header className="app-header">
          <h1>J2V - Japanese to Vietnamese Translator</h1>
          <p className="subtitle">Real-time Streaming Translation & Text-to-Speech</p>
        </header>

        <div className="content-section">
          {error && <div className="error-alert">{error}</div>}

          {whisperAvailable === false && (
            <div className="error-alert" style={{ background: "var(--warning-bg, #fff3cd)", color: "#856404", borderColor: "#ffc107" }}>
              Faster-Whisper model not installed – transcription is disabled.
              Please install it from the Models screen.
            </div>
          )}

          <div className="controls">
            <div className="device-selector">
              <label htmlFor="capture-backend">Capture Backend:</label>
              <select
                id="capture-backend"
                value={captureBackend}
                onChange={(e) =>
                  setCaptureBackend(e.target.value as CaptureBackendMode)
                }
                disabled={isRecording}
              >
                <option value="auto">Auto</option>
                <option value="screen-capture-kit">ScreenCaptureKit (macOS)</option>
                <option value="wasapi-loopback">WASAPI Loopback (Windows)</option>
                <option value="loopback-driver">Loopback Driver</option>
              </select>
            </div>

            <div className="controls-actions">
              <button
                className={`btn ${isRecording ? "btn-danger" : "btn-success"}`}
                onClick={handleStartRecording}
                disabled={isLoading}
              >
                {isLoading
                  ? "Processing..."
                  : isRecording
                  ? "Stop Recording"
                  : "Start Recording"}
              </button>

              <button
                className="btn btn-info"
                onClick={handleTTS}
                disabled={!vietnameseText}
              >
                Text-to-Speech
              </button>
            </div>
          </div>

          {isRecording && (
            <div className="recording-indicator">
              <div className="recording-pulse">
                <span className="pulse-dot"></span>
                <span>
                  Recording... {recordingDuration}s
                  {isTranscribing && " · Transcribing..."}
                </span>
              </div>
            </div>
          )}

          <TranscriptionDisplay
            japaneseText={japaneseText}
            vietnameseText={vietnameseText}
          />
        </div>

        <footer className="app-footer">
          <p>
            {isRecording
              ? whisperAvailable
                ? "Capturing system audio and transcribing Japanese in real-time via Whisper..."
                : "System-audio capture in progress... (install Whisper model for transcription)"
              : "Ready. Choose backend then Start Recording."}
          </p>
        </footer>
      </div>
    </div>
  );
};

export default MainApp;
