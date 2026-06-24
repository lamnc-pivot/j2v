use crossbeam_channel::{bounded, unbounded, Sender};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_QWEN_MODEL: &str = "qwen2.5:7b";
const OLLAMA_GENERATE_URL: &str = "http://127.0.0.1:11434/api/generate";
const MAX_TRANSLATION_ATTEMPTS: u32 = 3;
const RETRY_BASE_BACKOFF_MS: u64 = 250;
const CIRCUIT_BREAKER_FAILURE_THRESHOLD: u32 = 5;
const CIRCUIT_BREAKER_OPEN_SECS: u64 = 20;

static TRANSLATION_WORKER: OnceLock<TranslationWorkerHandle> = OnceLock::new();

#[derive(Debug)]
struct CircuitBreaker {
    consecutive_failures: u32,
    open_until: Option<Instant>,
}

struct TranslationWorkerHandle {
    task_tx: Sender<TranslationTask>,
}

struct TranslationTask {
    source_text: String,
    response_tx: Sender<Result<String, String>>,
}

#[derive(Debug)]
struct TranslationFailure {
    message: String,
    retryable: bool,
}

impl TranslationFailure {
    fn retryable(message: String) -> Self {
        Self {
            message,
            retryable: true,
        }
    }

    fn non_retryable(message: String) -> Self {
        Self {
            message,
            retryable: false,
        }
    }
}

impl CircuitBreaker {
    fn new() -> Self {
        Self {
            consecutive_failures: 0,
            open_until: None,
        }
    }

    fn reject_message_if_open(&self, now: Instant) -> Option<String> {
        let open_until = self.open_until?;
        if now >= open_until {
            return None;
        }

        let remaining_secs = open_until.duration_since(now).as_secs().max(1);
        Some(format!(
            "Translation temporarily unavailable: Ollama circuit breaker is open ({}s remaining)",
            remaining_secs
        ))
    }

    fn close_if_elapsed(&mut self, now: Instant) {
        if let Some(open_until) = self.open_until {
            if now >= open_until {
                self.open_until = None;
                self.consecutive_failures = 0;
                log::info!("Translation circuit breaker closed; resuming requests");
            }
        }
    }

    fn on_success(&mut self) {
        if self.consecutive_failures > 0 {
            log::info!(
                "Translation worker recovered after {} consecutive failures",
                self.consecutive_failures
            );
        }

        self.consecutive_failures = 0;
    }

    fn on_failure(&mut self, error_message: String) -> String {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);

        if self.consecutive_failures < CIRCUIT_BREAKER_FAILURE_THRESHOLD {
            return error_message;
        }

        self.open_until = Some(Instant::now() + Duration::from_secs(CIRCUIT_BREAKER_OPEN_SECS));
        let degraded = format!(
            "Translation degraded: Ollama had {} consecutive failures. Circuit breaker opened for {}s. Last error: {}",
            self.consecutive_failures,
            CIRCUIT_BREAKER_OPEN_SECS,
            error_message
        );
        log::warn!("{}", degraded);
        degraded
    }
}

#[derive(Debug, Serialize)]
struct OllamaGenerateOptions {
    temperature: f32,
}

#[derive(Debug, Serialize)]
struct OllamaGenerateRequest<'a> {
    model: &'a str,
    prompt: String,
    stream: bool,
    keep_alive: &'a str,
    options: OllamaGenerateOptions,
}

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: Option<String>,
    error: Option<String>,
}

pub fn translate_ja_to_vi(source_text: &str) -> Result<String, String> {
    let trimmed = source_text.trim();
    if trimmed.is_empty() {
        return Err("Source text is empty".to_string());
    }

    let worker = TRANSLATION_WORKER.get_or_init(spawn_translation_worker);
    let (response_tx, response_rx) = bounded::<Result<String, String>>(1);

    worker
        .task_tx
        .send(TranslationTask {
            source_text: trimmed.to_string(),
            response_tx,
        })
        .map_err(|_| "Translation worker is unavailable".to_string())?;

    response_rx
        .recv()
        .map_err(|_| "Translation worker did not return a response".to_string())?
}

pub fn model_name() -> &'static str {
    DEFAULT_QWEN_MODEL
}

fn spawn_translation_worker() -> TranslationWorkerHandle {
    let (task_tx, task_rx) = unbounded::<TranslationTask>();

    std::thread::spawn(move || {
        let client = match build_http_client() {
            Ok(client) => client,
            Err(error) => {
                log::error!("Failed to initialize translation HTTP client: {}", error);
                drain_tasks_with_error(&task_rx, error);
                return;
            }
        };

        log::info!("Translation worker started (persistent Ollama client)");
        let mut breaker = CircuitBreaker::new();

        while let Ok(task) = task_rx.recv() {
            let now = Instant::now();
            if let Some(message) = breaker.reject_message_if_open(now) {
                let _ = task.response_tx.send(Err(message));
                continue;
            }
            breaker.close_if_elapsed(now);

            match translate_with_retry(&client, &task.source_text) {
                Ok(translated) => {
                    breaker.on_success();
                    let _ = task.response_tx.send(Ok(translated));
                }
                Err(message) => {
                    let _ = task.response_tx.send(Err(breaker.on_failure(message)));
                }
            }
        }

        log::info!("Translation worker exiting");
    });

    TranslationWorkerHandle { task_tx }
}

fn drain_tasks_with_error(
    task_rx: &crossbeam_channel::Receiver<TranslationTask>,
    message: String,
) {
    while let Ok(task) = task_rx.recv() {
        let _ = task.response_tx.send(Err(message.clone()));
    }
}

fn build_http_client() -> Result<Client, String> {
    Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

fn translate_with_retry(client: &Client, source_text: &str) -> Result<String, String> {
    let mut last_error = "Unknown translation error".to_string();

    for attempt in 1..=MAX_TRANSLATION_ATTEMPTS {
        match translate_with_ollama(client, source_text) {
            Ok(translated) => {
                if attempt > 1 {
                    log::info!(
                        "Translation succeeded on retry attempt {}/{}",
                        attempt,
                        MAX_TRANSLATION_ATTEMPTS
                    );
                }
                return Ok(translated);
            }
            Err(error) => {
                last_error = error.message;

                if !error.retryable || attempt == MAX_TRANSLATION_ATTEMPTS {
                    break;
                }

                let backoff_ms = RETRY_BASE_BACKOFF_MS * (1_u64 << (attempt - 1));
                log::warn!(
                    "Transient translation error on attempt {}/{}: {}. Retrying in {}ms",
                    attempt,
                    MAX_TRANSLATION_ATTEMPTS,
                    last_error,
                    backoff_ms
                );
                thread::sleep(Duration::from_millis(backoff_ms));
            }
        }
    }

    Err(last_error)
}

fn translate_with_ollama(client: &Client, source_text: &str) -> Result<String, TranslationFailure> {
    let request = OllamaGenerateRequest {
        model: DEFAULT_QWEN_MODEL,
        prompt: build_translation_prompt(source_text),
        stream: false,
        keep_alive: "30m",
        options: OllamaGenerateOptions { temperature: 0.0 },
    };

    let response = client
        .post(OLLAMA_GENERATE_URL)
        .json(&request)
        .send()
        .map_err(|e| {
            TranslationFailure::retryable(format!(
                "Failed to call Ollama API. Ensure Ollama is running at {}: {}",
                OLLAMA_GENERATE_URL, e
            ))
        })?;

    let status = response.status();
    let body = response
        .text()
        .map_err(|e| TranslationFailure::retryable(format!("Failed to read Ollama response: {}", e)))?;

    if !status.is_success() {
        let retryable_status = status.is_server_error() || status.as_u16() == 429;

        if let Ok(parsed) = serde_json::from_str::<OllamaGenerateResponse>(&body) {
            if let Some(error) = parsed.error {
                return Err(TranslationFailure {
                    message: format!("Ollama translation failed: {}", error),
                    retryable: retryable_status,
                });
            }
        }

        return Err(TranslationFailure {
            message: format!(
                "Ollama translation failed with status {}: {}",
                status,
                body.trim()
            ),
            retryable: retryable_status,
        });
    }

    let parsed: OllamaGenerateResponse = serde_json::from_str(&body)
        .map_err(|e| TranslationFailure::retryable(format!("Invalid Ollama JSON response: {}", e)))?;

    if let Some(error) = parsed.error {
        return Err(TranslationFailure::non_retryable(format!(
            "Ollama translation failed: {}",
            error
        )));
    }

    let translated = parsed.response.unwrap_or_default().trim().to_string();
    if translated.is_empty() {
        return Err(TranslationFailure::non_retryable(
            "Qwen returned an empty translation".to_string(),
        ));
    }

    Ok(translated)
}

fn build_translation_prompt(source_text: &str) -> String {
    format!(
        "You are a professional Japanese to Vietnamese translator. Translate naturally and accurately into Vietnamese. Preserve meaning, names, and numbers. Output only Vietnamese text with no explanations.\n\nJapanese:\n{}\n\nVietnamese:",
        source_text
    )
}
