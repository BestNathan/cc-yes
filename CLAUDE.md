# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test

```bash
cargo build                  # Debug build
cargo build --release        # Release build → target/release/cc-yes (~1.1MB)
cargo test                   # 19 tests (16 unit + 3 integration)
cargo test --test integration  # Integration tests only
cargo clippy                 # Lint
```

## Architecture

cc-yes is a Claude Code plugin (single Rust binary) that auto-approves tool-use permissions when all extracted items match user-configured allowlists across 5 dimensions.

### Core pipeline

```
Tool invoked → PreToolUse hook → hook-entry.sh → cc-yes hook
  → parse command → extract 5 dimensions → match against yes rules
  → all matched? → {"decision":"approve"} → Claude skips permission prompt
  → any mismatch? → {"decision":"delegate"} → Claude shows normal prompt
                    + snapshots permissions.allow → if user clicked "Always allow"
                    → PostToolUse → cc-yes after → auto-learns new rules
```

### Module map

| Module | Role |
|--------|------|
| `config.rs` | Data types: `YesConfig` (5-dim allowlists), `ExtractedItems`, `HookInput`/`Decision` (stdin/stdout protocol), `SettingsFile` |
| `parser.rs` | `parse_bash()` splits commands on `&&`/`||`/`;`/`|`, extracts `cmd`/`files`/`url`/`env` from tokens, recursively parses `.py`/`.sh`/`.js` scripts for `imports` with limits (500 lines, 20KB, no `eval`/dynamic imports) |
| `matcher.rs` | `matches_all()` checks all extracted items against rules. Rules support prefix (`git`), exact (`cargo build`), and glob (`npm run dev:*`, `src/**`). `match_single()` is the public API for individual checks. |
| `settings.rs` | 3-layer deep-merge: `~/.claude/settings.json` → `.claude/settings.json` → `.claude/settings.local.json`. Arrays concatenated + deduplicated. Writes always go to `settings.local.json`. |
| `hook.rs` | `run_hook()` — PreToolUse entry point. Reads stdin → extracts → matches → outputs Decision. Snapshots `permissions.allow` to temp dir on delegate for auto-learn detection. |
| `after.rs` | `run_after()` — PostToolUse entry point. Compares current `permissions.allow` against snapshot. New entries = user clicked "Always allow" → learns missing items into `yes` config. |
| `main.rs` | CLI via clap: `add`/`remove`/`list`/`check`/`install` (source build) + `hook`/`after` (internal stdin mode) |

### Hook protocol

Binary receives `HookInput` JSON on stdin (`tool_name`, `tool_input`, `session_id`, `cwd`), writes `Decision` JSON to stdout (`{"decision":"approve"}` or `{"decision":"delegate"}`). Never denies — only approves or delegates.

### Plugin skeleton

- `hooks/hooks.json` — PreToolUse on `Bash` + `Write|Edit|WebFetch|WebSearch|NotebookEdit`, PostToolUse on `Bash`. All invoke `scripts/hook-entry.sh`.
- `scripts/hook-entry.sh` — Checks `CC_YES_ENABLED` and binary existence, dispatches to `cc-yes hook` or `cc-yes after`.
- `skills/install.md` — `/cc-yes:install` skill for platform detection + binary download (from GitHub Releases) or source build.
- `.github/workflows/release.yml` — Builds 5 native platform binaries on push to main, creates GitHub Release.

### Config shape

```json
{
  "yes": {
    "cmd": ["git", "cargo build", "npm run dev:*"],
    "files": ["*.rs", "*.toml", "src/**"],
    "url": ["https://docs.rs/*", "https://api.github.com/*"],
    "imports": ["os", "numpy", "fs", "axios"],
    "env": ["PATH", "HOME", "RUST_LOG", "CARGO_HOME"]
  }
}
```

`cc-yes add` writes to `.claude/settings.local.json`. `cc-yes list` shows merged result across all 3 layers.

### Autoyes

Set `yes.autoyes = true` to auto-approve ALL permission requests without rule matching.
When feishu is also configured, each auto-approved request sends a notification card.

```json
{
  "yes": {
    "autoyes": true,
    "feishu": {
      "app_id": "...",
      "app_secret": "...",
      "chat_id": "..."
    }
  }
}
```

CLI:
```bash
cc-yes autoyes enable              # Enable for current project
cc-yes autoyes enable --scope global  # Enable for all projects
cc-yes autoyes disable             # Disable for current project
cc-yes autoyes status              # Show status across all layers
```

Layer priority: local > project > global. A project-level `autoyes: false` can override a global `autoyes: true`.

### Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `CC_YES_ENABLED` | `1` | Set `0` to disable cc-yes globally |
| `CC_YES_AUTO_LEARN` | `1` | Set `0` to disable auto-learning from "Always allow" |
| `CLAUDE_PLUGIN_ROOT` | auto | Plugin install directory, used by hook-entry.sh |
| `CC_YES_REPO` | `BestNathan/cc-yes` | GitHub repo for binary downloads |
