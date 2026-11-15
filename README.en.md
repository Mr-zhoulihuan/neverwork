# NeverWork

An AI-native IDE written in pure Rust, self-drawn with GPUI (the engine behind Zed). The UI, editor and multi-agent collaboration are all built from scratch — no WebView.

> The actual code lives in the [`NeverWork/`](NeverWork/) subdirectory (a standalone Cargo workspace). The repository root only holds planning docs, design files and screenshots.

## Quick start

```bash
cd NeverWork

cargo run -p neverwork        # launch the app
cargo test --workspace        # all tests (40)
cargo build -p neverwork      # build the app only
```

Requires Rust stable + MSVC toolchain (developed on rustc 1.100.0).

Set these environment variables to enable AI chat:

```bash
set NW_API_KEY=sk-xxx
set NW_MODEL=gpt-4o                          # optional
set NW_BASE_URL=https://api.openai.com/v1    # optional
set NW_PROVIDER=anthropic                    # optional, for claude models
```

Other runtime variables: `NW_ROOT` (workspace root), `NW_VIEW=team` (open the team editor directly), `NW_OPEN=<path>` (open a file on start), `NW_CONFIG_DIR` (override `~/.neverwork`).

## Layout

```
NeverWork/
  crates/
    core/    domain model (Workspace / Team / Role / Session / Message) + atomic JSON config
    ui/      design tokens (theme) + base component library
    agent/   OpenAI-compatible / Anthropic providers (SSE streaming) + team orchestration
    app/     GPUI UI
              state.rs          state and behaviour (workspace / session / keyboard / sending)
              text_buffer.rs    unified text buffer: cursor / selection / undo-redo
              input_handler.rs  the only entry point for IME and text insertion
              keymap.rs         keystroke -> edit command routing
              line_cache.rs     per-line highlight cache
              highlight.rs      lightweight lexical highlighter
              filesystem.rs     directory walk / read-write / recent files
              views/            sidebar / chat / team / editor / files / palette
  design/   design spec, HTML mockups and run screenshots
```

## Implemented

- **Three-column UI**: icon rail + session card list / chat and editor / file tree, changes, preview
- **Multi-agent teams**: custom roles (model, prompt, identity colour) with pipeline / parallel / supervisor modes
- **Streaming chat**: provider abstraction with SSE chunk-by-chunk rendering and mid-stream cancel
- **Editor**: cursor, selection, undo/redo, clipboard, click-to-position per character, syntax highlighting, atomic save, virtual scrolling
- **File tree**: expand on demand, recent changes, read-only preview
- **Command palette**: Cmd/Ctrl+K

## Not implemented yet

- Agent tool calls (read/write files, run commands) with diff output written back
- `Supervisor` orchestration currently behaves like parallel; no real dispatching
- Find and replace, multi-cursor, code folding
- Remote workspace server (M3)
- LSP / tree-sitter / terminal / Git integration / global search

See [`NeverWork/README.md`](NeverWork/README.md) and [`gpui-ide-plan-v2.md`](gpui-ide-plan-v2.md) for details and milestones.

## Development environment note

The current dev machine is a GameViewer remote session at 200% DPI: GPUI derives scale=2 from
`GetDpiForWindow/96`, but the window is created at 1:1 physical pixels, so content is enlarged and
cropped on the right. Layout therefore folds responsively using
`visible_w = bounds.width / scale` (collapses the right panel below 1080, keeps only the centre
below 820); the "◧ Focus" title-bar toggle switches manually. A normal display (scale=1) shows all
three columns correctly.

## License

Apache-2.0
