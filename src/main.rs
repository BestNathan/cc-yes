mod after;
mod config;
mod hook;
mod matcher;
mod parser;
mod settings;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "cc-yes", version = "0.1.0", about = "Auto-approve Claude Code tool-use permissions")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install the cc-yes binary
    Install {
        #[arg(long, group = "install_source")]
        bin: bool,
        #[arg(long, group = "install_source")]
        source: bool,
        /// GitHub repository override (default: user/cc-yes)
        #[arg(long, default_value = "")]
        repo: String,
    },
    /// Add a rule to settings.local.json
    Add {
        /// Dimension: cmd, files, url, imports, env
        dimension: String,
        /// Rule pattern to add (e.g., "git", "cargo build", "*.rs")
        rule: String,
    },
    /// Remove a rule from settings.local.json
    Remove {
        /// Dimension: cmd, files, url, imports, env
        dimension: String,
        /// Rule pattern to remove
        rule: String,
    },
    /// Show merged yes configuration
    List {
        /// Optional: filter by dimension (cmd, files, url, imports, env)
        dimension: Option<String>,
    },
    /// Dry-run: check a command against current rules
    Check {
        /// The bash command to check
        command: String,
    },
    /// Internal: handle PreToolUse hook (reads stdin)
    Hook,
    /// Internal: handle PostToolUse auto-learn (reads stdin)
    After,
}

fn main() -> Result<(), String> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir().map_err(|e| format!("cwd error: {}", e))?;

    match cli.command {
        Commands::Install { bin, source, repo } => {
            if bin {
                let artifact = detect_platform_artifact()?;
                let repo = if repo.is_empty() {
                    "user/cc-yes".to_string()
                } else {
                    repo.clone()
                };
                let url = format!(
                    "https://github.com/{}/releases/latest/download/{}",
                    repo, artifact
                );
                println!("Downloading {} from {}", artifact, url);

                let response = ureq::get(&url)
                    .call()
                    .map_err(|e| format!("Download failed: {}. Is the binary built and released?", e))?;

                if response.status() != 200 {
                    return Err(format!(
                        "Download failed: HTTP {}. Check that the release exists and includes {}",
                        response.status(),
                        artifact
                    ));
                }

                let plugin_root = std::env::var("CLAUDE_PLUGIN_ROOT")
                    .unwrap_or_else(|_| cwd.to_string_lossy().to_string());
                let bin_dir = PathBuf::from(&plugin_root).join("bin");
                std::fs::create_dir_all(&bin_dir)
                    .map_err(|e| format!("mkdir {}: {}", bin_dir.display(), e))?;

                let bin_path = bin_dir.join("cc-yes");
                let mut reader = response.into_reader();
                let mut file = std::fs::File::create(&bin_path)
                    .map_err(|e| format!("Cannot create {}: {}", bin_path.display(), e))?;
                std::io::copy(&mut reader, &mut file)
                    .map_err(|e| format!("Failed to write binary: {}", e))?;

                // Make executable on Unix
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = std::fs::metadata(&bin_path)
                        .map_err(|e| format!("Cannot read metadata: {}", e))?
                        .permissions();
                    perms.set_mode(0o755);
                    std::fs::set_permissions(&bin_path, perms)
                        .map_err(|e| format!("Cannot chmod: {}", e))?;
                }

                println!("Installed to {}", bin_path.display());
            } else if source {
                println!("Building from source...");
                let status = std::process::Command::new("cargo")
                    .args(["build", "--release"])
                    .status()
                    .map_err(|e| format!("cargo build failed: {}", e))?;
                if !status.success() {
                    return Err("cargo build --release failed".to_string());
                }
                // Copy to plugin bin directory
                let plugin_root = std::env::var("CLAUDE_PLUGIN_ROOT")
                    .unwrap_or_else(|_| cwd.to_string_lossy().to_string());
                let bin_dir = PathBuf::from(&plugin_root).join("bin");
                std::fs::create_dir_all(&bin_dir)
                    .map_err(|e| format!("mkdir {}: {}", bin_dir.display(), e))?;
                std::fs::copy("target/release/cc-yes", bin_dir.join("cc-yes"))
                    .map_err(|e| format!("copy failed: {}", e))?;
                println!("Installed to {}/cc-yes", bin_dir.display());
            } else {
                println!("Usage: cc-yes install --bin | --source");
            }
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

        Commands::Hook => {
            hook::run_hook()?;
        }

        Commands::After => {
            after::run_after()?;
        }
    }

    Ok(())
}

fn detect_platform_artifact() -> Result<String, String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    match (os, arch) {
        ("macos", "aarch64") => Ok("cc-yes-darwin-arm64".to_string()),
        ("macos", "x86_64") => Ok("cc-yes-darwin-x64".to_string()),
        ("linux", "aarch64") => Ok("cc-yes-linux-arm64".to_string()),
        ("linux", "x86_64") => Ok("cc-yes-linux-x64".to_string()),
        ("windows", "x86_64") => Ok("cc-yes-win-x64.exe".to_string()),
        _ => Err(format!(
            "Unsupported platform: {}-{}. Build from source with: cc-yes install --source",
            os, arch
        )),
    }
}

fn print_list(label: &str, items: &[String]) {
    if !items.is_empty() {
        println!("[{}]", label);
        for item in items {
            println!("  {}", item);
        }
    }
}
