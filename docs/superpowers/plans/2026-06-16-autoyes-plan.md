# Autoyes 功能实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 添加 autoyes 配置项，开启后自动允许所有权限请求，可选通过飞书发送通知卡片。

**Architecture:** 在 YesConfig 新增 `autoyes: Option<bool>` 字段，hook 流程最前端检查 autoyes 值，为 true 时直接 approve 并可选发 Feishu 通知。CLI 新增 `autoyes enable/disable/status` 子命令。

**Tech Stack:** Rust, serde/serde_json, clap, tokio (feishu async), reqwest

---

## 文件变更地图

| 文件 | 变更类型 | 职责 |
|------|----------|------|
| `src/config.rs` | 修改 | `YesConfig` 新增 `autoyes` 字段；`is_empty()` 检查 autoyes |
| `src/settings.rs` | 修改 | `merge_into()` autoyes 覆盖逻辑；`read_autoyes()` 辅助函数 |
| `src/feishu.rs` | 修改 | 新增 `send_autoyes_notification()` |
| `src/hook.rs` | 修改 | `run_hook()` 前置 autoyes 检查 |
| `src/main.rs` | 修改 | `Autoyes` CLI 子命令 + 参数解析 |
| `src/settings.rs` (tests) | 修改 | 新增 autoyes merge 测试 |
| `tests/integration.rs` | 修改 | autoyes hook 集成测试 |
| `CLAUDE.md` | 修改 | 更新文档 |

---

### Task 1: 配置结构 — `autoyes` 字段 + `is_empty()` 更新

**Files:**
- Modify: `src/config.rs:1-29` (YesConfig struct + is_empty method)

- [ ] **Step 1: 在 YesConfig 中新增 autoyes 字段**

在 `src/config.rs` 的 `YesConfig` 结构体中，`env` 字段下方新增：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct YesConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cmd: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub url: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feishu: Option<FeishuConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub autoyes: Option<bool>,
}
```

- [ ] **Step 2: 更新 `is_empty()` 方法**

`is_empty()` 现在需要同时检查 autoyes 字段，防止 autoyes=true 但其他维度为空时 hook 提前退出：

```rust
impl YesConfig {
    /// Returns true if all five dimensions are empty AND autoyes is not enabled.
    pub fn is_empty(&self) -> bool {
        self.cmd.is_empty()
            && self.files.is_empty()
            && self.url.is_empty()
            && self.imports.is_empty()
            && self.env.is_empty()
            && self.autoyes != Some(true)
    }
}
```

- [ ] **Step 3: 编译验证**

```bash
cargo build 2>&1 | head -20
```

Expected: 编译通过（可能有 unused field warning for autoyes，后续任务会用上）

- [ ] **Step 4: Commit**

```bash
git add src/config.rs
git commit -m "feat(autoyes): add autoyes field to YesConfig, update is_empty()"
```

---

### Task 2: Settings — `merge_into()` autoyes 覆盖逻辑

**Files:**
- Modify: `src/settings.rs:47-63` (merge_into function)

- [ ] **Step 1: 更新 `merge_into()` 函数**

在 `feishu` 覆盖逻辑下方，新增 autoyes 覆盖（高层覆盖低层）：

```rust
fn merge_into(base: &mut YesConfig, other: &YesConfig) {
    base.cmd.extend(other.cmd.iter().cloned());
    base.files.extend(other.files.iter().cloned());
    base.url.extend(other.url.iter().cloned());
    base.imports.extend(other.imports.iter().cloned());
    base.env.extend(other.env.iter().cloned());
    // feishu: higher-priority layer overrides
    if other.feishu.is_some() {
        base.feishu = other.feishu.clone();
    }
    // autoyes: higher-priority layer overrides
    if other.autoyes.is_some() {
        base.autoyes = other.autoyes;
    }

    dedup(&mut base.cmd);
    dedup(&mut base.files);
    dedup(&mut base.url);
    dedup(&mut base.imports);
    dedup(&mut base.env);
}
```

- [ ] **Step 2: 编译验证**

```bash
cargo build 2>&1 | head -10
```

Expected: 编译通过

- [ ] **Step 3: Commit**

```bash
git add src/settings.rs
git commit -m "feat(autoyes): merge_into autoyes override logic"
```

---

### Task 3: Settings 测试 — 3 层 autoyes 覆盖

**Files:**
- Modify: `src/settings.rs:131-184` (tests module)

- [ ] **Step 1: 添加 3 层 autoyes 覆盖测试**

在 `src/settings.rs` 的 tests 模块末尾新增：

```rust
#[test]
fn test_autoyes_global_override() {
    let tmp = std::env::temp_dir().join("cc-yes-test-autoyes-override");
    let _ = std::fs::remove_dir_all(&tmp);

    // Layer 1: global has autoyes=true
    write_temp_file(&tmp, "home/.claude/settings.json", r#"{"yes":{"autoyes":true}}"#);
    // Layer 2: project has autoyes=false
    write_temp_file(&tmp, "project/.claude/settings.json", r#"{"yes":{"autoyes":false}}"#);

    std::env::set_var("HOME", tmp.join("home").to_str().unwrap());

    let (merged, _) = load_merged(&tmp.join("project")).unwrap();
    assert_eq!(merged.autoyes, Some(false), "project-level false should override global true");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_autoyes_local_override() {
    let tmp = std::env::temp_dir().join("cc-yes-test-autoyes-local");
    let _ = std::fs::remove_dir_all(&tmp);

    // Layer 1: global has autoyes=true
    write_temp_file(&tmp, "home/.claude/settings.json", r#"{"yes":{"autoyes":true}}"#);
    // Layer 2: project has autoyes=false
    write_temp_file(&tmp, "project/.claude/settings.json", r#"{"yes":{"autoyes":false}}"#);
    // Layer 3: local has autoyes=true
    write_temp_file(&tmp, "project/.claude/settings.local.json", r#"{"yes":{"autoyes":true}}"#);

    std::env::set_var("HOME", tmp.join("home").to_str().unwrap());

    let (merged, _) = load_merged(&tmp.join("project")).unwrap();
    assert_eq!(merged.autoyes, Some(true), "local-level true should override project false");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_autoyes_not_set_returns_none() {
    let tmp = std::env::temp_dir().join("cc-yes-test-autoyes-none");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join(".claude")).unwrap();
    std::env::set_var("HOME", tmp.to_str().unwrap());

    let (merged, _) = load_merged(&tmp).unwrap();
    assert_eq!(merged.autoyes, None, "autoyes should be None when not configured");

    let _ = std::fs::remove_dir_all(&tmp);
}
```

- [ ] **Step 2: 运行测试**

```bash
cargo test settings::tests::test_autoyes -- --nocapture
```

Expected: 3 个测试全部 PASS

- [ ] **Step 3: Commit**

```bash
git add src/settings.rs
git commit -m "test(autoyes): 3-layer override unit tests"
```

---

### Task 4: Feishu — `send_autoyes_notification()` 通知卡片

**Files:**
- Modify: `src/feishu.rs` (add new function after existing code)

- [ ] **Step 1: 添加 `send_autoyes_notification()` 函数**

在 `feishu.rs` 文件末尾（`update_card` 函数后）新增：

```rust
/// Send a read-only "auto-allowed" notification card to Feishu.
/// Same card format as the approval card, but green header, no buttons.
pub fn send_autoyes_notification(config: &FeishuConfig, input: &HookInput, command: &str) {
    if !config.is_configured() {
        return;
    }
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(_) => return,
    };
    rt.block_on(send_autoyes_notification_async(config, input, command));
}

async fn send_autoyes_notification_async(
    config: &FeishuConfig,
    input: &HookInput,
    command: &str,
) {
    let token = match get_token(&config.app_id, &config.app_secret).await {
        Ok(t) => t,
        Err(_) => return,
    };

    let cwd = input.cwd.as_deref().unwrap_or("");
    let project = std::path::Path::new(cwd)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let branch = std::process::Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(cwd)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let sid = input.session_id.as_deref().unwrap_or("-");
    let branch_display = if branch.is_empty() { "-".to_string() } else { branch.clone() };
    let title = if branch.is_empty() {
        format!("{} — ✅ 自动允许", project)
    } else {
        format!("{} ({}) — ✅ 自动允许", project, branch)
    };

    let card = serde_json::json!({
        "config": {"update_multi": true},
        "header": {
            "title": {"tag": "plain_text", "content": title},
            "template": "green"
        },
        "elements": [
            {"tag": "hr"},
            {"tag": "div", "fields": [
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**工具**\n{}", input.tool_name)}},
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**命令**\n{}", command)}}
            ]},
            {"tag": "div", "fields": [
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**Session**\n{}", sid)}},
                {"is_short": true, "text": {"tag": "lark_md", "content": format!("**分支**\n{}", branch_display)}}
            ]},
            {"tag": "hr"},
            {"tag": "note", "elements": [
                {"tag": "plain_text", "content": format!("🕐 {}  ·  自动允许", now)}
            ]}
        ]
    });
    let body = serde_json::to_string(&serde_json::json!({
        "receive_id": config.chat_id, "msg_type": "interactive",
        "content": serde_json::to_string(&card).unwrap()
    })).unwrap();

    let _ = send_msg(&token, &body).await;
}
```

- [ ] **Step 2: 编译验证**

```bash
cargo build 2>&1 | head -10
```

Expected: 编译通过

- [ ] **Step 3: Commit**

```bash
git add src/feishu.rs
git commit -m "feat(autoyes): send_autoyes_notification feishu card"
```

---

### Task 5: Hook — `run_hook()` 前置 autoyes 检查

**Files:**
- Modify: `src/hook.rs:14-38` (run_hook function, add autoyes check after config load)

- [ ] **Step 1: 在 `run_hook()` 中插入 autoyes 检查**

在 `run_hook()` 函数中，加载配置之后（当前 `if config.is_empty()` 检查之后）、提取维度之前，新增 autoyes 检查：

```rust
// After: let Ok((config, local_path)) = settings::load_merged(&cwd) else { ... };
// After: if config.is_empty() { return Ok(()); }

// Autoyes: if enabled, approve everything
if config.autoyes == Some(true) {
    let command_str = match input.tool_name.as_str() {
        "Bash" => input.tool_input.get("command").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        "Write" | "Edit" | "NotebookEdit" => input.tool_input.get("file_path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        "WebFetch" => input.tool_input.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        _ => format!("{:?}", input.tool_input),
    };

    log::log_decision(&input.tool_name, &command_str, "allow", "autoyes enabled");

    // Send feishu notification if configured
    if let Some(ref feishu_config) = config.feishu {
        if feishu_config.is_configured() {
            feishu::send_autoyes_notification(feishu_config, &input, &command_str);
        }
    }

    let output = HookSpecificOutput {
        hook_event_name: "PreToolUse".to_string(),
        permission_decision: "allow".to_string(),
        permission_decision_reason: "Auto-allowed (autoyes enabled)".to_string(),
    };
    let wrapper = serde_json::json!({
        "hookSpecificOutput": output,
    });
    println!("{}", serde_json::to_string(&wrapper).unwrap());
    return Ok(());
}

// ... rest of existing code (extract items → match yes rules → feishu → delegate)
```

- [ ] **Step 2: 编译验证**

```bash
cargo build 2>&1 | head -10
```

Expected: 编译通过

- [ ] **Step 3: Commit**

```bash
git add src/hook.rs
git commit -m "feat(autoyes): run_hook autoyes early-exit approve path"
```

---

### Task 6: CLI — `autoyes enable/disable/status` 子命令

**Files:**
- Modify: `src/main.rs` (add Autoyes enum variant + CLI handling)

- [ ] **Step 1: 新增 CLI 子命令枚举**

在 `src/main.rs` 的 `HookCommand` 枚举后新增：

```rust
#[derive(Subcommand)]
enum AutoyesCommand {
    /// Enable autoyes (auto-approve all permission requests)
    Enable {
        /// Scope: project (default) or global
        #[arg(long, default_value = "project")]
        scope: String,
    },
    /// Disable autoyes
    Disable {
        /// Scope: project (default) or global
        #[arg(long, default_value = "project")]
        scope: String,
    },
    /// Show autoyes status across all config layers
    Status,
}
```

在 `Commands` 枚举中新增：

```rust
/// Manage autoyes settings
#[command(subcommand)]
Autoyes(AutoyesCommand),
```

- [ ] **Step 2: 实现 `autoyes enable`**

在 `main()` 的 match 中，`Commands::List` 分支后新增：

```rust
Commands::Autoyes(cmd) => match cmd {
    AutoyesCommand::Enable { scope } => {
        match scope.as_str() {
            "global" => {
                let home = std::env::var("HOME")
                    .map_err(|_| "$HOME not set".to_string())?;
                let global_path = PathBuf::from(&home).join(".claude").join("settings.json");
                let mut yes = config::YesConfig::default();
                yes.autoyes = Some(true);
                settings::write_to_local(&global_path, &yes)?;
                println!("Enabled autoyes globally ({})", global_path.display());
            }
            "project" => {
                let (_, local_path) = settings::load_merged(&cwd)?;
                if !local_path.parent().unwrap().exists() {
                    return Err("No project config found. Create .claude/ in your project root first.".to_string());
                }
                let mut yes = config::YesConfig::default();
                yes.autoyes = Some(true);
                settings::write_to_local(&local_path, &yes)?;
                println!("Enabled autoyes for project ({})", local_path.display());
            }
            _ => return Err(format!("Unknown scope: {}. Use: project, global", scope)),
        }
    }
    // ... (continue in next step)
```

- [ ] **Step 3: 实现 `autoyes disable` 和 `autoyes status`**

接续上面的 match：

```rust
    AutoyesCommand::Disable { scope } => {
        match scope.as_str() {
            "global" => {
                let home = std::env::var("HOME")
                    .map_err(|_| "$HOME not set".to_string())?;
                let global_path = PathBuf::from(&home).join(".claude").join("settings.json");
                let mut yes = config::YesConfig::default();
                yes.autoyes = Some(false);
                settings::write_to_local(&global_path, &yes)?;
                println!("Disabled autoyes globally ({})", global_path.display());
            }
            "project" => {
                let (_, local_path) = settings::load_merged(&cwd)?;
                let mut yes = config::YesConfig::default();
                yes.autoyes = Some(false);
                settings::write_to_local(&local_path, &yes)?;
                println!("Disabled autoyes for project ({})", local_path.display());
            }
            _ => return Err(format!("Unknown scope: {}. Use: project, global", scope)),
        }
    }
    AutoyesCommand::Status => {
        let home = std::env::var("HOME").unwrap_or_default();
        let global_path = PathBuf::from(&home).join(".claude").join("settings.json");
        let project_path = cwd.join(".claude").join("settings.json");
        let local_path = cwd.join(".claude").join("settings.local.json");

        fn read_autoyes_label(path: &std::path::Path) -> String {
            match std::fs::read_to_string(path) {
                Ok(content) => match serde_json::from_str::<config::SettingsFile>(&content) {
                    Ok(s) => match s.yes.and_then(|y| y.autoyes) {
                        Some(true) => "enabled".to_string(),
                        Some(false) => "disabled".to_string(),
                        None => "(not set)".to_string(),
                    },
                    Err(_) => "(parse error)".to_string(),
                },
                Err(_) => "(not found)".to_string(),
            }
        }

        let global_val = read_autoyes_label(&global_path);
        let project_val = read_autoyes_label(&project_path);
        let local_val = read_autoyes_label(&local_path);

        // Compute effective value (highest priority layer with Some(_))
        let effective = read_autoyes_label(&local_path);
        let effective_final = if effective != "(not found)" && effective != "(not set)" {
            effective.clone()
        } else {
            let project_effective = read_autoyes_label(&project_path);
            if project_effective != "(not found)" && project_effective != "(not set)" {
                project_effective
            } else {
                let global_effective = read_autoyes_label(&global_path);
                if global_effective != "(not found)" && global_effective != "(not set)" {
                    global_effective
                } else {
                    "(not set)".to_string()
                }
            }
        };

        let pad1 = 40usize.saturating_sub("Global (~/.claude/settings.json):".len());
        let pad2 = 40usize.saturating_sub("Project (.claude/settings.json):".len());
        let pad3 = 40usize.saturating_sub("Local   (.claude/settings.local.json):".len());
        println!("Global (~/.claude/settings.json):      {}{}", " ".repeat(pad1.saturating_sub(6)), global_val);
        println!("Project (.claude/settings.json):       {}{}", " ".repeat(pad2.saturating_sub(6)), project_val);
        println!("Local   (.claude/settings.local.json): {}{}", " ".repeat(pad3.saturating_sub(6)), local_val);
        println!("→ Result: {}", effective_final);
    }
},
```

- [ ] **Step 4: 编译验证**

```bash
cargo build 2>&1 | head -20
```

Expected: 编译通过

- [ ] **Step 5: 手动验证 CLI**

```bash
cargo build && ./target/debug/cc-yes autoyes --help
./target/debug/cc-yes autoyes status
```

Expected: 显示 help 和当前状态

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "feat(autoyes): CLI enable/disable/status subcommands"
```

---

### Task 7: 集成测试 — autoyes hook 路径

**Files:**
- Modify: `tests/integration.rs` (add new tests at end)

- [ ] **Step 1: 添加 autoyes 集成测试**

```rust
#[test]
fn test_hook_autoyes_approves_unknown_command() {
    let tmp = std::env::temp_dir().join("cc-yes-integration-autoyes");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join(".claude")).unwrap();

    // autoyes=true, no cmd rules
    let settings = r#"{"yes":{"autoyes":true}}"#;
    std::fs::write(tmp.join(".claude").join("settings.local.json"), settings).unwrap();
    std::env::set_var("HOME", tmp.to_str().unwrap());

    let hook_input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {
            "command": "rm -rf /",
            "description": "Dangerous command"
        },
        "session_id": "test-autoyes-1",
        "cwd": tmp.to_str().unwrap()
    });

    let mut child = Command::new(binary_path())
        .arg("hook")
        .arg("pretooluse")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .current_dir(&tmp)
        .spawn()
        .unwrap();

    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(hook_input.to_string().as_bytes()).unwrap();
    }

    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();

    assert_eq!(
        result["hookSpecificOutput"]["permissionDecision"], "allow",
        "autoyes should approve any command"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_hook_autoyes_false_delegates() {
    let tmp = std::env::temp_dir().join("cc-yes-integration-autoyes-false");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join(".claude")).unwrap();

    // autoyes=false, no cmd rules
    let settings = r#"{"yes":{"autoyes":false}}"#;
    std::fs::write(tmp.join(".claude").join("settings.local.json"), settings).unwrap();
    std::env::set_var("HOME", tmp.to_str().unwrap());

    let hook_input = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": {
            "command": "rm -rf /",
            "description": "Dangerous command"
        },
        "session_id": "test-autoyes-false-1",
        "cwd": tmp.to_str().unwrap()
    });

    let mut child = Command::new(binary_path())
        .arg("hook")
        .arg("pretooluse")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .current_dir(&tmp)
        .spawn()
        .unwrap();

    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(hook_input.to_string().as_bytes()).unwrap();
    }

    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    // autoyes=false with no matching rules → delegate (silent exit)
    assert!(
        stdout.trim().is_empty(),
        "autoyes=false should fall through to normal delegate flow. Got: {}",
        stdout.trim()
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
```

- [ ] **Step 2: 运行集成测试**

```bash
cargo test --test integration -- --nocapture
```

Expected: 所有集成测试 PASS（包括原有 4 个 + 新增 2 个 = 6 个）

- [ ] **Step 3: Commit**

```bash
git add tests/integration.rs
git commit -m "test(autoyes): integration tests for autoyes approve/delegate"
```

---

### Task 8: 文档 — 更新 CLAUDE.md

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: 在 CLAUDE.md 中新增 autoyes 配置说明**

在 Config shape 代码块后（或适当位置）新增：

```markdown
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
```

- [ ] **Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md with autoyes config and CLI"
```

---

### Task 9: 全量测试 + 清理

- [ ] **Step 1: 运行全量测试**

```bash
cargo test 2>&1
```

Expected: 全部测试 PASS（原有 19 + 新增 5 = 24）

- [ ] **Step 2: 运行 clippy**

```bash
cargo clippy 2>&1 | head -30
```

Expected: 无 warning 或 warning 可忽略（autoyes 相关字段必须无 warning）

- [ ] **Step 3: 如有问题，修复后提交**

```bash
# Fix any clippy warnings
git add -A
git commit -m "fix: address clippy warnings for autoyes feature"
```

- [ ] **Step 4: 最终确认**

```bash
cargo build --release 2>&1 | tail -5
cargo test 2>&1 | tail -10
```

---

## 完成标准

- [ ] 全部测试 PASS
- [ ] `cargo clippy` 无新增 warning
- [ ] `cc-yes autoyes --help` 显示正确的帮助信息
- [ ] autoyes 开启时任意命令被自动允许
- [ ] autoyes 关闭时走原有规则匹配/飞书审批流程
- [ ] 3 层配置覆盖正确工作
- [ ] CLAUDE.md 文档已更新
