# TAI — Terminal AI

A GPU-rendered terminal emulator with a built-in AI assistant, written in Rust.

TAI combines a fully functional terminal (powered by [Ghostty](https://github.com/ghostty-org/ghostty)'s VT emulator) with OpenAI's Chat Completions API, letting you ask questions, get explanations, and execute commands — all without leaving the terminal.

![Platform: macOS](https://img.shields.io/badge/platform-macOS-blue)
![Language: Rust](https://img.shields.io/badge/language-Rust-orange)
![License: MIT](https://img.shields.io/badge/license-MIT-green)

## Features

### Terminal
- Full VT emulation via `libghostty-vt` (SGR colors, alternate screen, mouse tracking, etc.)
- GPU-accelerated rendering with [Raylib](https://www.raylib.com/)
- 100,000-line scrollback buffer (configurable)
- Dynamic font resizing (`Cmd+`/`Cmd-`/`Cmd+0`)
- Text selection and clipboard support (`Cmd+C`/`Cmd+V`)
- Scrollbar minimap with density visualization, click-to-scroll, and drag navigation
- Automatic window title update with the foreground process name
- JetBrains Mono font included

### AI Assistant
- Activated with `Ctrl+/` — opens a floating command palette-style prompt
- Streaming responses displayed inline with colored output
- **Tool use**: the AI can execute shell commands via the `run_command` tool
- **Confirmation mode**: each command shows a confirmation overlay — run, cancel, or edit
- **YOLO mode** (`Ctrl+Y`): auto-execute commands without confirmation
- Multi-turn conversation with context (terminal buffer, CWD, OS, shell, command history)
- Conversation management (`/clear` to reset)
- Unified prompt history — shell commands and AI prompts share one history, navigable with arrow keys

### Minimap
- VS Code-style density minimap on the scrollbar
- Proportional density bars showing text content per line
- Viewport indicator with drag-and-drop support
- Automatically rebuilds from terminal scrollback on font size change or window resize

## Prerequisites

- **Rust** (stable, edition 2024)
- **Zig** >= 0.14 (to build `libghostty-vt`)
- **macOS** (Linux support is possible but untested)
- **OpenAI API key** (for AI features)

## Building

```bash
git clone https://github.com/tonyredondo/tai.git
cd tai
cargo build --release
```

On first build, the build script will:
1. Clone the Ghostty repository to your system cache
2. Check out the pinned commit
3. Build `libghostty-vt` as a static library using Zig
4. Generate Rust FFI bindings via `bindgen`

Subsequent builds are fast since the library is cached.

## Running

```bash
# Set your OpenAI API key
export OPENAI_API_KEY="sk-..."

# Run
cargo run --release
```

Without an API key, TAI still works as a regular terminal — AI features are simply disabled.

## Configuration

TAI reads its config from `~/.config/tai/config.toml`:

```toml
[ai]
model = "gpt-5.4"          # OpenAI model to use
api_key = ""                # Alternative to OPENAI_API_KEY env var
auto_execute = false        # Start in YOLO mode
max_context_lines = 100     # Terminal lines sent as context
max_history = 20            # Conversation turns to keep

[terminal]
font_size = 16              # Default font size in points
scrollback = 100000         # Scrollback buffer size in lines
```

All fields are optional — defaults are shown above.

## Keybindings

| Key | Action |
|---|---|
| `Ctrl+/` | Toggle AI prompt |
| `Ctrl+Y` | Toggle YOLO mode (auto-execute AI commands) |
| `Cmd++` / `Cmd+-` | Increase / decrease font size |
| `Cmd+0` | Reset font size to default |
| `Cmd+C` | Copy selected text |
| `Cmd+V` | Paste from clipboard |
| `↑` / `↓` | Navigate unified history (in AI prompt) |
| `Ctrl+J` | Insert newline in AI prompt |
| `Enter` | Submit AI prompt / confirm command |
| `Esc` | Cancel AI prompt or command |
| `e` | Edit command (in confirmation mode) |

## Architecture

```
src/
├── main.rs              # Window, render loop, input dispatch
├── config.rs            # TOML configuration
├── router.rs            # Input routing, AI interaction, tool execution
├── minimap.rs           # Scrollbar minimap with density visualization
├── overlay.rs           # Command confirmation overlay
├── selection.rs         # Text selection and clipboard
├── status_bar.rs        # Bottom status bar
├── ai/
│   ├── bridge.rs        # Async channel bridge to AI client
│   ├── client.rs        # OpenAI API streaming client
│   ├── context.rs       # System/user message construction
│   ├── conversation.rs  # Conversation history management
│   ├── tools.rs         # Tool definitions (run_command)
│   └── auth.rs          # API key resolution
└── terminal/
    ├── engine.rs        # libghostty-vt FFI wrapper
    ├── input.rs         # Keyboard/mouse → VT encoding
    ├── pty.rs           # PTY spawn and I/O
    └── renderer.rs      # Cell-by-cell GPU rendering
```

## How AI Interaction Works

1. Press `Ctrl+/` to open the AI prompt
2. Type your question or instruction and press `Enter`
3. TAI sends the query along with context (recent terminal output, CWD, OS, shell) to the AI
4. The AI can respond with text (displayed inline in cyan) or request to run commands
5. Commands go through a confirmation step (unless YOLO mode is on)
6. Command output is captured and sent back to the AI for follow-up reasoning
7. The AI can chain multiple commands in a single response

## License

MIT
