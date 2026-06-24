# J2V App

Ứng dụng desktop dịch tiếng Nhật sang tiếng Việt theo luồng thời gian thực, xây dựng bằng Tauri + React + TypeScript.

Mục tiêu của dự án là tạo trải nghiệm "nghe tiếng Nhật -> nhận transcript nhanh -> dịch sang tiếng Việt -> phát lại TTS" trong một app desktop gọn nhẹ.

## 1) Trạng thái hiện tại

Đã làm:
- Luồng 2 màn hình: kiểm tra model -> màn hình chính.
- Backend Tauri (Rust) cho kiểm tra/cài model.
- Streaming capture audio từ hệ thống (loopback theo nền tảng).
- Nhận event realtime từ backend lên frontend.
- Tích hợp transcribe bằng faster-whisper (Python script) trong luồng streaming.

Chưa hoàn thiện:
- Dịch Nhật -> Việt realtime trong UI hiện mới là placeholder.
- Text-to-Speech hiện mới là placeholder ở frontend.
- Cài đặt model Qwen/Silero/MeloTTS hiện chủ yếu tạo thư mục marker, chưa tải model thực tế.

## 2) Công nghệ sử dụng

- Frontend: React 18 + TypeScript + Vite
- Desktop shell: Tauri 2
- Backend: Rust
- STT: faster-whisper (Python)

Phiên bản chính (tham chiếu từ mã nguồn hiện tại):
- Node: khuyến nghị 20+
- React: 18.3.1
- TypeScript: 5.6.x
- Vite: 6.0.x
- Tauri: 2.0.x

## 3) Yêu cầu môi trường

Bạn cần cài đặt:
- Node.js + npm
- Rust toolchain (cargo, rustc)
- Python 3 (khuyến nghị 3.10+)

Gợi ý nhanh:
- Node: https://nodejs.org
- Rust: https://rustup.rs
- Tauri prerequisites: https://tauri.app/start/prerequisites/

Lưu ý:
- Trên macOS, backend hiện ưu tiên backend capture kiểu ScreenCaptureKit.
- Để cài faster-whisper model thành công, môi trường Python phải có package faster-whisper.

## 4) Cài đặt và chạy dự án

### 4.1 Cài dependency JavaScript

```bash
npm install
```

### 4.2 Cài dependency Python cho Whisper

```bash
python3 -m pip install -U pip
python3 -m pip install faster-whisper
```

### 4.3 Chạy web dev (Vite)

```bash
npm run dev
```

### 4.4 Chạy app desktop (Tauri dev)

```bash
npm run tauri dev
```

### 4.5 Build production

```bash
npm run build
npm run tauri build
```

## 5) Scripts chính

- `npm run dev`: chạy Vite dev server
- `npm run build`: type-check + build frontend
- `npm run preview`: preview frontend build
- `npm run tauri dev`: chạy desktop app chế độ dev
- `npm run tauri build`: build desktop bundle

## 6) Luồng hoạt động

1. Màn hình Model Setup gọi command backend để kiểm tra trạng thái model.
2. Người dùng cài model từ UI (qua Tauri commands).
3. Khi vào màn hình chính, bấm Start Recording để bắt đầu streaming capture.
4. Frontend polling event từ backend (`get_streaming_events`) để cập nhật trạng thái/chunk/transcript.
5. Nếu faster-whisper sẵn sàng, backend sẽ chạy worker transcription theo cửa sổ thời gian ngắn và trả event `TranscriptionReady`.
6. Khi bấm Stop Recording, backend flush dữ liệu và trả metadata file ghi âm.

## 7) Cấu trúc thư mục

```text
.
├── src/                        # Frontend React
│   ├── screens/                # Màn hình ModelCheck và MainApp
│   ├── components/             # Component tái sử dụng
│   ├── types/                  # Kiểu dữ liệu frontend
│   └── styles.css              # CSS global
├── src-tauri/                  # Backend Rust + cấu hình Tauri
│   └── src/
│       ├── commands.rs         # Tauri commands bridge frontend <-> backend
│       ├── audio/              # Capture audio theo nền tảng
│       ├── models/             # Check/install model
│       ├── transcription/      # Worker và wrapper transcribe
│       └── utils/              # Đường dẫn, tiện ích
└── scripts/                    # Python scripts (install/transcribe Whisper)
```

## 8) Các Tauri commands đang dùng

Nhóm model:
- `check_model_status`
- `install_model`

Nhóm audio/transcription:
- `start_audio_capture`
- `stop_audio_capture`
- `list_audio_devices`
- `get_audio_capture_status`
- `start_streaming_capture`
- `start_streaming_capture_with_transcription`
- `stop_streaming_capture`
- `get_streaming_events`
- `transcribe_audio_file`

## 9) Lưu ý vận hành

- Nếu cài Faster-Whisper bị lỗi, kiểm tra:
  - Python đang dùng có đúng không (`python3 --version`)
  - Package `faster-whisper` đã được cài chưa
  - Quyền ghi vào thư mục model của ứng dụng
- Nếu không thấy transcript realtime:
  - Xác nhận model faster-whisper đã cài thành công ở màn hình Model Setup
  - Kiểm tra log backend khi chạy `npm run tauri dev`
- Trên một số thiết bị, chất lượng capture loopback ảnh hưởng trực tiếp chất lượng STT.

## 10) Roadmap đề xuất

- Tích hợp translator backend thật (ví dụ Ollama + Qwen) cho luồng Nhật -> Việt.
- Tích hợp TTS thực (MeloTTS) và playback theo chunk.
- Chuẩn hóa cơ chế quản lý model (versioning, verify checksum, retry).
- Bổ sung test tự động cho commands Rust và luồng frontend.
- Thêm telemetry/logging có cấu trúc để debug realtime pipeline.

## 11) Đóng góp

Nếu bạn muốn đóng góp:
- Fork repository
- Tạo branch tính năng
- Commit rõ ràng theo từng thay đổi
- Mở Pull Request với mô tả ngắn gọn và cách kiểm thử

## 12) License

Dự án hiện chưa khai báo file LICENSE trong repository. Bạn nên thêm LICENSE (ví dụ MIT) để rõ ràng quyền sử dụng.
