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

## Steps

1. Detect platform (uname -s, uname -m)
2. If `--bin`: download matching binary from GitHub releases to `${CLAUDE_PLUGIN_ROOT}/bin/cc-yes`
3. If `--source`: run `cargo build --release` and copy `target/release/cc-yes` to `${CLAUDE_PLUGIN_ROOT}/bin/cc-yes`
4. Verify: `${CLAUDE_PLUGIN_ROOT}/bin/cc-yes --version` prints version
