# Contributing to J2V

Thanks for contributing.

## Branch strategy

- Keep `main` stable.
- Create a feature branch from `main`:
  - `feat/<short-name>`
  - `fix/<short-name>`
  - `chore/<short-name>`

Example:

```bash
git checkout -b feat/realtime-translation
```

## Local setup

```bash
npm install
python3 -m pip install -U pip
python3 -m pip install faster-whisper
npm run tauri dev
```

## Commit convention

Use clear commit messages:

- `feat: add realtime translation panel`
- `fix: prevent duplicate transcript lines`
- `chore: update readme setup section`

## Pull request checklist

- Code builds locally (`npm run build`).
- Desktop app runs locally (`npm run tauri dev`).
- No generated build artifacts committed.
- README updated if behavior/setup changed.
- PR description explains what changed and why.

## Review notes

- Keep PRs focused and small.
- Include screenshots or short recordings for UI changes.
- Mention platform tested (macOS/Windows/Linux).
