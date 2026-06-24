# Copilot Instructions for J2V Project

## Project Overview

J2V (Japanese to Vietnamese Translator) is a Tauri-based desktop application that enables real-time Japanese speech-to-text conversion, translation to Vietnamese, and text-to-speech playback.

## Tech Stack

- **Frontend**: React 18.3.1 + TypeScript 5.6.2
- **Build Tool**: Vite 6.0.3
- **Desktop Framework**: Tauri 2.0.5
- **Runtime**: Node.js 24.16.0+
- **Language**: TypeScript with JSX support

## Project Structure

```
src/
├── screens/           # Two main application screens
├── components/        # Reusable React components
├── App.tsx           # Root component with navigation
├── main.tsx          # React entry point
└── styles.css        # Global stylesheet
```

## Key Guidelines

### Development Rules
- Use React hooks for state management (`useState`, `useEffect`, etc.)
- Keep components focused and reusable
- Use TypeScript strict mode (enabled in tsconfig.json)
- CSS modules are in a single `styles.css` file
- All components use functional components with React.FC

### File Naming
- React components: PascalCase (.tsx)
- Utilities/helpers: camelCase (.ts)
- CSS classes: kebab-case

### Component Guidelines
- Props should be typed with interfaces
- Use semantic HTML
- Maintain responsive design (mobile-first approach)
- Accessibility considerations (ARIA labels, keyboard navigation)

### CSS Guidelines
- Use CSS custom properties (--primary-color, --success-color, etc.)
- Mobile breakpoints: 480px, 768px, 1024px
- Animations: Use CSS keyframes (no libraries)
- Colors: Use provided color palette in :root

## Current Screens

### Screen 1: ModelCheck (src/screens/ModelCheck.tsx)
- Displays 5 required models and their installation status
- Provides install/mark buttons for each model
- Shows continue button when all models are ready
- Props: `onAllModelsReady: () => void`

### Screen 2: MainApp (src/screens/MainApp.tsx)
- Main application interface
- Start/Stop recording button
- Text-to-Speech button
- Transcription display (Japanese/Vietnamese)
- No props required

## Important Files

- **tsconfig.json**: JSX is configured as "react-jsx"
- **vite.config.ts**: React plugin is enabled
- **package.json**: Contains all dependencies with pinned versions
- **index.html**: React root element is `<div id="root"></div>`

## Development Commands

```bash
npm run dev          # Start development server
npm run build        # Build for production
npm run tauri dev    # Run as desktop app (dev mode)
npm run tauri build  # Build desktop executable
```

## Common Tasks

### Adding a New Component
1. Create file in `src/components/` with PascalCase name
2. Export as `export default ComponentName`
3. Use TypeScript interface for props if needed
4. Add styles to `src/styles.css`

### Adding New Styles
1. Add CSS rules to `src/styles.css`
2. Use CSS custom properties for colors
3. Include media queries for responsive design
4. Use kebab-case for CSS class names

### State Management
- Use `useState` for local component state
- Props for parent-to-child communication
- Callbacks (`onEventName`) for child-to-parent
- No external state management library (keep it simple for MVP)

## Known Limitations (Current Phase)

- Audio capture is UI only (backend not implemented)
- Speech-to-text is placeholder (no Whisper integration)
- Translation is placeholder (no Ollama integration)
- Text-to-speech is placeholder (no MeloTTS integration)
- Model installation scripts are not implemented

## Next Steps (Backend)

1. Integrate Faster-Whisper for speech recognition
2. Implement Ollama + Qwen2.5-7B for translation
3. Add MeloTTS for audio playback
4. Implement audio stream capture from speakers/headphones
5. Add backend model installation handlers
6. Error handling and status messages

## Build Status

✅ TypeScript compilation passes
✅ Vite build succeeds (147.76 KB, gzipped: 47.46 KB)
✅ No console errors or warnings
✅ Responsive design tested
✅ All components render correctly

## Resources

- [Tauri Documentation](https://tauri.app/)
- [React Documentation](https://react.dev/)
- [TypeScript Handbook](https://www.typescriptlang.org/docs/)
- [Vite Guide](https://vitejs.dev/)

---

For detailed implementation information, see IMPLEMENTATION.md
For quick start guide, see SETUP_GUIDE.md
For project overview, see README.md
