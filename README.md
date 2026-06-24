# J2V App

Ứng dụng desktop dịch tiếng Nhật sang tiếng Việt theo thời gian thực, xây dựng với Tauri + React + TypeScript.

Mục tiêu của dự án là cung cấp luồng: nghe tiếng Nhật từ audio hệ thống -> nhận transcript -> dịch sang tiếng Việt -> chuẩn bị cho TTS.

## Tính năng hiện có

- Luồng 2 màn hình:
  - Model Setup: kiểm tra và cài model/phụ thuộc cần thiết.
  - Main App: start/stop streaming và hiển thị kết quả realtime.
- Capture audio hệ thống theo backend nền tảng (macOS/Windows) và phát event theo lô.
- Transcription realtime bằng persistent Python worker (faster-whisper), không spawn process cho từng chunk.
- Silero VAD trong worker để tách đoạn speech trước khi decode.
- Dịch Nhật -> Việt ở backend qua Ollama (model qwen2.5:7b), có retry + circuit breaker.
- Frontend xử lý buffering translation theo sequence để giữ đúng thứ tự hiển thị.

## Tính năng đang ở mức placeholder

- Nút Text-to-Speech trong UI mới là placeholder, chưa playback thật bằng MeloTTS.
- Cài MeloTTS trong màn hình model hiện vẫn chủ yếu tạo marker thư mục (chưa tải runtime đầy đủ).

## Tech stack

- Frontend: React 18, TypeScript 5, Vite 6
- Desktop: Tauri 2
- Backend app: Rust
- STT: Python faster-whisper
- Translation: Ollama HTTP API (mặc định qwen2.5:7b)

## Yêu cầu môi trường

Bạn cần cài đặt:

- Node.js + npm
- Rust toolchain (cargo, rustc)
- Python 3 (khuyến nghị 3.10+)
- Ollama (để chạy translation backend)
- Các prerequisite của Tauri theo OS

Tham khảo:

- https://tauri.app/start/prerequisites/
- https://rustup.rs/
- https://nodejs.org/
- https://ollama.com/

## Cài đặt và chạy nhanh

### 1) Cài dependency frontend

```bash
npm install
```

### 2) Cài dependency Python cho transcription

```bash
python3 -m pip install -U pip
python3 -m pip install faster-whisper onnxruntime numpy
```

### 3) Chạy app desktop ở chế độ dev

```bash
npm run tauri dev
```

Ghi chú:

- Có thể chạy web-only bằng lệnh `npm run dev`.
- Translation qua Ollama yêu cầu Ollama daemon đang chạy và model qwen2.5:7b đã pull xong.

### 4) Build production

```bash
npm run build
npm run tauri build
```

## Scripts npm

- `npm run dev`: chạy Vite dev server
- `npm run build`: chạy TypeScript build + Vite build
- `npm run preview`: preview frontend dist
- `npm run tauri`: gọi Tauri CLI (ví dụ `npm run tauri dev`, `npm run tauri build`)

## Luồng hoạt động chính

1. Frontend gọi `check_model_status` để lấy trạng thái 5 model.
2. Người dùng cài model qua `install_model`.
3. Khi bấm Start Recording:
   - Nếu Faster-Whisper sẵn sàng: gọi `start_streaming_capture_with_transcription`.
   - Nếu chưa sẵn sàng: gọi `start_streaming_capture` (chỉ capture audio event).
4. Frontend polling `get_streaming_events` mỗi 100ms để nhận:
   - `ChunkReceived`
   - `TranscriptionReady` / `TranscriptionError`
   - `TranslationReady` / `TranslationError`
5. Khi Stop Recording: gọi `stop_streaming_capture` và flush nốt event tồn.

## Mô hình model hiện tại

- Ollama:
  - Check: kiểm tra thư mục cài đặt Ollama theo OS.
  - Install: dùng cài tự động theo OS (macOS qua Homebrew).
- Qwen2.5-7B:
  - Check: parse kết quả `ollama list` để tìm tag `qwen2.5:7b`.
  - Install: chạy `ollama pull qwen2.5:7b`.
- Faster-Whisper:
  - Install: chạy script `scripts/install_whisper_model.py` (mặc định profile `small`).
  - Check: xác thực có `model.bin` + `config.json` (trực tiếp hoặc nested dir).
- Silero VAD:
  - Install: tải runtime `silero_vad_v6.onnx` vào `faster_whisper/assets` trong Python environment hiện tại.
  - Check: xác thực file runtime `silero_vad_v6.onnx` tồn tại đúng asset path mà `faster_whisper` sử dụng.
- MeloTTS:
  - Check/install tạm thời theo marker folder trong thư mục models của app.

## Capture backend và nền tảng

- macOS:
  - Mặc định frontend chọn `screen-capture-kit`.
  - Tauri bundle yêu cầu macOS tối thiểu 13.0.
- Windows:
  - Frontend có option `wasapi-loopback`.
- Chế độ `auto` vẫn có sẵn.

## Các Tauri commands đã expose

- Model:
  - `check_model_status`
  - `install_model`
- Audio:
  - `start_audio_capture`
  - `stop_audio_capture`
  - `list_audio_devices`
  - `get_audio_capture_status`
  - `start_streaming_capture`
  - `start_streaming_capture_with_transcription`
  - `stop_streaming_capture`
  - `get_streaming_events`
- STT/Translate:
  - `transcribe_audio_file`
  - `translate_text`

## Cấu trúc thư mục

```text
.
├── src/                        # Frontend React
│   ├── screens/                # ModelCheck, MainApp
│   ├── components/             # UI components
│   ├── types/                  # TypeScript types
│   └── styles.css              # Global styles
├── src-tauri/                  # Backend Rust + Tauri config
│   └── src/
│       ├── commands.rs         # Tauri commands
│       ├── audio/              # Capture/audio events
│       ├── models/             # Model check/install
│       ├── transcription/      # Persistent worker integration
│       ├── translation.rs      # Ollama translation worker
│       └── utils/              # Path/util helpers
└── scripts/                    # Python scripts cho Whisper
```

## Troubleshooting nhanh

### Không có transcript realtime

- Kiểm tra Faster-Whisper đã install thành công ở màn hình Model Setup.
- Đảm bảo Python có `faster-whisper`, `onnxruntime`, `numpy`.
- Xem log khi chạy `npm run tauri dev` để tìm lỗi worker/script.

### Không có bản dịch tiếng Việt

- Đảm bảo Ollama đang chạy local tại `http://127.0.0.1:11434`.
- Đảm bảo đã pull model `qwen2.5:7b`.
- Nếu backend báo circuit breaker mở, chờ vài giây để worker thử lại.

### Cài Ollama thất bại trên macOS

- Cần Homebrew sẵn sàng nếu dùng cài tự động từ app.
- Có thể cài thủ công từ trang Ollama rồi kiểm tra lại trong app.

## Đóng góp

Xem hướng dẫn chi tiết tại file CONTRIBUTING.md.

## License

Dự án có khai báo license trong file LICENSE.
