---
name: cc-yes:install
description: Install the cc-yes binary (download pre-built or build from source)
---

# cc-yes:install

Install the cc-yes binary for auto-approving Claude Code tool-use permissions.

## Usage

```
/cc-yes:install --bin       Download pre-built binary from GitHub releases
/cc-yes:install --source    Build from source: cargo build --release
```

## Option A: Download pre-built binary (`--bin`)

### 1. Detect platform

```bash
UNAME_S=$(uname -s)
UNAME_M=$(uname -m)
```

### 2. Map to artifact name

| OS | Arch | Artifact |
|----|------|----------|
| Darwin | arm64 | `cc-yes-darwin-arm64` |
| Darwin | x86_64 | `cc-yes-darwin-x64` |
| Linux | aarch64 | `cc-yes-linux-arm64` |
| Linux | x86_64 | `cc-yes-linux-x64` |
| MINGW* / MSYS* | x86_64 | `cc-yes-win-x64.exe` |

If the platform is not in this table, fall back to `--source`.

### 3. Determine download URL

```bash
# Default repo, override with CC_YES_REPO env var
REPO="${CC_YES_REPO:-user/cc-yes}"
ARTIFACT="cc-yes-<platform-suffix>"
URL="https://github.com/${REPO}/releases/latest/download/${ARTIFACT}"
```

### 4. Download and install

```bash
BIN_DIR="${CLAUDE_PLUGIN_ROOT}/bin"
mkdir -p "$BIN_DIR"

# Download
curl -fsSL "$URL" -o "${BIN_DIR}/cc-yes"

# Make executable
chmod +x "${BIN_DIR}/cc-yes"

# Verify
"${BIN_DIR}/cc-yes" --version
```

## Option B: Build from source (`--source`)

### 1. Build

```bash
cd "${CLAUDE_PLUGIN_ROOT}"
cargo build --release
```

### 2. Install

```bash
mkdir -p "${CLAUDE_PLUGIN_ROOT}/bin"
cp target/release/cc-yes "${CLAUDE_PLUGIN_ROOT}/bin/cc-yes"
chmod +x "${CLAUDE_PLUGIN_ROOT}/bin/cc-yes"

# Verify
"${CLAUDE_PLUGIN_ROOT}/bin/cc-yes" --version
```

## Notes

- Pre-built binaries are created by the GitHub Actions release workflow on push to main
- Binary is placed at `${CLAUDE_PLUGIN_ROOT}/bin/cc-yes` — the path expected by `hooks/hooks.json`
- After install, hooks become active immediately (no restart needed)
