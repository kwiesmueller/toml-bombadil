use anyhow::Result;
use clap::builder::PossibleValuesParser;
use clap::CommandFactory;
use clap::Parser;
use clap_complete::Shell;
use colored::Colorize;
use std::io;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use toml_bombadil::conflict::ConflictStrategy;
use toml_bombadil::packages::discover::discover_package;
use toml_bombadil::packages::persist::{
    append_package_to_file, default_packages_file, interactive_add_package,
};
use toml_bombadil::packages::state::PackagesState;
use toml_bombadil::packages::{InstallStatus, PackageManager, RemoveStatus};
use toml_bombadil::platform::PlatformContext;
use toml_bombadil::settings::Settings;
use toml_bombadil::validate;
use toml_bombadil::{Bombadil, MetadataType, Mode};

// v4 imports
use toml_bombadil::audit::{
    format_action_details, format_file_history, format_revert_plan, format_session_log,
    format_traces, generate_diff_from_storage, print_legend, ActionId, AuditStorage,
    FileRevertOptions, RevertEngine,
};
use toml_bombadil::config::{
    generate_patch_schema, generate_schema, generate_semantic_patch_schema,
};
use toml_bombadil::packages::drift::DriftReport;
use toml_bombadil::packages::managers::{
    apt::Apt, brew::Brew, cargo::Cargo, dnf::Dnf, flatpak::Flatpak, pacman::Pacman,
    PackageManager as PackageManagerTrait,
};

macro_rules! fatal {
    ($($tt:tt)*) => {{
        use std::io::Write;
        writeln!(&mut ::std::io::stderr(), $($tt)*).unwrap();
        ::std::process::exit(1)
    }}
}

fn parse_strategy(s: &str) -> Result<ConflictStrategy, String> {
    s.parse()
}

/// Load profile names, trying v4 config first and falling back to v3.
///
/// Used as the `value_parser` for clap profile arguments so that CLI
/// tab-completion and validation reflect the currently configured profiles.
/// Returns a `PossibleValuesParser` that clap uses for completion and validation.
fn load_profile_names() -> PossibleValuesParser {
    let names: Vec<&'static str> = match toml_bombadil::config::load_config() {
        Ok(config) => config
            .profiles
            .keys()
            .map(|s| &*Box::leak(s.clone().into_boxed_str()))
            .collect(),
        Err(_) => vec![],
    };
    PossibleValuesParser::new(names)
}

/// Get dotfiles path from v4 config, falling back to v3 Settings.
fn get_dotfiles_path() -> Result<PathBuf> {
    let config_path = toml_bombadil::config::config_path()
        .map_err(|e| anyhow::anyhow!("Failed to find config: {}", e))?;
    let config = toml_bombadil::config::load_config_resolved(&config_path)
        .map_err(|e| anyhow::anyhow!("Failed to load config: {}", e))?;
    Ok(toml_bombadil::config::resolve_dotfiles_dir(
        &config,
        &config_path,
    ))
}

/// Toml is a dotfile template manager, written in rust.
#[derive(Parser)]
#[command(
    version,
    name = "Toml Bombadil",
    author = "Paul D. <paul.delafosse@protonmail.com>"
)]
enum Cli {
    /// Link a given dotfile directory settings to "XDG_CONFIG_DIR/bombadil.toml"
    Install {
        /// Path to your dotfile directory
        #[clap(value_name = "CONFIG", required = false)]
        config: Option<PathBuf>,
    },
    /// Symlink a copy of your dotfiles and inject variables according to bombadil.toml settings
    Link {
        /// A list of comma separated profiles to activate
        #[clap(short, long, required = false, value_parser = load_profile_names(), num_args(0..))]
        profiles: Vec<String>,

        /// Force overwrite conflicts (dotfile always wins, system files backed up)
        #[clap(short, long)]
        force: bool,

        /// Conflict resolution strategy: interactive (default), dotfile-wins, system-wins, skip
        #[clap(long, value_parser = parse_strategy)]
        strategy: Option<ConflictStrategy>,

        /// Show what would be done without making changes
        #[clap(long)]
        dry_run: bool,

        /// Review and confirm each change interactively
        #[clap(long)]
        review: bool,
    },
    /// Remove all symlinks defined in your bombadil.toml
    Unlink,
    /// Watch dotfiles and automatically run link on changes
    Watch {
        /// A list of comma separated profiles to activate
        #[clap(short, long, required = false, value_parser = load_profile_names(), num_args(0..))]
        profiles: Vec<String>,
    },
    /// Add a secret var to bombadil environment
    AddSecret {
        /// Key of the secret variable to create
        #[clap(short, long)]
        key: String,
        #[clap(short, long, required_unless_present = "ask")]
        value: String,
        /// Get the secret value from stdin
        #[clap(long, short)]
        ask: bool,
        /// Path of the var file to modify
        #[clap(long, short)]
        file: String,
    },
    /// Get metadata about dots, hooks, path, profiles, vars, or platform
    Get {
        #[clap(value_name = "VALUE", value_parser = ["dots", "prehooks", "posthooks", "path", "profiles", "vars", "secrets", "platform"])]
        value: String,
        #[clap(value_parser = load_profile_names(), num_args(0..))]
        profiles: Vec<String>,
    },
    /// Generate shell completions
    GenerateCompletions {
        /// Type of completions to generate
        #[clap(name = "type", value_enum)]
        shell: Shell,
    },
    /// Scan dotfiles for potential unencrypted secrets and security issues
    Validate {
        /// Exit with non-zero status code if any issues are found
        #[clap(long)]
        strict: bool,

        /// Path to the dotfiles directory (defaults to configured path)
        #[clap(short, long)]
        path: Option<PathBuf>,
    },
    /// Apply full desired state: dotfiles + packages (new sync engine)
    ///
    /// Replaces `link` and `packages sync`. Reads the active profile from
    /// `.active_profile` in the dotfiles directory (written by `bombadil init`).
    Sync {
        /// Show plan without executing
        #[clap(long)]
        dry_run: bool,

        /// Override active tags (comma-separated; supplement profile tags)
        #[clap(short, long, value_delimiter = ',')]
        tags: Vec<String>,

        /// Skip package operations
        #[clap(long)]
        only_dots: bool,

        /// Skip dot operations
        #[clap(long)]
        only_packages: bool,

        /// Remove packages installed but not in config
        #[clap(long)]
        prune_packages: bool,
    },
    /// Manage software packages
    Packages {
        #[clap(subcommand)]
        command: PackagesCommand,
    },
    /// Generate JSON Schema for configuration files
    Schema {
        /// Type of schema to generate
        #[clap(value_enum, default_value = "config")]
        schema_type: SchemaType,

        /// Output file (default: stdout)
        #[clap(short, long)]
        output: Option<PathBuf>,
    },
    /// Detect drift between configured and installed packages
    Drift {
        /// Show extra packages (installed but not configured)
        #[clap(long)]
        show_extra: bool,

        /// A list of profiles to activate
        #[clap(short, long, value_parser = load_profile_names(), num_args(0..))]
        profiles: Vec<String>,
    },
    /// Show action history from past sessions
    Log {
        /// Maximum number of sessions to show
        #[clap(short = 'n', long, default_value = "10")]
        limit: usize,

        /// Filter by file path
        #[clap(short, long)]
        file: Option<PathBuf>,

        /// Filter by dot name
        #[clap(short, long)]
        dot: Option<String>,

        /// Show detailed action list for each session
        #[clap(short, long)]
        verbose: bool,
    },
    /// Inspect a specific action by its ID, or all actions for a file
    Inspect {
        /// Action ID to inspect (e.g., "abcde")
        #[clap(value_name = "ACTION_ID", required_unless_present = "file")]
        action_id: Option<String>,

        /// What to show: details (default), diff, or content
        #[clap(value_enum, default_value = "details")]
        view: InspectView,

        /// Inspect all actions for a file (shows file history)
        #[clap(long, short)]
        file: Option<PathBuf>,

        /// Show diff for the action (or in file history view)
        #[clap(long)]
        diff: bool,

        /// Show captured logs
        #[clap(long)]
        log: bool,
    },
    /// Revert a specific action by its ID, or all actions for a file
    Revert {
        /// Action ID to revert (e.g., "abcde")
        #[clap(value_name = "ACTION_ID", required_unless_present = "file")]
        action_id: Option<String>,

        /// Force revert even if file was modified after the action
        #[clap(short, long)]
        force: bool,

        /// Show what would be done without making changes
        #[clap(long)]
        dry_run: bool,

        /// Revert all actions on a file
        #[clap(long, short = 'F')]
        file: Option<PathBuf>,

        /// Revert to a specific action's state (use with --file)
        #[clap(long, requires = "file")]
        to: Option<String>,

        /// Continue reverting even if one action fails
        #[clap(long)]
        continue_on_error: bool,
    },
    /// Migrate v3 bombadil.toml imports to v4 dots.toml format
    ///
    /// Reads bombadil.toml from <dotfiles-dir> and generates a dots.toml for
    /// each imported config file. Files in <dotfiles-dir> are never modified.
    /// Generated dots.toml files are written to <output-dir> (default: same
    /// as <dotfiles-dir>) maintaining the same directory structure.
    /// Existing dots.toml files in the output are never overwritten.
    Migrate {
        /// Path to the dotfiles directory containing bombadil.toml
        #[clap(value_name = "DOTFILES_DIR")]
        dotfiles_dir: PathBuf,

        /// Directory to write generated dots.toml files into
        /// (defaults to <dotfiles-dir>; files in <dotfiles-dir> are never modified)
        #[clap(short, long)]
        output: Option<PathBuf>,

        /// Print what would be written without creating any files
        #[clap(long)]
        dry_run: bool,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum InspectView {
    /// Show action details
    Details,
    /// Show the diff for the action
    Diff,
    /// Show before/after content
    Content,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum SchemaType {
    /// Main bombadil.toml configuration schema
    Config,
    /// Line-based patch file schema
    Patch,
    /// Semantic patch file schema (for JSON/YAML/TOML/INI)
    SemanticPatch,
}

#[derive(Parser)]
enum PackagesCommand {
    /// Install packages (discovers and persists unknown packages automatically)
    Install {
        /// Package name to install (if omitted, installs all configured packages)
        #[clap(value_name = "PACKAGE")]
        package: Option<String>,

        /// Filter by tags (comma-separated)
        #[clap(short, long, value_delimiter = ',')]
        tags: Vec<String>,

        /// Exclude packages with these tags (comma-separated)
        #[clap(short, long, value_delimiter = ',')]
        exclude: Vec<String>,

        /// Force specific installation method
        #[clap(short, long, value_parser = ["dnf", "apt", "brew", "pacman", "cargo", "go", "flatpak", "binary", "git", "source"])]
        method: Option<String>,

        /// Open editor for interactive package configuration
        #[clap(short, long)]
        interactive: bool,

        /// Target file for new package definitions
        #[clap(short, long)]
        file: Option<PathBuf>,

        /// A list of profiles to activate
        #[clap(short, long, value_parser = load_profile_names(), num_args(0..))]
        profiles: Vec<String>,
    },
    /// List configured packages
    List {
        /// Show only installed packages
        #[clap(long)]
        installed: bool,

        /// Show only packages not yet installed
        #[clap(long)]
        available: bool,

        /// Filter by tags (comma-separated)
        #[clap(short, long, value_delimiter = ',')]
        tags: Vec<String>,

        /// A list of profiles to activate
        #[clap(short, long, value_parser = load_profile_names(), num_args(0..))]
        profiles: Vec<String>,
    },
    /// Update packages (check for new versions)
    Update {
        /// Package name to update (if omitted, updates all)
        #[clap(value_name = "PACKAGE")]
        package: Option<String>,

        /// A list of profiles to activate
        #[clap(short, long, value_parser = load_profile_names(), num_args(0..))]
        profiles: Vec<String>,
    },
    /// Remove a package
    Remove {
        /// Package name to remove
        #[clap(value_name = "PACKAGE", required = true)]
        package: String,

        /// Keep package in configuration (only uninstall)
        #[clap(long)]
        keep_config: bool,

        /// Skip confirmation prompt
        #[clap(short, long)]
        yes: bool,

        /// A list of profiles to activate
        #[clap(short, long, value_parser = load_profile_names(), num_args(0..))]
        profiles: Vec<String>,
    },
    /// Sync packages (install missing, optionally prune orphans)
    Sync {
        /// Remove packages that are installed but not in config
        #[clap(long)]
        prune: bool,

        /// Filter by tags (comma-separated)
        #[clap(short, long, value_delimiter = ',')]
        tags: Vec<String>,

        /// A list of profiles to activate
        #[clap(short, long, value_parser = load_profile_names(), num_args(0..))]
        profiles: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli: Cli = Cli::parse();

    match cli {
        Cli::Install { config } => {
            Bombadil::link_self_config(config).unwrap_or_else(|err| fatal!("{}", err));
        }
        Cli::Link {
            profiles,
            force,
            strategy,
            dry_run,
            review,
        } => {
            let mut bombadil = Bombadil::load(Mode::Gpg).unwrap_or_else(|err| fatal!("{}", err));

            bombadil
                .enable_profiles_v4(profiles.iter().map(String::as_str).collect())
                .unwrap_or_else(|err| fatal!("{}", err));

            // Determine conflict strategy
            let conflict_strategy = if force {
                ConflictStrategy::DotfileWins
            } else {
                strategy.unwrap_or(ConflictStrategy::Interactive)
            };

            // Phase 1: Plan the installation
            let plan = bombadil
                .plan_install(conflict_strategy)
                .unwrap_or_else(|err| fatal!("{}", err));

            // Display the plan
            toml_bombadil::audit::print_plan_header(&plan);

            if plan.is_empty() {
                return Ok(());
            }

            // Phase 2: If dry-run, stop here
            if dry_run {
                toml_bombadil::audit::print_legend();
                return Ok(());
            }

            // Phase 3: Execute the plan
            let session = bombadil
                .execute_install_with_options(plan, profiles.clone(), conflict_strategy, review)
                .unwrap_or_else(|err| fatal!("{}", err));

            // Display session summary
            toml_bombadil::audit::print_session_summary(&session);
        }
        Cli::Sync {
            dry_run,
            tags,
            only_dots,
            only_packages,
            prune_packages,
        } => {
            use toml_bombadil::sync::{SyncEngine, SyncOptions};

            let config_path =
                toml_bombadil::config::config_path().unwrap_or_else(|err| fatal!("{}", err));

            // Read active profile from .active_profile file
            let dotfiles_dir = {
                let config = toml_bombadil::config::load_config_from(&config_path)
                    .unwrap_or_else(|err| fatal!("{}", err));
                toml_bombadil::config::resolve_dotfiles_dir(&config, &config_path)
            };
            let active_profile_file = dotfiles_dir.join(".active_profile");
            let active_profile = if active_profile_file.exists() {
                std::fs::read_to_string(&active_profile_file)
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
            } else {
                None
            };

            let options = SyncOptions {
                profile: active_profile,
                dry_run,
                extra_tags: tags,
                only_dots,
                only_packages,
                prune_packages,
            };

            let engine = SyncEngine::new(&config_path);
            let plan = engine
                .plan(&options)
                .unwrap_or_else(|err| fatal!("{}", err));

            if dry_run {
                // Print plan summary
                println!("Sync plan ({} items):", plan.items.len());
                for item in &plan.items {
                    match item {
                        toml_bombadil::sync::SyncItem::Hook(h) => {
                            let owner = h.owner.as_deref().unwrap_or("global");
                            println!("  [{:?}] hook ({owner}): {}", h.phase, h.command);
                        }
                        toml_bombadil::sync::SyncItem::Dot(d) => {
                            println!(
                                "  {} //{}: {} files",
                                d.action.indicator(),
                                d.namespace,
                                d.files.len()
                            );
                        }
                        toml_bombadil::sync::SyncItem::Package(p) => {
                            println!("  pkg: {}", p.name);
                        }
                    }
                }
            } else {
                engine.execute(plan).unwrap_or_else(|err| fatal!("{}", err));
            }
        }
        Cli::Watch { profiles } => {
            Bombadil::watch(profiles).await?;
        }
        Cli::Unlink => {
            Bombadil::load(Mode::NoGpg)
                .and_then(|bombadil| bombadil.uninstall())
                .unwrap_or_else(|err| fatal!("{}", err));
        }
        Cli::AddSecret {
            key,
            value,
            ask,
            file,
        } => {
            let value = if ask {
                println!("Type the value and press enter to confirm :");
                std::io::stdin().lock().lines().next().unwrap().unwrap()
            } else {
                value
            };

            let var_file = file;
            let path = Path::new(&var_file);

            if !path.exists() {
                fatal!(
                    "Error trying to write secret to {} : No such file",
                    var_file
                )
            };

            if path.is_dir() {
                fatal!(
                    "Error trying to write secret to {} : is a directory",
                    var_file
                )
            }

            Bombadil::load(Mode::Gpg)
                .and_then(|bombadil| bombadil.add_secret(&key, &value, &var_file))
                .unwrap_or_else(|err| fatal!("{}", err));
        }
        Cli::Get { value, profiles } => {
            // Handle platform specially since it doesn't need bombadil settings
            if value == "platform" {
                let platform = PlatformContext::detect();
                println!("os: {}", platform.os);
                if let Some(distro) = &platform.distro {
                    println!("distro: {}", distro);
                }
                if let Some(version) = &platform.distro_version {
                    println!("distro_version: {}", version);
                }
                println!("arch: {}", platform.arch);
                println!("hostname: {}", platform.hostname);
                println!("username: {}", platform.username);
                println!("home: {}", platform.home);
                return Ok(());
            }

            let metadata_type = match value.as_str() {
                "dots" => MetadataType::Dots,
                "prehooks" => MetadataType::PreHooks,
                "posthooks" => MetadataType::PostHooks,
                "path" => MetadataType::Path,
                "profiles" => MetadataType::Profiles,
                "vars" => MetadataType::Vars,
                "secrets" => MetadataType::Secrets,
                _ => unreachable!(),
            };

            let mut bombadil = match metadata_type {
                MetadataType::Secrets => Bombadil::load(Mode::Gpg),
                _ => Bombadil::load(Mode::NoGpg),
            }
            .unwrap_or_else(|err| fatal!("{}", err));

            bombadil
                .enable_profiles_v4(profiles.iter().map(String::as_str).collect())
                .unwrap_or_else(|err| fatal!("{}", err));

            bombadil
                .print_metadata(metadata_type, &mut io::stdout())
                .expect("Failed to write metadata to stdout");
        }
        Cli::GenerateCompletions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "bombadil", &mut io::stdout())
        }
        Cli::Validate { strict, path } => {
            // Get the dotfiles path - either from argument or from settings
            let dotfiles_path = match path {
                Some(p) => p,
                None => get_dotfiles_path().unwrap_or_else(|err| fatal!("{}", err)),
            };

            let report =
                validate::scan_directory(&dotfiles_path).unwrap_or_else(|err| fatal!("{}", err));

            report.print();

            if strict && report.has_issues() {
                std::process::exit(1);
            }
        }
        Cli::Packages { command } => {
            handle_packages_command(command).unwrap_or_else(|err| fatal!("{}", err));
        }
        Cli::Schema {
            schema_type,
            output,
        } => {
            let schema = match schema_type {
                SchemaType::Config => generate_schema(),
                SchemaType::Patch => generate_patch_schema(),
                SchemaType::SemanticPatch => generate_semantic_patch_schema(),
            };

            let json = serde_json::to_string_pretty(&schema)
                .unwrap_or_else(|err| fatal!("Failed to serialize schema: {}", err));

            match output {
                Some(path) => {
                    std::fs::write(&path, &json).unwrap_or_else(|err| {
                        fatal!("Failed to write schema to {}: {}", path.display(), err)
                    });
                    println!("{} Schema written to {}", "✓".green(), path.display());
                }
                None => {
                    println!("{}", json);
                }
            }
        }
        Cli::Drift {
            show_extra,
            profiles: _profiles,
        } => {
            // Load v4 config
            let config = toml_bombadil::config::load_config()
                .unwrap_or_else(|err| fatal!("Failed to load config: {}", err));

            // Create available package managers
            let managers: Vec<Box<dyn PackageManagerTrait>> = vec![
                Box::new(Dnf),
                Box::new(Apt),
                Box::new(Brew),
                Box::new(Pacman),
                Box::new(Cargo),
                Box::new(Flatpak),
            ];

            let report = if show_extra {
                DriftReport::detect_with_extras(&config, &managers)
                    .unwrap_or_else(|err| fatal!("Drift detection failed: {}", err))
            } else {
                DriftReport::detect(&config, &managers)
                    .unwrap_or_else(|err| fatal!("Drift detection failed: {}", err))
            };

            report.print();

            // Exit with non-zero if drift detected
            if !report.is_clean() {
                std::process::exit(1);
            }
        }
        Cli::Log {
            limit,
            file,
            dot,
            verbose,
        } => {
            let dotfiles_path = get_dotfiles_path().unwrap_or_else(|err| fatal!("{}", err));
            let storage = AuditStorage::new(&dotfiles_path);

            let sessions = storage
                .list_sessions()
                .unwrap_or_else(|err| fatal!("Failed to read sessions: {}", err));

            if sessions.is_empty() {
                println!("{}", "No action history found.".yellow());
                println!("Run '{}' to create actions.", "bombadil link".cyan());
                return Ok(());
            }

            println!("{}", "Action History:".bold());
            println!();

            // Collect matching sessions up to limit
            let mut matching_sessions = Vec::new();
            for session in sessions {
                // Filter by file if specified
                if let Some(ref file_filter) = file {
                    let has_matching_action = session.actions.iter().any(|a| {
                        a.action_type
                            .target_path()
                            .map(|p| p == file_filter)
                            .unwrap_or(false)
                    });
                    if !has_matching_action {
                        continue;
                    }
                }

                // Filter by dot if specified
                if let Some(ref dot_filter) = dot {
                    let has_matching_action = session
                        .actions
                        .iter()
                        .any(|a| a.dot_name.as_ref() == Some(dot_filter));
                    if !has_matching_action {
                        continue;
                    }
                }

                matching_sessions.push(session);

                if matching_sessions.len() >= limit {
                    break;
                }
            }

            let count = matching_sessions.len();

            // Display oldest first so newest is at the bottom (closest to cursor)
            for session in matching_sessions.into_iter().rev() {
                print!("{}", format_session_log(&session, verbose));
            }

            if count == 0 {
                println!("{}", "No matching sessions found.".yellow());
            }

            print_legend();
        }
        Cli::Inspect {
            action_id,
            view,
            file,
            diff: show_diff,
            log: show_log,
        } => {
            let dotfiles_path = get_dotfiles_path().unwrap_or_else(|err| fatal!("{}", err));
            let storage = AuditStorage::new(&dotfiles_path);

            // File history mode
            if let Some(file_path) = file {
                let file_index = storage
                    .load_file_index()
                    .unwrap_or_else(|err| fatal!("Failed to load file index: {}", err));

                let file_actions = match file_index.get(&file_path) {
                    Some(actions) => actions.clone(),
                    None => {
                        println!("{}", "No actions found for this file.".yellow());
                        return Ok(());
                    }
                };

                // Build the enriched action list with full Action and Session data
                let mut enriched_actions = Vec::new();
                for file_action in &file_actions {
                    let session = storage
                        .find_session_for_action(&file_action.action_id)
                        .ok()
                        .flatten();
                    let action = session
                        .as_ref()
                        .and_then(|s| s.get_action(&file_action.action_id));
                    enriched_actions.push((file_action.clone(), action.cloned(), session));
                }

                let history = format_file_history(
                    &file_path,
                    &enriched_actions
                        .iter()
                        .map(|(fa, a, s)| (fa.clone(), a.as_ref(), s.as_ref()))
                        .collect::<Vec<_>>(),
                    show_diff,
                    Some(&storage),
                );
                println!("{}", history);
                return Ok(());
            }

            // Single action mode
            let action_id_str = action_id.expect("action_id required when --file not specified");
            let action_id = ActionId::from_string(&action_id_str);
            let session = storage
                .find_session_for_action(&action_id)
                .unwrap_or_else(|err| fatal!("Failed to find action: {}", err));

            let session = match session {
                Some(s) => s,
                None => fatal!("Action '{}' not found", action_id),
            };

            let action = session
                .get_action(&action_id)
                .expect("Action should exist in session");

            // Show logs if requested
            if show_log {
                if storage.has_logs(&action_id) {
                    let logs = storage
                        .load_logs(&action_id)
                        .unwrap_or_else(|err| fatal!("Failed to load logs: {}", err));
                    // Try to parse as structured traces first, fall back to raw text
                    match toml_bombadil::audit::deserialize_traces(&logs) {
                        Ok(traces) if !traces.is_empty() => {
                            println!("{}", format_traces(&traces));
                        }
                        _ => {
                            // Raw log format (e.g., hook stdout/stderr)
                            println!("{}", logs);
                        }
                    }
                } else {
                    println!("{}", "No logs captured for this action.".yellow());
                }
                return Ok(());
            }

            // If --diff flag is set, override view to Diff
            let effective_view = if show_diff { InspectView::Diff } else { view };

            match effective_view {
                InspectView::Details => {
                    println!("{}", format_action_details(action));
                }
                InspectView::Diff => match generate_diff_from_storage(&storage, action) {
                    Some(diff) => println!("{}", diff),
                    None => {
                        println!("{}", "No diff available for this action.".yellow());
                        println!("Diff is only available for file update/patch actions.");
                    }
                },
                InspectView::Content => {
                    println!("{}", "Before:".bold());
                    match storage.load_before_content(&action_id) {
                        Ok(content) => {
                            let text = String::from_utf8_lossy(&content);
                            println!("{}", text);
                        }
                        Err(_) => println!("{}", "(no before content)".dimmed()),
                    }

                    println!("\n{}", "After:".bold());
                    match storage.load_after_content(&action_id) {
                        Ok(content) => {
                            let text = String::from_utf8_lossy(&content);
                            println!("{}", text);
                        }
                        Err(_) => println!("{}", "(no after content)".dimmed()),
                    }
                }
            }
        }
        Cli::Revert {
            action_id,
            force,
            dry_run,
            file,
            to,
            continue_on_error,
        } => {
            let dotfiles_path = get_dotfiles_path().unwrap_or_else(|err| fatal!("{}", err));
            let storage = AuditStorage::new(&dotfiles_path);

            // File revert mode
            if let Some(file_path) = file {
                let file_index = storage
                    .load_file_index()
                    .unwrap_or_else(|err| fatal!("Failed to load file index: {}", err));

                let engine = RevertEngine::with_file_index(storage, file_index);

                let options = FileRevertOptions {
                    force,
                    dry_run,
                    continue_on_error,
                    to_action_id: to.map(ActionId::from_string),
                };

                if dry_run {
                    println!("{}", "Dry run — no changes will be made.\n".yellow());
                    println!("  File: {}", file_path.display().to_string().bold());
                    if let Some(ref to_id) = options.to_action_id {
                        println!("  To action: {}", to_id);
                    }
                    println!();
                    let plans = engine
                        .plan_revert_file(&file_path, options.to_action_id.as_ref())
                        .unwrap_or_else(|err| fatal!("Failed to plan revert: {}", err));
                    if plans.is_empty() {
                        println!("{}", "  No actions to revert for this file.".yellow());
                    } else {
                        for plan in &plans {
                            print!("{}", format_revert_plan(plan));
                        }
                    }
                    return Ok(());
                }

                let results = if let Some(ref to_id) = options.to_action_id {
                    engine
                        .revert_file_to(&file_path, to_id, options.clone())
                        .unwrap_or_else(|err| fatal!("File revert failed: {}", err))
                } else {
                    engine
                        .revert_file(&file_path, options)
                        .unwrap_or_else(|err| fatal!("File revert failed: {}", err))
                };

                if results.is_empty() {
                    println!("{}", "No actions to revert for this file.".yellow());
                    return Ok(());
                }

                let mut success_count = 0;
                let mut fail_count = 0;
                for result in &results {
                    if result.success {
                        println!("{} [{}] {}", "✓".green(), result.action_id, result.message);
                        success_count += 1;
                    } else {
                        println!("{} [{}] {}", "✗".red(), result.action_id, result.message);
                        fail_count += 1;
                    }
                }

                println!();
                println!(
                    "{}: {} reverted, {} failed",
                    "Summary".bold(),
                    success_count,
                    fail_count
                );

                if fail_count > 0 {
                    std::process::exit(1);
                }
                return Ok(());
            }

            // Single action revert mode
            let action_id_str = action_id.expect("action_id required when --file not specified");
            let action_id = ActionId::from_string(&action_id_str);
            let session = storage
                .find_session_for_action(&action_id)
                .unwrap_or_else(|err| fatal!("Failed to find action: {}", err));

            let session = match session {
                Some(s) => s,
                None => fatal!("Action '{}' not found", action_id),
            };

            let action = session
                .get_action(&action_id)
                .expect("Action should exist in session");

            if dry_run {
                println!("{}", "Dry run — no changes will be made.\n".yellow());
                let engine = RevertEngine::new(storage);
                let plan = engine.plan_revert(action);
                print!("{}", format_revert_plan(&plan));
                if !plan.can_proceed {
                    println!(
                        "\n  {}",
                        "This action cannot be automatically reverted.".red()
                    );
                }
                return Ok(());
            }

            let engine = RevertEngine::new(storage);
            let result = engine
                .revert(action, force)
                .unwrap_or_else(|err| fatal!("Revert failed: {}", err));

            if result.success {
                println!("{} {}", "✓".green(), result.message);
                if result.modified_warning {
                    println!(
                        "{}: File was modified after the action, forced revert.",
                        "Warning".yellow()
                    );
                }
            } else {
                println!("{} {}", "✗".red(), result.message);
                std::process::exit(1);
            }
        }
        Cli::Migrate {
            dotfiles_dir,
            output,
            dry_run,
        } => {
            let out_dir = output.unwrap_or_else(|| dotfiles_dir.clone());
            match toml_bombadil::migrate::migrate(&dotfiles_dir, &out_dir, dry_run) {
                Ok(report) => report.print(),
                Err(e) => fatal!("migration failed: {}", e),
            }
        }
    };

    Ok(())
}

fn handle_packages_command(command: PackagesCommand) -> Result<()> {
    let settings = Settings::get()?;
    let dotfiles_path = settings.get_dotfiles_path()?;
    let platform = PlatformContext::detect();

    match command {
        PackagesCommand::Install {
            package,
            tags,
            exclude,
            method: _method,
            interactive,
            file,
            profiles: _profiles,
        } => {
            // Get configured packages
            let packages = settings.settings.packages.clone();

            match package {
                Some(pkg_name) => {
                    // Install specific package
                    if packages.contains_key(&pkg_name) {
                        // Known package - install it
                        let manager =
                            PackageManager::new(packages, dotfiles_path.clone(), tags, exclude);
                        let result = manager.install_by_name(&pkg_name)?;

                        // Update state
                        if let InstallStatus::Installed = result.status {
                            let mut state = PackagesState::read(&dotfiles_path)?;
                            state.record_install(
                                &pkg_name,
                                result.method.as_deref().unwrap_or("unknown"),
                                result.version,
                                None,
                            );
                            state.write()?;
                        }
                    } else {
                        // Unknown package - discover, persist, then install
                        println!(
                            "{} '{}' not in config, discovering install methods...",
                            "Package".yellow(),
                            pkg_name
                        );

                        let target_file =
                            file.unwrap_or_else(|| default_packages_file(&dotfiles_path));

                        let new_package = if interactive {
                            // Interactive mode - open editor
                            interactive_add_package(&pkg_name, &tags, &target_file, &dotfiles_path)?
                        } else {
                            // Auto-discover
                            let discovery = discover_package(&pkg_name, &platform);

                            for msg in &discovery.messages {
                                println!("  {}", msg);
                            }

                            if !discovery.methods.has_any() {
                                println!(
                                    "\n{}: No install methods found. Use {} for manual configuration.",
                                    "Hint".cyan(),
                                    "--interactive".bold()
                                );
                                return Ok(());
                            }

                            let new_pkg = discovery.to_package(tags.clone());

                            // Persist to config
                            append_package_to_file(
                                &pkg_name,
                                &new_pkg,
                                &target_file,
                                &dotfiles_path,
                            )?;
                            println!("  {} Added to {}", "✓".green(), target_file.display());

                            new_pkg
                        };

                        // Now install it
                        let mut pkgs = std::collections::HashMap::new();
                        pkgs.insert(pkg_name.clone(), new_package);

                        let manager =
                            PackageManager::new(pkgs, dotfiles_path.clone(), vec![], vec![]);
                        let result = manager.install_by_name(&pkg_name)?;

                        // Update state
                        if let InstallStatus::Installed = result.status {
                            let mut state = PackagesState::read(&dotfiles_path)?;
                            state.record_install(
                                &pkg_name,
                                result.method.as_deref().unwrap_or("unknown"),
                                result.version,
                                None,
                            );
                            state.write()?;
                        }
                    }
                }
                None => {
                    // Install all configured packages matching filters
                    let manager =
                        PackageManager::new(packages, dotfiles_path.clone(), tags, exclude);
                    let results = manager.install_all()?;

                    // Update state for all installed packages
                    let mut state = PackagesState::read(&dotfiles_path)?;
                    for result in results {
                        if let InstallStatus::Installed = result.status {
                            state.record_install(
                                &result.name,
                                result.method.as_deref().unwrap_or("unknown"),
                                result.version,
                                None,
                            );
                        }
                    }
                    state.write()?;
                }
            }
        }
        PackagesCommand::List {
            installed,
            available,
            tags,
            profiles: _profiles,
        } => {
            let packages = settings.settings.packages.clone();
            let state = PackagesState::read(&dotfiles_path)?;

            let manager = PackageManager::new(packages, dotfiles_path, tags, vec![]);

            let pkg_list = manager.list_packages();

            println!("{}", "Configured packages:".bold());
            println!();

            for info in pkg_list {
                let is_installed = state.is_installed(&info.name);

                // Filter based on flags
                if installed && !is_installed {
                    continue;
                }
                if available && is_installed {
                    continue;
                }

                let status_icon = if is_installed {
                    "✓".green()
                } else {
                    "○".yellow()
                };

                let tags_str = if info.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", info.tags.join(", "))
                };

                let methods_str = info.available_methods.join(", ");

                println!(
                    "  {} {}{} ({})",
                    status_icon,
                    info.name.bold(),
                    tags_str.dimmed(),
                    methods_str.dimmed()
                );

                if let Some(pkg_state) = state.get(&info.name) {
                    println!("      {}", pkg_state.display().dimmed());
                }
            }
        }
        PackagesCommand::Update {
            package,
            profiles: _profiles,
        } => {
            println!(
                "{}: Package updates not yet implemented. For now, reinstall with:",
                "Note".yellow()
            );
            if let Some(pkg) = package {
                println!("  bombadil packages install {}", pkg);
            } else {
                println!("  bombadil packages install");
            }
        }
        PackagesCommand::Remove {
            package,
            keep_config,
            yes,
            profiles: _profiles,
        } => {
            let packages = settings.settings.packages.clone();

            // Check if the package is known
            if !packages.contains_key(&package) {
                println!(
                    "{} Package '{}' is not in configuration.",
                    "Error:".red().bold(),
                    package
                );
                return Ok(());
            }

            // Confirmation prompt unless --yes is provided
            if !yes {
                print!(
                    "Remove package '{}'? This will uninstall it from your system. [y/N] ",
                    package.bold()
                );
                use std::io::Write;
                std::io::stdout().flush()?;

                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                let input = input.trim().to_lowercase();

                if input != "y" && input != "yes" {
                    println!("{}", "Aborted.".yellow());
                    return Ok(());
                }
            }

            // Perform the actual removal via the package manager
            let manager = PackageManager::new(packages, dotfiles_path.clone(), vec![], vec![]);
            let result = manager.remove_package(&package)?;

            match result.status {
                RemoveStatus::Removed => {
                    // Update state to reflect removal
                    let mut state = PackagesState::read(&dotfiles_path)?;
                    if state.remove(&package).is_some() {
                        state.write()?;
                    }

                    if !keep_config {
                        println!(
                            "  {}: To also remove from config, edit the packages file manually.",
                            "Note".yellow()
                        );
                    }
                }
                RemoveStatus::Failed(ref msg) => {
                    println!(
                        "  {} Package removal failed: {}",
                        "Error:".red().bold(),
                        msg
                    );
                }
                RemoveStatus::NotFound => {
                    println!(
                        "  {} Package '{}' not found in configuration.",
                        "Warning:".yellow(),
                        package
                    );
                }
            }
        }
        PackagesCommand::Sync {
            prune,
            tags,
            profiles: _profiles,
        } => {
            let packages = settings.settings.packages.clone();
            let state = PackagesState::read(&dotfiles_path)?;

            // Find missing packages
            let configured: Vec<String> = packages.keys().cloned().collect();
            let missing = state.missing_packages(&configured);

            if missing.is_empty() {
                println!("{} All configured packages are installed", "✓".green());
            } else {
                println!(
                    "{} Missing packages: {}",
                    "Installing".green().bold(),
                    missing.join(", ")
                );

                let manager =
                    PackageManager::new(packages.clone(), dotfiles_path.clone(), tags, vec![]);

                let results = manager.install_all()?;

                // Update state
                let mut state = PackagesState::read(&dotfiles_path)?;
                for result in results {
                    if let InstallStatus::Installed = result.status {
                        state.record_install(
                            &result.name,
                            result.method.as_deref().unwrap_or("unknown"),
                            result.version,
                            None,
                        );
                    }
                }
                state.write()?;
            }

            if prune {
                let orphaned = state.orphaned_packages(&configured);
                if !orphaned.is_empty() {
                    println!(
                        "\n{}: Orphaned packages (installed but not in config):",
                        "Warning".yellow()
                    );
                    for pkg in orphaned {
                        println!("  - {}", pkg);
                    }
                    println!(
                        "\n{}: Automatic uninstallation not yet implemented.",
                        "Note".yellow()
                    );
                }
            }
        }
    }

    Ok(())
}
