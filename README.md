# cc-yes

Auto-approve Claude Code tool-use permissions based on user-configured allowlists.

When Claude wants to run a Bash command, cc-yes intercepts the permission prompt. If every command, file, URL, import, and env var in that invocation matches your configured rules, it auto-approves. Otherwise, it falls back to the normal prompt — never blocks.

## Install

```bash
# Via Claude Code skill (recommended):
/cc-yes:install --source    # Build from source
/cc-yes:install --bin       # Download pre-built from GitHub Releases

# Or manually:
cargo build --release
cp target/release/cc-yes ${CLAUDE_PLUGIN_ROOT}/bin/cc-yes
```

## Quick Start

```bash
# Add allowed commands
cc-yes add cmd git
cc-yes add cmd "cargo build"
cc-yes add cmd "npm run dev:*"
cc-yes add files "src/**"
cc-yes add url "https://api.github.com/*"
cc-yes add imports numpy
cc-yes add env RUST_LOG

# Check what a command would do
cc-yes check "git pull && cargo build"

# View merged config (across global → project → local)
cc-yes list
```

## How it works

Five dimensions are extracted from every tool invocation and checked against your allowlists:

| Dimension | What it captures | Example |
|-----------|-----------------|---------|
| `cmd` | Executables + subcommands | `git`, `cargo build`, `python *.py` |
| `files` | File paths read/written | `src/**.rs`, `Cargo.toml` |
| `url` | URLs accessed | `https://api.github.com/*` |
| `imports` | Script imports | `os`, `numpy`, `axios` |
| `env` | Environment variables | `RUST_LOG`, `CARGO_HOME` |

**Rule matching**: prefix (`git` matches `git status`), exact (`cargo build` matches `cargo build --release`), glob (`npm run dev:*` matches `npm run dev:build`).

**Auto-learn**: When you click "Always allow" on a permission prompt, cc-yes learns from it — automatically adding the missing commands, files, and URLs to your allowlist.

**Script awareness**: When a Bash command invokes a script (`.py`, `.sh`, `.js`), cc-yes parses the script to extract imports and internal commands. Skips files over 500 lines, 20KB, or with dynamic constructs.

## Configuration

Config lives in `settings.json` under the `yes` key, merged across 3 layers:
1. `~/.claude/settings.json` — global defaults
2. `.claude/settings.json` — project shared
3. `.claude/settings.local.json` — local overrides (gitignored)

## Env vars

| Variable | Default | Purpose |
|----------|---------|---------|
| `CC_YES_ENABLED` | `1` | Set `0` to disable |
| `CC_YES_AUTO_LEARN` | `1` | Set `0` to disable auto-learning |

## License

MIT
