mod after;
mod config;
mod feishu;
mod hook;
mod log;
mod matcher;
mod parser;
mod permission_request;
mod settings;
mod ws;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "cc-yes", version = "0.1.1", about = "Auto-approve Claude Code tool-use permissions")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install the cc-yes binary from source
    Install,
    /// Add a rule to settings.local.json
    Add {
        dimension: String,
        rule: String,
    },
    /// Remove a rule from settings.local.json
    Remove {
        dimension: String,
        rule: String,
    },
    /// Show merged yes configuration
    List {
        dimension: Option<String>,
    },
    /// Dry-run: check a command against current rules
    Check {
        command: String,
    },
    /// Start WebSocket daemon for long-running event/card handling
    Daemon,
    /// Hook handlers (called by Claude Code, read stdin)
    #[command(subcommand)]
    Hook(HookCommand),
}

#[derive(Subcommand)]
enum HookCommand {
    /// PreToolUse: check rules → approve or delegate
    #[command(name = "pretooluse")]
    PreToolUse,
    /// PermissionRequest: send feishu card → wait for approval
    #[command(name = "permission-request")]
    PermissionRequest,
    /// PostToolUse: auto-learn from "Always allow" clicks
    #[command(name = "posttooluse")]
    PostToolUse,
}

fn main() -> Result<(), String> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir().map_err(|e| format!("cwd error: {}", e))?;

    match cli.command {
        Commands::Install => {
            println!("Building from source...");
            let status = std::process::Command::new("cargo")
                .args(["build", "--release"])
                .status()
                .map_err(|e| format!("cargo build failed: {}", e))?;
            if !status.success() {
                return Err("cargo build --release failed".to_string());
            }
            let plugin_root = std::env::var("CLAUDE_PLUGIN_ROOT")
                .unwrap_or_else(|_| cwd.to_string_lossy().to_string());
            let bin_dir = PathBuf::from(&plugin_root).join("bin");
            std::fs::create_dir_all(&bin_dir)
                .map_err(|e| format!("mkdir {}: {}", bin_dir.display(), e))?;
            std::fs::copy("target/release/cc-yes", bin_dir.join("cc-yes"))
                .map_err(|e| format!("copy failed: {}", e))?;
            println!("Installed to {}/cc-yes", bin_dir.display());
        }

        Commands::Add { dimension, rule } => {
            let (_, local_path) = settings::load_merged(&cwd)?;
            let mut to_add = config::YesConfig::default();
            match dimension.as_str() {
                "cmd" => to_add.cmd.push(rule.clone()),
                "files" => to_add.files.push(rule.clone()),
                "url" => to_add.url.push(rule.clone()),
                "imports" => to_add.imports.push(rule.clone()),
                "env" => to_add.env.push(rule.clone()),
                _ => return Err(format!("Unknown dimension: {}. Use: cmd, files, url, imports, env", dimension)),
            }
            settings::write_to_local(&local_path, &to_add)?;
            println!("Added {}: \"{}\" to {}", dimension, rule, local_path.display());
        }

        Commands::Remove { dimension, rule } => {
            let (_, local_path) = settings::load_merged(&cwd)?;
            settings::remove_from_local(&local_path, &dimension, &rule)?;
            println!("Removed {}: \"{}\" from {}", dimension, rule, local_path.display());
        }

        Commands::List { dimension } => {
            let (config, _) = settings::load_merged(&cwd)?;
            match dimension.as_deref() {
                Some("cmd") => print_list("cmd", &config.cmd),
                Some("files") => print_list("files", &config.files),
                Some("url") => print_list("url", &config.url),
                Some("imports") => print_list("imports", &config.imports),
                Some("env") => print_list("env", &config.env),
                Some(d) => return Err(format!("Unknown dimension: {}", d)),
                None => {
                    print_list("cmd", &config.cmd);
                    print_list("files", &config.files);
                    print_list("url", &config.url);
                    print_list("imports", &config.imports);
                    print_list("env", &config.env);
                }
            }
        }

        Commands::Check { command } => {
            let (config, _) = settings::load_merged(&cwd)?;
            let extracted = parser::parse_bash(&command, &cwd);
            if extracted.is_empty() {
                println!("→ Cannot parse — would DELEGATE");
                return Ok(());
            }
            for cmd in &extracted.cmd {
                let ok = matcher::match_single(cmd, &config.cmd);
                println!("cmd: {} → {}", cmd, if ok { "✅" } else { "❌" });
            }
            for file in &extracted.files {
                let ok = matcher::match_single(file, &config.files);
                println!("files: {} → {}", file, if ok { "✅" } else { "❌" });
            }
            for url in &extracted.url {
                let ok = matcher::match_single(url, &config.url);
                println!("url: {} → {}", url, if ok { "✅" } else { "❌" });
            }
            for import in &extracted.imports {
                let ok = matcher::match_single(import, &config.imports);
                println!("imports: {} → {}", import, if ok { "✅" } else { "❌" });
            }
            for env in &extracted.env {
                let ok = matcher::match_single(env, &config.env);
                println!("env: {} → {}", env, if ok { "✅" } else { "❌" });
            }
            if matcher::matches_all(&extracted, &config) {
                println!("→ would AUTO-APPROVE ✅");
            } else {
                println!("→ would NOT auto-approve (delegate to user)");
            }
        }

        Commands::Hook(cmd) => match cmd {
            HookCommand::PreToolUse => hook::run_hook()?,
            HookCommand::PermissionRequest => permission_request::run_permission_request()?,
            HookCommand::PostToolUse => after::run_after()?,
        },

        Commands::Daemon => {
            tracing_subscriber::fmt::init();
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| format!("tokio runtime: {}", e))?;
            rt.block_on(async {
                let registry = std::sync::Arc::new(
                    crate::ws::HandlerRegistry::new(64)
                );
                // Register built-in handlers
                registry.register(crate::ws::EventHandler::new(|event| {
                    tracing::info!("event received: {:?}", event);
                    None
                })).await;

                let config = crate::ws::WsConfig {
                    app_id: std::env::var("FEISHU_APP_ID")
                        .map_err(|_| "FEISHU_APP_ID not set".to_string())?,
                    app_secret: std::env::var("FEISHU_APP_SECRET")
                        .map_err(|_| "FEISHU_APP_SECRET not set".to_string())?,
                    domain: "https://open.feishu.cn".into(),
                    registry,
                };

                let client = crate::ws::WsClient::new(config);
                client.start().await.map_err(|e| format!("ws error: {}", e))
            })?;
        }
    }

    Ok(())
}

fn print_list(label: &str, items: &[String]) {
    if !items.is_empty() {
        println!("[{}]", label);
        for item in items {
            println!("  {}", item);
        }
    }
}
