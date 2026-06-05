# cc-yes Design Spec

## Overview

cc-yes is a Claude Code plugin that automatically approves tool-use permission prompts when all commands, files, URLs, environment variables, and imports within a tool invocation match user-configured allowlists. When a match is not complete, it falls back to the normal Claude Code permission flow without blocking. It also learns from "Always allow" decisions to grow its ruleset automatically.

## Project Structure

```
cc-yes/
├── .claude-plugin/
│   └── plugin.json              # Plugin manifest
├── hooks/
│   └── hooks.json               # PreToolUse + PostToolUse hook definitions
├── skills/
│   └── install.md               # /cc-yes:install skill entry point
├── scripts/
│   └── hook-entry.sh            # Thin wrapper: detect binary, dispatch hook/after
├── src/
│   ├── main.rs                  # CLI entry: add/remove/list/check/hook/after/install
│   ├── hook.rs                  # PreToolUse handler: stdin JSON → decision → stdout JSON
│   ├── after.rs                 # PostToolUse handler: auto-learn from "Always allow"
│   ├── parser.rs                # Extract cmd/files/url/imports/env from tool input
│   ├── matcher.rs               # Match extracted items against yes rules (prefix, exact, glob)
│   ├── settings.rs              # 3-layer merge: ~/.claude/ → .claude/ → .claude/settings.local.json
│   └── config.rs                # yes config struct definitions + validation
├── Cargo.toml
└── Cargo.lock
```

### Install Flow

```
User installs plugin (marketplace or local path)
  → Plugin skeleton is present (hooks, scripts, skills, src)
  → No binary yet → hooks silently skip
  → User runs /cc-yes:install --source   (cargo build --release → copy to bin/)
  → Or /cc-yes:install --bin             (download pre-built from GitHub releases)
  → Binary at bin/cc-yes → hooks become active
```

## Configuration: `yes` Object

Located in `settings.json`, merged across 3 layers (priority low→high):
1. `~/.claude/settings.json` — user global defaults
2. `.claude/settings.json` — project shared
3. `.claude/settings.local.json` — local overrides (gitignored)

Deep merge on the `yes` object: arrays are concatenated and deduplicated. Higher layers can add to, but not remove from, lower layers (explicit array replacement resets).

### Five Dimensions

```json
{
  "yes": {
    "cmd": [
      "git",
      "cargo build",
      "npm run dev:*",
      "python *.py",
      "bash *.sh"
    ],
    "files": [
      "*.md",
      "*.rs",
      "*.toml",
      "Cargo.toml",
      "src/**",
      "tests/**"
    ],
    "url": [
      "https://docs.rs/*",
      "https://github.com/*",
      "https://api.github.com/*"
    ],
    "imports": [
      "os",
      "json",
      "pathlib",
      "numpy",
      "fs",
      "path",
      "axios"
    ],
    "env": [
      "PATH",
      "HOME",
      "RUST_LOG",
      "CARGO_HOME",
      "DEBUG"
    ]
  }
}
```

| Dimension | Meaning | Match syntax |
|-----------|---------|--------------|
| `cmd` | Executables + subcommands + arg patterns | Prefix (`git`), exact (`cargo build`), glob (`npm run dev:*`, `python *.py`) |
| `files` | File paths read/written | Glob patterns (`*.rs`, `src/**`, `Cargo.toml`) |
| `url` | URLs accessed | Glob patterns (`https://docs.rs/*`) |
| `imports` | Packages/modules imported by scripts | Exact names (`os`, `numpy`, `fs`, `axios`) |
| `env` | Environment variables used/set | Exact names (`RUST_LOG`, `CARGO_HOME`) |

## Extraction Rules

cc-yes extracts from tool input into the five dimensions, independent of which tool triggered the hook:

| Tool | Extraction |
|------|------------|
| **Bash** | Parse command tree (`&&`, `\|\|`, `;`, `\|`) → extract executables as `cmd`, detect file operands as `files`, URL arguments as `url`, env var references (`$VAR`/`export VAR`) as `env`. If command involves a script (python/bash/node), also parse the script file for `imports`, internal `cmd` calls, `files`, and `url`. |
| **Write** | Target path → `files` |
| **Edit** | Target path → `files` |
| **WebFetch** | URL → `url` |
| **WebSearch** | No extractable dimensions → delegate |
| **NotebookEdit** | Target path → `files` |

### Script Deep Parsing

When a Bash command invokes a script file (`.py`, `.sh`, `.js`, `.ts`):

| Script type | Parse for |
|-------------|-----------|
| **Bash (.sh)** | All invoked commands, file paths, URLs, env vars |
| **Python (.py)** | `import` statements, `subprocess.run()` / `os.system()` calls, `open()` file paths, URL strings |
| **JavaScript (.js/.ts)** | `require()` / `import` statements, `child_process.exec()` calls, `fs.readFile`/`writeFile` paths, URL strings |

**Script parsing limits**: Skip deep parsing and delegate if:
- File exceeds 500 lines or 20KB
- Import/require graph depth exceeds 3 levels
- File contains dynamic imports (`import(variable)`, `require(variable)`) or `eval()`-like constructs

If a script file cannot be read, cannot be parsed, or exceeds these limits, treat its contents as unknown → delegate.

## Decision Logic

```
For each tool invocation:
  1. Extract all items across 5 dimensions
  2. For each extracted item, check against corresponding yes.<dimension> rules
  3. All match → output {"decision": "approve"}
  4. Any mismatch → output {"decision": "delegate"}
```

**Unparseable rule**: If cc-yes cannot extract any items across the 5 dimensions from a tool invocation (e.g., WebSearch with only a query string, unrecognized tool, or Bash command that doesn't parse cleanly), delegate immediately — cc-yes only handles what it can parse with confidence.

**Critical rule**: cc-yes never denies. It only approves or delegates. If it cannot parse or cannot confirm everything, Claude Code's normal permission flow takes over.

## Hook Architecture

### hooks/hooks.json

```json
{
  "description": "cc-yes: auto-approve based on yes rules",
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [{
          "type": "command",
          "command": "${CLAUDE_PLUGIN_ROOT}/scripts/hook-entry.sh"
        }]
      },
      {
        "matcher": "Write|Edit|WebFetch|WebSearch|NotebookEdit",
        "hooks": [{
          "type": "command",
          "command": "${CLAUDE_PLUGIN_ROOT}/scripts/hook-entry.sh"
        }]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [{
          "type": "command",
          "command": "${CLAUDE_PLUGIN_ROOT}/scripts/hook-entry.sh after"
        }]
      }
    ]
  }
}
```

### scripts/hook-entry.sh

```bash
#!/bin/bash
# Check if cc-yes is globally disabled
if [ "${CC_YES_ENABLED:-1}" = "0" ]; then
  exit 0
fi

BIN="${CLAUDE_PLUGIN_ROOT}/bin/cc-yes"
if [ ! -x "$BIN" ]; then
  exit 0  # Binary not installed yet, silently skip
fi

case "${1:-hook}" in
  after) exec "$BIN" after ;;
  *)     exec "$BIN" hook ;;
esac
```

### PreToolUse Flow

```
Claude Code fires PreToolUse hook
  → hook-entry.sh → cc-yes hook
  → Receives JSON via stdin:
      { "tool_name": "Bash", "tool_input": { "command": "...", "description": "..." },
        "session_id": "...", "cwd": "/path/to/project" }
  → Parse tool_input → extract dimensions
  → Load merged yes config from settings.json layers
  → Check all extracted items against rules
  → All match → stdout: {"decision": "approve"}
  → Any miss → stdout: {"decision": "delegate"}
```

### PostToolUse Flow (Auto-Learn)

**Detection mechanism: snapshot comparison.**

When cc-yes hook decides to delegate (not approve), it writes a snapshot of the current `permissions.allow` array from `settings.local.json` to a temp file: `/tmp/cc-yes-<session_id>.json`.

```
PreToolUse: delegate decision
  → cc-yes hook snapshots permissions.allow → /tmp/cc-yes-<session>.json

User clicks "Always allow"
  → Claude Code writes new entry to permissions.allow in settings.local.json

PostToolUse: cc-yes after
  → Check CC_YES_AUTO_LEARN (default: 1)
  → Read current permissions.allow from settings.local.json
  → Read snapshot from /tmp/cc-yes-<session>.json
  → Diff: new entries? → user clicked "Always allow"
  → Only if new entries found:
    → Parse the executed command
    → Extract all 5 dimensions
    → Add missing items to yes config in .claude/settings.local.json
    → Deep-merge into existing yes block
  → Clean up snapshot file

If user clicked "Yes" (one-time): no new permissions.allow entry → cc-yes does nothing.
If cc-yes hook approved: no snapshot was written → cc-yes after skips (already allowed).
```

**Snapshot temp file naming:** `/tmp/cc-yes-${session_id}.json` — unique per session, cleaned up after PostToolUse.

## Environment Variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `CC_YES_ENABLED` | `1` | Set to `0` to globally disable cc-yes (hooks become no-ops) |
| `CC_YES_AUTO_LEARN` | `1` | Set to `0` to disable automatic yes config updates from "Always allow" |

## CLI

```
cc-yes install --bin         Download pre-built binary from GitHub releases
cc-yes install --source      Build from source: cargo build --release → copy to bin/

cc-yes add cmd git           Add rule to .claude/settings.local.json
cc-yes add cmd "cargo build"
cc-yes add cmd "npm run dev:*"
cc-yes add files "*.rs"
cc-yes add files "src/**"
cc-yes add url "https://docs.rs/*"
cc-yes add imports numpy
cc-yes add env RUST_LOG

cc-yes remove cmd git        Remove rule from settings.local.json
cc-yes remove files "*.rs"

cc-yes list                  Show merged yes config (all dimensions)
cc-yes list cmd              Show only cmd rules
cc-yes list files            Show only files rules

cc-yes check "git pull"      Dry-run: parse command and show what matches/misses

cc-yes hook                  Called internally by Claude Code hooks (stdin mode)
cc-yes after                 Called internally by PostToolUse hook (stdin mode)
```

`cc-yes add` always writes to `.claude/settings.local.json`, appending to the existing `yes` block.

`cc-yes check` is a dry-run for testing rules:
```
$ cc-yes check "git pull && rm -rf /"
cmd: git → ✅
cmd: rm → ❌
→ would NOT auto-approve
```

## Release & Distribution

- Source published on GitHub
- Pre-built binaries published on GitHub Releases for: darwin-arm64, darwin-x64, linux-arm64, linux-x64, win-x64
- Plugin installed via marketplace or `git clone` into plugins directory
- Binary obtained via `/cc-yes:install --bin` (download) or `--source` (cargo build)

## Skill Entry Points

- `/cc-yes:install` — Install the binary (`--bin` or `--source`)
- Future: `/cc-yes:add` — Guide for adding rules interactively
