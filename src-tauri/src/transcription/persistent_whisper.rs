use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

#[derive(Debug, Serialize)]
struct WorkerRequest<'a> {
    #[serde(rename = "type")]
    request_type: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    audio_path: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
struct WorkerResponse {
    success: bool,
    results: Option<Vec<WorkerResult>>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkerResult {
    text: String,
    language: String,
    language_probability: f32,
}

#[derive(Debug, Clone)]
pub struct PersistentTranscriptionResult {
    pub text: String,
    pub language: String,
    pub language_probability: f32,
}

pub struct PersistentWhisperWorker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    is_shutdown: bool,
}

impl PersistentWhisperWorker {
    pub fn new(model_dir: &Path, transcribe_script_path: &Path) -> Result<Self, String> {
        let worker_script_path = transcribe_script_path.with_file_name("transcribe_worker.py");
        if !worker_script_path.exists() {
            return Err(format!(
                "Persistent transcription worker script not found: {}",
                worker_script_path.display()
            ));
        }

        let python = find_python()?;
        let mut child = Command::new(&python)
            .arg(&worker_script_path)
            .arg(model_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| {
                format!(
                    "Failed to launch persistent transcription worker with {}: {}",
                    python, e
                )
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Failed to capture worker stdin".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Failed to capture worker stdout".to_string())?;
        let mut worker = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            is_shutdown: false,
        };

        let ready = worker.read_response()?;
        if !ready.success {
            return Err(
                ready
                    .error
                    .unwrap_or_else(|| "Persistent transcription worker failed to initialize".to_string()),
            );
        }

        Ok(worker)
    }

    pub fn transcribe_file(
        &mut self,
        audio_path: &Path,
    ) -> Result<Vec<PersistentTranscriptionResult>, String> {
        let audio_path_str = audio_path
            .to_str()
            .ok_or_else(|| format!("Non-UTF8 audio path: {}", audio_path.display()))?;
        let request = WorkerRequest {
            request_type: "append_chunk",
            audio_path: Some(audio_path_str),
        };

        self.send_request(&request)?;
        let response = self.read_response()?;
        if !response.success {
            return Err(response
                .error
                .unwrap_or_else(|| "Persistent transcription worker returned an unknown error".to_string()));
        }

        Ok(response
            .results
            .unwrap_or_default()
            .into_iter()
            .map(|result| PersistentTranscriptionResult {
                text: result.text,
                language: result.language,
                language_probability: result.language_probability,
            })
            .collect())
    }

    pub fn flush_pending(&mut self) -> Result<Vec<PersistentTranscriptionResult>, String> {
        let request = WorkerRequest {
            request_type: "flush",
            audio_path: None,
        };

        self.send_request(&request)?;
        let response = self.read_response()?;
        if !response.success {
            return Err(response
                .error
                .unwrap_or_else(|| "Persistent transcription worker returned an unknown error".to_string()));
        }

        Ok(response
            .results
            .unwrap_or_default()
            .into_iter()
            .map(|result| PersistentTranscriptionResult {
                text: result.text,
                language: result.language,
                language_probability: result.language_probability,
            })
            .collect())
    }

    pub fn shutdown(&mut self) {
        if self.is_shutdown {
            return;
        }
        self.is_shutdown = true;

        let request = WorkerRequest {
            request_type: "shutdown",
            audio_path: None,
        };

        let _ = self.send_request(&request);
        let _ = self.read_response();
        let _ = self.child.wait();
    }

    fn send_request(&mut self, request: &WorkerRequest<'_>) -> Result<(), String> {
        let payload = serde_json::to_string(request)
            .map_err(|e| format!("Failed to serialize worker request: {}", e))?;
        writeln!(self.stdin, "{}", payload)
            .map_err(|e| format!("Failed to write worker request: {}", e))?;
        self.stdin
            .flush()
            .map_err(|e| format!("Failed to flush worker request: {}", e))
    }

    fn read_response(&mut self) -> Result<WorkerResponse, String> {
        let mut line = String::new();
        let read = self
            .stdout
            .read_line(&mut line)
            .map_err(|e| format!("Failed to read worker response: {}", e))?;
        if read == 0 {
            return Err("Persistent transcription worker closed unexpectedly".to_string());
        }

        serde_json::from_str(line.trim())
            .map_err(|e| format!("Invalid worker response: {}", e))
    }
}

impl Drop for PersistentWhisperWorker {
    fn drop(&mut self) {
        self.shutdown();
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn find_python() -> Result<String, String> {
    for candidate in ["python3", "python"] {
        let ok = Command::new(candidate)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            return Ok(candidate.to_string());
        }
    }

    Err("Python 3 not found. Please install Python 3.9+".to_string())
}