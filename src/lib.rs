use self::settings::profiles::Profile;
use crate::audit::{
    ActionPlan, ActionType as AuditActionType, AuditStorage, PlannedAction, Session,
};
use crate::conflict::{
    Conflict, ConflictContext, ConflictResolution, ConflictStrategy, ReviewChange, ReviewContext,
};
use crate::gpg::Gpg;
use crate::hook::Hook;
use crate::paths::{unlink, DotPaths};
use crate::templating::Variables;
use anyhow::{anyhow, Context, Result};
use colored::*;
use ignore_files::IgnoreFilter;
use settings::dots::Dot;
use settings::Settings;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::Write;
use std::os::unix;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use std::{fs, io};
use watchexec::{
    action::{Action as WatchAction, Outcome},
    config::{InitConfig, RuntimeConfig},
    error::RuntimeError,
    event::{filekind::FileEventKind, Tag},
    handler::PrintDebug,
    signal::source::MainSignal,
    Watchexec,
};
use watchexec_filterer_ignore::IgnoreFilterer;

pub mod audit;
pub mod conflict;
mod error;
mod gpg;
mod hook;
pub mod packages;
pub mod paths;
pub mod platform;
pub mod settings;
mod templating;
pub mod validate;

// v4 new modules
pub mod config;
pub mod core;
pub mod dots;

// Re-export drift detection from packages (v4)
pub use packages::drift;
pub use packages::managers;

pub(crate) const BOMBADIL_CONFIG: &str = "bombadil.toml";
const STATE_FILE: &str = "previous_state.toml";

/// Result of linking a dotfile.
#[derive(PartialEq, Eq, Debug)]
pub enum LinkResult {
    Updated,
    Created,
    Ignored,
    Unchanged,
}

/// Trait for accessing dot variable paths.
pub(crate) trait DotVar {
    fn vars(&self) -> Option<PathBuf>;
    fn get_source(&self) -> Option<&PathBuf>;

    fn is_default_var_path(&self) -> bool {
        self.vars() == Some(Dot::default_vars())
    }

    fn resolve_from_source(&self, source: &Path, path: &Path) -> Option<PathBuf> {
        let relative_to_dot = settings::dotfile_dir().join(source).join(path);
        let relative_to_dotfile_dir = settings::dotfile_dir().join(path);

        if relative_to_dot.exists() {
            Some(relative_to_dot)
        } else if let Some(parent) = source.parent() {
            if parent.join(path).exists() {
                Some(parent.join(path))
            } else if relative_to_dotfile_dir.exists() && !self.is_default_var_path() {
                Some(relative_to_dotfile_dir)
            } else {
                self.vars_path_not_found(source, path)
            }
        } else {
            self.vars_path_not_found(source, path)
        }
    }

    fn vars_path_not_found(&self, source: &Path, path: &Path) -> Option<PathBuf> {
        if !self.is_default_var_path() {
            eprintln!(
                "{} {:?} {} {:?} {} {:?}",
                "WARNING: Variable path".yellow(),
                path,
                "was neither found in".yellow(),
                source,
                "nor in".yellow(),
                settings::dotfile_dir()
            );
        }
        None
    }
}

impl DotVar for Dot {
    fn vars(&self) -> Option<PathBuf> {
        Some(self.vars.clone())
    }

    fn get_source(&self) -> Option<&PathBuf> {
        Some(&self.source)
    }
}

impl DotVar for settings::dots::DotOverride {
    fn vars(&self) -> Option<PathBuf> {
        self.vars.clone()
    }

    fn get_source(&self) -> Option<&PathBuf> {
        self.source.as_ref()
    }
}

impl settings::dots::DotOverride {
    pub(crate) fn resolve_var_path(&self, origin: Option<&PathBuf>) -> Option<PathBuf> {
        let source = match (self.get_source(), origin) {
            (Some(source), _) => source,
            (None, Some(origin)) => origin,
            _ => panic!("Dot has no source path"),
        };

        let vars = self.vars().unwrap_or_else(Dot::default_vars);
        self.resolve_from_source(source, &vars)
    }
}

/// State tracking for installed symlinks.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub(crate) struct BombadilState {
    #[serde(skip)]
    pub path: PathBuf,
    pub symlinks: HashSet<PathBuf>,
}

impl BombadilState {
    pub fn read(path: PathBuf) -> Result<Self> {
        let state_path = path.join(".dots").join(STATE_FILE);

        if state_path.exists() {
            ::config::Config::builder()
                .add_source(::config::File::from(state_path))
                .build()?
                .try_deserialize::<BombadilState>()
                .map_err(|err| anyhow!("{} : {}", "Previous state format error".red(), err))
        } else {
            Err(anyhow!(
                "Unable to find Previous state file {}",
                state_path.display()
            ))
        }
    }

    pub fn write(&self) -> Result<()> {
        let content = toml::to_string(&self)?;
        fs::write(&self.path, content)?;
        fs::File::open(&self.path)?.sync_data()?;
        Ok(())
    }

    pub fn remove_targets(&self) -> Vec<Result<PathBuf>> {
        let mut unlink_results = vec![];

        self.symlinks.iter().for_each(|path| {
            unlink_results.push(
                unlink(path)
                    .map(|_| path.to_owned())
                    .map_err(|err| anyhow!("Failed to unlink dot entry {:?} : {}", path, err)),
            );
        });

        unlink_results
    }
}

impl From<&Bombadil> for BombadilState {
    fn from(current: &Bombadil) -> Self {
        let path = current
            .dotfiles_absolute_path()
            .unwrap()
            .join(".dots")
            .join(STATE_FILE);
        let symlinks = current
            .dots
            .iter()
            .map(|dot| dot.1.target().unwrap())
            .collect();

        Self { path, symlinks }
    }
}

impl Dot {
    pub(crate) fn install(
        &self,
        vars: &Variables,
        auto_ignored: Vec<PathBuf>,
        profiles: &[String],
    ) -> Result<LinkResult> {
        let source = &self.source()?;
        let target = &self.build_copy_path();
        let source_str = source.to_str().unwrap_or_default();

        let ignored_paths = if self.ignore.is_empty() {
            auto_ignored
        } else {
            let mut ignored_paths = self.get_ignored_paths(source_str)?;
            ignored_paths.extend_from_slice(&auto_ignored);
            ignored_paths
        };

        // Add local vars to the global ones
        let mut vars = vars.clone();

        if let Some(local_vars_path) = self.resolve_var_path() {
            let local_vars = Dot::load_local_vars(&local_vars_path);
            vars.extend(local_vars);
        }

        // Resolve % reference
        vars.resolve_ref();

        // Recursively copy dotfile to .dots directory
        self.traverse_and_copy(source, target, ignored_paths.as_slice(), &vars, profiles)
    }

    fn load_local_vars(source: &Path) -> Variables {
        Variables::from_toml(source).unwrap_or_else(|err| {
            eprintln!("{}", err.to_string().yellow());
            Variables::default()
        })
    }

    fn get_ignored_paths(&self, source_str: &str) -> Result<Vec<PathBuf>> {
        Ok(
            globwalk::GlobWalkerBuilder::from_patterns(source_str, self.ignore.as_slice())
                .build()?
                .filter_map(Result::ok)
                .map(|entry| entry.path().to_path_buf())
                .collect(),
        )
    }

    fn traverse_and_copy(
        &self,
        source: &PathBuf,
        target: &PathBuf,
        ignored: &[PathBuf],
        vars: &Variables,
        profiles: &[String],
    ) -> Result<LinkResult> {
        if ignored.contains(source) {
            return Ok(LinkResult::Ignored);
        }

        // Single file : inject vars and write to .dots/
        if source.is_file() {
            fs::create_dir_all(target.parent().unwrap())?;
            match vars.to_dot(source, profiles) {
                Ok(content) if target.exists() => self.update(source, target, content),
                Ok(content) => self.create(source, target, content),
                Err(_) if target.exists() => self.update_raw(source, target),
                Err(_) => {
                    fs::copy(source, target)?;
                    Ok(LinkResult::Created)
                }
            }
        } else {
            fs::create_dir_all(target)?;
            let mut link_results = vec![];

            for entry in source.read_dir()? {
                let entry_path = &entry?.path();
                let entry_name = entry_path.file_name().unwrap().to_str().unwrap();
                let result = self.traverse_and_copy(
                    &source.join(entry_name),
                    &target.join(entry_name),
                    ignored,
                    vars,
                    &[],
                );

                match result {
                    Ok(result) => link_results.push(result),
                    Err(err) => eprintln!("{err}"),
                }
            }

            if link_results.contains(&LinkResult::Updated) {
                Ok(LinkResult::Updated)
            } else if link_results.contains(&LinkResult::Created) {
                Ok(LinkResult::Created)
            } else {
                Ok(LinkResult::Unchanged)
            }
        }
    }

    fn create(&self, source: &PathBuf, target: &PathBuf, content: String) -> Result<LinkResult> {
        use std::io::Write;
        let permissions = fs::metadata(source)?.permissions();
        let mut dot_copy = fs::File::create(target)?;
        dot_copy.write_all(content.as_bytes())?;
        dot_copy.set_permissions(permissions)?;
        Ok(LinkResult::Created)
    }

    fn update(&self, source: &PathBuf, target: &PathBuf, content: String) -> Result<LinkResult> {
        use std::io::Write;
        let target_content = fs::read_to_string(target)?;
        if target_content == content {
            Ok(LinkResult::Unchanged)
        } else {
            let permissions = fs::metadata(source)?.permissions();
            let mut dot_copy = fs::OpenOptions::new().write(true).truncate(true).open(target)?;
            dot_copy.write_all(content.as_bytes())?;
            dot_copy.set_permissions(permissions)?;
            dot_copy.sync_data()?;
            Ok(LinkResult::Updated)
        }
    }

    fn update_raw(&self, source: &PathBuf, target: &PathBuf) -> Result<LinkResult> {
        use std::io::Write;
        let target_content = fs::read(target)?;
        let content = fs::read(source)?;

        if target_content == content {
            Ok(LinkResult::Unchanged)
        } else {
            let permissions = fs::metadata(source)?.permissions();
            let mut dot_copy = fs::OpenOptions::new().write(true).truncate(true).open(target)?;

            dot_copy.write_all(&content)?;
            dot_copy.set_permissions(permissions)?;
            dot_copy.sync_data()?;
            Ok(LinkResult::Updated)
        }
    }
}

/// The main crate struct, it contains all needed medata about a
/// dotfile directory and how to install it.
#[derive(Clone)]
pub struct Bombadil {
    // path to self configuration, relative to $HOME
    path: PathBuf,
    // A list of dotfiles to link for this instance
    dots: HashMap<String, Dot>,
    // Variables for the tera template context
    vars: Variables,
    // Pre-hook commands, run before `bombadil-link`
    prehooks: Vec<Hook>,
    // Post-hook commands, run after `bombadil-link`
    posthooks: Vec<Hook>,
    // Available profiles
    profiles: HashMap<String, Profile>,
    // Profiles enabled for this isntance
    profile_enabled: Vec<String>,
    // A GPG user id, linking to user encryption/decryption key via gnupg
    gpg: Option<Gpg>,
}

/// Enable or disable GPG encryption when linking dotfiles
pub enum Mode {
    Gpg,
    NoGpg,
}

impl Bombadil {
    /// Symlink `bombadil.toml` to `$XDG_CONFIG/bombadil.toml` so we can later read it from there.
    pub fn link_self_config(dotfiles_path: Option<PathBuf>) -> Result<()> {
        // Get the provided path and attempt to resolve 'bombadil.toml' if it's a directory
        let path = match dotfiles_path {
            None => PathBuf::from(BOMBADIL_CONFIG),
            Some(path) if path.is_dir() => path.join(BOMBADIL_CONFIG),
            Some(path) => path,
        };

        match path.canonicalize() {
            Ok(path) => {
                match dirs::config_dir() {
                    None => Err(anyhow!("$XDG_CONFIG does not exist")),
                    Some(config_dir) => {
                        let bombadil_xdg_config = config_dir.join(BOMBADIL_CONFIG);

                        // Attempt to locate a previous '$HOME/.settings/bombadil.toml' link and remove it
                        if fs::symlink_metadata(&bombadil_xdg_config).is_ok() {
                            fs::remove_file(&bombadil_xdg_config)?;
                        }

                        // Symlink to '$HOME/.settings/bombadil.toml'
                        unix::fs::symlink(&path, &bombadil_xdg_config)
                            .map_err(|err| {
                                anyhow!(
                                    "Failed to symlink {:?} to {:?} : {}",
                                    path,
                                    bombadil_xdg_config,
                                    err
                                )
                            })
                            .map(|_result| {
                                let source = format!("{:?}", &path).blue();
                                let dest = format!("{:?}", &bombadil_xdg_config).green();
                                println!("{} => {}", source, dest)
                            })
                    }
                }
            }
            Err(_err) => {
                let err = format!("{path:?} {}", "not found in current directory");
                Err(anyhow!("{}", err.red()))
            }
        }
    }

    /// The installation process is composed of the following steps :
    /// 1. Run pre install hooks
    /// 2. If any previous state is found in `.dot/previous_state.toml`, remove the existing symlinks
    /// 3. Clean existing rendered dotfiles templates in `.dot`
    /// 4. Copy and symlink dotfiles according to the current `$XDG_CONFIG/bombadil.toml` configuration
    /// 5. Run post install hooks
    /// 6. Write current state to `.dot/previous_state.toml`
    pub fn install(&self) -> Result<()> {
        self.install_with_strategy(ConflictStrategy::Interactive)
    }

    /// Install dotfiles with a specific conflict resolution strategy
    pub fn install_with_strategy(&self, strategy: ConflictStrategy) -> Result<()> {
        self.check_dotfile_dir()?;

        self.prehooks.iter().map(Hook::run).for_each(|result| {
            if let Err(err) = result {
                eprintln!("{}", err);
            }
        });
        let dot_copy_dir = &self.path.join(".dots");

        // Conflict resolution context
        let mut conflict_ctx = ConflictContext::new(strategy);

        // Render current settings and create symlinks
        fs::create_dir_all(dot_copy_dir)?;
        for (key, dot) in self.dots.iter() {
            match dot.install(
                &self.vars,
                self.get_auto_ignored_files(key),
                self.profile_enabled.as_slice(),
            ) {
                Err(err) => {
                    eprintln!("{}", err);
                    continue;
                }
                Ok(linked) => {
                    let copy_path = &dot.copy_path()?;
                    let target = &dot.target()?;

                    match linked {
                        LinkResult::Updated => {
                            let source = format!("{:?}", copy_path).blue();
                            let dest = format!("{:?}", target).yellow();
                            println!("{} => {}", source, dest)
                        }
                        LinkResult::Created => {
                            let source = format!("{:?}", copy_path).blue();
                            let dest = format!("{:?}", target).green();
                            println!("Created - {} => {}", source, dest)
                        }
                        LinkResult::Ignored => {
                            let source = format!("{:?}", copy_path);
                            let dest = format!("{:?}", target);
                            println!("Ignored - {} => {}", source, dest)
                        }
                        LinkResult::Unchanged => {
                            let source = format!("{:?}", copy_path);
                            let dest = format!("{:?}", target);
                            println!("Unchanged - {} => {}", source, dest)
                        }
                    }
                }
            }

            // Check for conflicts before symlinking
            let copy_path = match dot.copy_path() {
                Ok(p) => p,
                Err(_) => {
                    // If we can't get the copy path, just try to symlink anyway
                    dot.symlink()?;
                    continue;
                }
            };
            let target = match dot.target() {
                Ok(t) => t,
                Err(_) => {
                    dot.symlink()?;
                    continue;
                }
            };
            let source = match dot.source() {
                Ok(s) => s,
                Err(_) => {
                    dot.symlink()?;
                    continue;
                }
            };

            // Detect conflict
            match Conflict::detect(key, &copy_path, &target, &source) {
                Ok(Some(conflict)) => {
                    let resolution = conflict_ctx.resolve(&conflict)?;

                    match resolution {
                        ConflictResolution::UseDotfile | ConflictResolution::UseDotfileForAll => {
                            // Backup and proceed with symlink
                            crate::paths::backup_and_unlink(&target)?;
                            dot.symlink()?;
                        }
                        ConflictResolution::UseSystem | ConflictResolution::UseSystemForAll => {
                            // Copy system content back to dotfile source
                            conflict.apply_use_system()?;
                            // Re-render the dot with updated source
                            let _ = dot.install(
                                &self.vars,
                                self.get_auto_ignored_files(key),
                                self.profile_enabled.as_slice(),
                            );
                            // Now symlink (target content now matches)
                            crate::paths::backup_and_unlink(&target)?;
                            dot.symlink()?;
                        }
                        ConflictResolution::Skip | ConflictResolution::SkipAll => {
                            println!("  {} Skipping: {}", "→".yellow(), target.display());
                            // Don't symlink, leave system file as-is
                        }
                    }
                }
                Ok(None) => {
                    // No conflict, proceed normally
                    dot.symlink()?;
                }
                Err(e) => {
                    eprintln!("Error detecting conflict for {}: {}", key, e);
                    dot.symlink()?;
                }
            }
        }

        // Print conflict summary
        conflict_ctx.print_summary();

        // Run post install hooks
        self.posthooks.iter().map(Hook::run).for_each(|result| {
            if let Err(err) = result {
                eprintln!("Failed to run posthook: {}", err);
            }
        });

        // Dump current settings
        let absolute_path_to_dot = &self.dotfiles_absolute_path()?;

        // Get previous state if any and remove symlinks
        let previous_state = BombadilState::read(absolute_path_to_dot.to_owned());
        let new_state = BombadilState::from(self);

        match previous_state {
            Ok(previous_state) => {
                let diff = previous_state.symlinks.difference(&new_state.symlinks);
                let diff_vec: Vec<_> = diff.collect();

                tracing::debug!(orphans = ?diff_vec, "Checking for orphaned symlinks");

                for orphan in diff_vec {
                    if orphan.exists() {
                        if let Ok(canonicalized) = orphan.canonicalize() {
                            unlink(orphan).context(format!(
                                "unlinking `{}`",
                                canonicalized.to_str().to_owned().unwrap().green()
                            ))?;
                            if canonicalized.is_dir() {
                                fs::remove_dir_all(&canonicalized)
                            } else {
                                fs::remove_file(&canonicalized)
                            }
                            .context(format!(
                                "deleting `{}`",
                                canonicalized.to_str().to_owned().unwrap().green()
                            ))?;
                            tracing::info!(target = ?canonicalized, symlink = ?orphan, "Deleted orphaned symlink");
                        }
                    }
                }
            }
            Err(err) => {
                tracing::debug!(error = %err, "No previous state found");
            }
        }

        new_state.write()?;

        Ok(())
    }

    /// Plan the installation without executing anything.
    ///
    /// Returns an ActionPlan that can be reviewed, modified, and then executed.
    pub fn plan_install(&self, strategy: ConflictStrategy) -> Result<ActionPlan> {
        self.check_dotfile_dir()?;

        let mut plan = ActionPlan::new();
        let _dot_copy_dir = self.path.join(".dots");

        // Plan prehooks first (they run before dot operations)
        for hook in &self.prehooks {
            plan.add(PlannedAction::new(
                AuditActionType::HookExecuted {
                    command: hook.command.clone(),
                    hook_type: crate::audit::HookType::PreInstall,
                    exit_code: 0,
                },
                None,
            ));
        }

        // Plan each dot
        for (key, dot) in self.dots.iter() {
            // Determine what the install would do
            let source = match dot.source() {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(dot = %key, error = %e, "Skipping dot due to source error");
                    continue;
                }
            };

            let copy_path = dot.build_copy_path();
            let target = match dot.target() {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(dot = %key, error = %e, "Skipping dot due to target error");
                    continue;
                }
            };

            // Determine action type based on current state
            let action_type = if target.exists() {
                // Check if this is a conflict
                match Conflict::detect(key, &copy_path, &target, &source) {
                    Ok(Some(_conflict)) => {
                        // There's a conflict - plan resolution based on strategy
                        match strategy {
                            ConflictStrategy::DotfileWins => {
                                // Will backup system file and use dotfile
                                plan.add(PlannedAction::new(
                                    AuditActionType::Backup {
                                        original: target.clone(),
                                        backup_location: self.backup_path(&target),
                                        content_hash: self.hash_file_if_exists(&target),
                                    },
                                    Some(key.clone()),
                                ));
                                AuditActionType::FileUpdate {
                                    target: target.clone(),
                                    source: source.clone(),
                                    before_hash: self.hash_file_if_exists(&target),
                                    after_hash: String::new(), // Computed at execution
                                }
                            }
                            ConflictStrategy::SystemWins => {
                                // Will copy system content to dotfile source
                                AuditActionType::ConflictResolved {
                                    target: target.clone(),
                                    resolution: crate::audit::ConflictResolution::UseSystem,
                                    dotfile_hash: self.hash_file_if_exists(&source),
                                    system_hash: self.hash_file_if_exists(&target),
                                }
                            }
                            ConflictStrategy::Skip => {
                                // Skip this dot
                                continue;
                            }
                            ConflictStrategy::Interactive => {
                                // Mark as conflict - will be resolved interactively
                                AuditActionType::ConflictResolved {
                                    target: target.clone(),
                                    resolution: crate::audit::ConflictResolution::Pending,
                                    dotfile_hash: self.hash_file_if_exists(&source),
                                    system_hash: self.hash_file_if_exists(&target),
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        // No conflict - regular update
                        if target.is_symlink() {
                            // Already a symlink, will update content
                            AuditActionType::FileUpdate {
                                target: target.clone(),
                                source: source.clone(),
                                before_hash: self.hash_file_if_exists(&copy_path),
                                after_hash: String::new(),
                            }
                        } else {
                            // Target exists but not a symlink - backup and link
                            plan.add(PlannedAction::new(
                                AuditActionType::Backup {
                                    original: target.clone(),
                                    backup_location: self.backup_path(&target),
                                    content_hash: self.hash_file_if_exists(&target),
                                },
                                Some(key.clone()),
                            ));
                            AuditActionType::SymlinkCreate {
                                source: copy_path.clone(),
                                target: target.clone(),
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(dot = %key, error = %e, "Error detecting conflict, will attempt link");
                        AuditActionType::SymlinkCreate {
                            source: copy_path.clone(),
                            target: target.clone(),
                        }
                    }
                }
            } else if copy_path.exists() {
                // Target doesn't exist but copy does - create symlink
                AuditActionType::SymlinkCreate {
                    source: copy_path.clone(),
                    target: target.clone(),
                }
            } else {
                // Neither exists - create new file and symlink
                AuditActionType::FileCreate {
                    target: target.clone(),
                    source: source.clone(),
                    content_hash: String::new(),
                }
            };

            plan.add(PlannedAction::new(action_type, Some(key.clone())));
        }

        // Plan orphan cleanup
        let absolute_path_to_dot = self.dotfiles_absolute_path()?;
        if let Ok(previous_state) = BombadilState::read(absolute_path_to_dot.clone()) {
            let new_state = BombadilState::from(self);
            let orphans: Vec<_> = previous_state
                .symlinks
                .difference(&new_state.symlinks)
                .filter(|p| p.exists())
                .collect();

            for orphan in orphans {
                if let Ok(target) = orphan.canonicalize() {
                    plan.add(PlannedAction::new(
                        AuditActionType::SymlinkRemove {
                            target: orphan.clone(),
                            was_pointing_to: target,
                        },
                        None,
                    ));
                }
            }
        }

        // Plan posthooks last (they run after dot operations)
        for hook in &self.posthooks {
            plan.add(PlannedAction::new(
                AuditActionType::HookExecuted {
                    command: hook.command.clone(),
                    hook_type: crate::audit::HookType::PostInstall,
                    exit_code: 0,
                },
                None,
            ));
        }

        Ok(plan)
    }

    /// Execute a planned installation.
    ///
    /// Takes a previously created ActionPlan and executes the approved actions.
    pub fn execute_install(&self, _plan: ActionPlan, profiles: Vec<String>) -> Result<Session> {
        self.execute_install_with_strategy(_plan, profiles, ConflictStrategy::Interactive)
    }

    /// Execute a planned installation with a specific conflict strategy.
    pub fn execute_install_with_strategy(
        &self,
        _plan: ActionPlan,
        profiles: Vec<String>,
        strategy: ConflictStrategy,
    ) -> Result<Session> {
        let dotfiles_path = self.dotfiles_absolute_path()?;
        let storage = AuditStorage::new(&dotfiles_path);
        storage.init()?;

        let mut session = Session::new("link", profiles.clone());
        let mut conflict_ctx = ConflictContext::new(strategy);

        // Run prehooks
        for hook in &self.prehooks {
            match hook.run_capture() {
                Ok(result) => {
                    let action = crate::audit::Action::new(
                        AuditActionType::HookExecuted {
                            command: result.command.clone(),
                            hook_type: crate::audit::HookType::PreInstall,
                            exit_code: result.exit_code,
                        },
                        None,
                    );

                    // Save hook output as logs
                    let log_content = format_hook_logs(&result);
                    if !log_content.is_empty() {
                        let _ = storage.save_logs(&action.id, &log_content);
                    }

                    session.add_action(action);

                    if !result.success() {
                        tracing::error!(
                            command = %result.command,
                            exit_code = result.exit_code,
                            "Prehook failed"
                        );
                    }
                }
                Err(err) => {
                    tracing::error!(error = %err, "Failed to run prehook");
                }
            }
        }

        let dot_copy_dir = self.path.join(".dots");
        fs::create_dir_all(&dot_copy_dir)?;

        // Execute each dot installation
        for (key, dot) in self.dots.iter() {
            // Use build_copy_path (doesn't require file to exist) for pre-check
            let copy_path = dot.build_copy_path();
            let target = match dot.target() {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(dot = %key, error = %e, "Skipping dot due to target error");
                    continue;
                }
            };
            let source = match dot.source() {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(dot = %key, error = %e, "Skipping dot due to source error");
                    continue;
                }
            };

            // Check for local modifications BEFORE rendering
            // If target is a symlink to .dots/, the user may have modified it
            let pre_render_content = if copy_path.exists() {
                fs::read(&copy_path).ok()
            } else {
                None
            };

            // Render template
            match dot.install(
                &self.vars,
                self.get_auto_ignored_files(key),
                self.profile_enabled.as_slice(),
            ) {
                Err(err) => {
                    tracing::error!(dot = %key, error = %err, "Failed to render dot");
                    continue;
                }
                Ok(link_result) => {
                    // Check if we overwrote local modifications
                    if let Some(ref old_content) = pre_render_content {
                        if let Ok(new_content) = fs::read(&copy_path) {
                            if old_content != &new_content && strategy != ConflictStrategy::DotfileWins {
                                // Content changed - user had local modifications
                                let old_str = String::from_utf8_lossy(old_content).to_string();
                                let new_str = String::from_utf8_lossy(&new_content).to_string();

                                // Create a conflict-like prompt
                                // Swap: system_content is what user has (will be shown as -)
                                //       dotfile_content is new render (will be shown as +)
                                let local_mod_conflict = Conflict {
                                    dot_name: key.clone(),
                                    target_path: target.clone(),
                                    source_path: source.clone(),
                                    rendered_path: copy_path.clone(),
                                    dotfile_content: old_str.clone(),  // User's current (shown as -)
                                    system_content: new_str.clone(),   // New render (shown as +)
                                };

                                let resolution = conflict_ctx.resolve(&local_mod_conflict)?;

                                // Record the conflict resolution as an action
                                let audit_resolution = match &resolution {
                                    ConflictResolution::UseDotfile
                                    | ConflictResolution::UseDotfileForAll => {
                                        crate::audit::ConflictResolution::UseDotfile
                                    }
                                    ConflictResolution::UseSystem
                                    | ConflictResolution::UseSystemForAll => {
                                        crate::audit::ConflictResolution::UseSystem
                                    }
                                    ConflictResolution::Skip | ConflictResolution::SkipAll => {
                                        crate::audit::ConflictResolution::KeepSystem
                                    }
                                };

                                let conflict_action = crate::audit::Action::new(
                                    AuditActionType::ConflictResolved {
                                        target: target.clone(),
                                        resolution: audit_resolution,
                                        dotfile_hash: crate::audit::content_hash(&new_content),
                                        system_hash: crate::audit::content_hash(old_content),
                                    },
                                    Some(key.clone()),
                                );
                                session.add_action(conflict_action);

                                match resolution {
                                    ConflictResolution::UseSystem
                                    | ConflictResolution::UseSystemForAll => {
                                        // Restore the user's modifications
                                        fs::write(&copy_path, old_content)?;
                                        tracing::info!(dot = %key, "Kept local modifications");
                                        continue; // Skip further processing for this dot
                                    }
                                    ConflictResolution::Skip | ConflictResolution::SkipAll => {
                                        // Restore and skip
                                        fs::write(&copy_path, old_content)?;
                                        continue;
                                    }
                                    _ => {
                                        // UseDotfile - keep the new rendered content
                                    }
                                }
                            }
                        }
                    }

                    // Determine action type and record
                    let (action_type, should_symlink) = match link_result {
                        LinkResult::Created => {
                            let content = fs::read(&copy_path).unwrap_or_default();
                            let hash = crate::audit::content_hash(&content);

                            // Save content for revert
                            let action = crate::audit::Action::new(
                                AuditActionType::FileCreate {
                                    target: target.clone(),
                                    source: source.clone(),
                                    content_hash: hash.clone(),
                                },
                                Some(key.clone()),
                            );
                            let _ = storage.save_after_content(&action.id, &content);

                            (Some(action), true)
                        }
                        LinkResult::Updated => {
                            let before = fs::read(&target).ok();
                            let after = fs::read(&copy_path).unwrap_or_default();
                            let before_hash = before
                                .as_ref()
                                .map(|b| crate::audit::content_hash(b))
                                .unwrap_or_default();
                            let after_hash = crate::audit::content_hash(&after);

                            let action = crate::audit::Action::new(
                                AuditActionType::FileUpdate {
                                    target: target.clone(),
                                    source: source.clone(),
                                    before_hash,
                                    after_hash,
                                },
                                Some(key.clone()),
                            );

                            if let Some(ref b) = before {
                                let _ = storage.save_before_content(&action.id, b);
                            }
                            let _ = storage.save_after_content(&action.id, &after);

                            (Some(action), true)
                        }
                        LinkResult::Unchanged => (None, true),
                        LinkResult::Ignored => (None, false),
                    };

                    // Add to session
                    if let Some(action) = action_type {
                        session.add_action(action);
                    }

                    // Check for conflicts before symlinking
                    let should_symlink = if should_symlink {
                        match Conflict::detect(key, &copy_path, &target, &source) {
                            Ok(Some(conflict)) => {
                                let resolution = conflict_ctx.resolve(&conflict)?;

                                // Record the conflict resolution
                                let audit_resolution = match &resolution {
                                    ConflictResolution::UseDotfile
                                    | ConflictResolution::UseDotfileForAll => {
                                        crate::audit::ConflictResolution::UseDotfile
                                    }
                                    ConflictResolution::UseSystem
                                    | ConflictResolution::UseSystemForAll => {
                                        crate::audit::ConflictResolution::UseSystem
                                    }
                                    ConflictResolution::Skip | ConflictResolution::SkipAll => {
                                        crate::audit::ConflictResolution::KeepSystem
                                    }
                                };

                                let dotfile_hash = fs::read(&copy_path)
                                    .map(|c| crate::audit::content_hash(&c))
                                    .unwrap_or_default();
                                let system_hash = fs::read(&target)
                                    .map(|c| crate::audit::content_hash(&c))
                                    .unwrap_or_default();

                                let conflict_action = crate::audit::Action::new(
                                    AuditActionType::ConflictResolved {
                                        target: target.clone(),
                                        resolution: audit_resolution,
                                        dotfile_hash,
                                        system_hash,
                                    },
                                    Some(key.clone()),
                                );
                                session.add_action(conflict_action);

                                match resolution {
                                    ConflictResolution::UseDotfile
                                    | ConflictResolution::UseDotfileForAll => {
                                        // Backup and proceed with symlink
                                        crate::paths::backup_and_unlink(&target)?;
                                        true
                                    }
                                    ConflictResolution::UseSystem
                                    | ConflictResolution::UseSystemForAll => {
                                        // Copy system content back to dotfile source
                                        conflict.apply_use_system()?;
                                        // Re-render the dot with updated source
                                        let _ = dot.install(
                                            &self.vars,
                                            self.get_auto_ignored_files(key),
                                            self.profile_enabled.as_slice(),
                                        );
                                        // Now symlink (target content now matches)
                                        crate::paths::backup_and_unlink(&target)?;
                                        true
                                    }
                                    ConflictResolution::Skip | ConflictResolution::SkipAll => {
                                        // Don't symlink, leave system file as-is
                                        false
                                    }
                                }
                            }
                            Ok(None) => true,  // No conflict
                            Err(e) => {
                                tracing::warn!(dot = %key, error = %e, "Error detecting conflict");
                                true
                            }
                        }
                    } else {
                        false
                    };

                    // Create symlink if needed
                    if should_symlink {
                        if let Err(e) = dot.symlink() {
                            tracing::error!(dot = %key, error = %e, "Failed to create symlink");
                        }
                    }
                }
            }
        }

        // Print conflict summary
        conflict_ctx.print_summary();

        // Run posthooks
        for hook in &self.posthooks {
            match hook.run_capture() {
                Ok(result) => {
                    let action = crate::audit::Action::new(
                        AuditActionType::HookExecuted {
                            command: result.command.clone(),
                            hook_type: crate::audit::HookType::PostInstall,
                            exit_code: result.exit_code,
                        },
                        None,
                    );

                    // Save hook output as logs
                    let log_content = format_hook_logs(&result);
                    if !log_content.is_empty() {
                        let _ = storage.save_logs(&action.id, &log_content);
                    }

                    session.add_action(action);

                    if !result.success() {
                        tracing::error!(
                            command = %result.command,
                            exit_code = result.exit_code,
                            "Posthook failed"
                        );
                    }
                }
                Err(err) => {
                    tracing::error!(error = %err, "Failed to run posthook");
                }
            }
        }

        // Clean up orphaned symlinks
        let absolute_path_to_dot = self.dotfiles_absolute_path()?;
        let previous_state = BombadilState::read(absolute_path_to_dot.clone());
        let new_state = BombadilState::from(self);

        if let Ok(previous_state) = previous_state {
            let diff = previous_state.symlinks.difference(&new_state.symlinks);
            for orphan in diff {
                if orphan.exists() {
                    if let Ok(canonicalized) = orphan.canonicalize() {
                        if let Ok(()) = unlink(orphan) {
                            if canonicalized.is_dir() {
                                let _ = fs::remove_dir_all(&canonicalized);
                            } else {
                                let _ = fs::remove_file(&canonicalized);
                            }

                            session.add_action(crate::audit::Action::new(
                                AuditActionType::SymlinkRemove {
                                    target: orphan.clone(),
                                    was_pointing_to: canonicalized,
                                },
                                None,
                            ));
                        }
                    }
                }
            }
        }

        // Save new state
        new_state.write()?;

        // Complete and save session
        session.complete();
        if !session.is_empty() {
            storage.save_session(&session)?;
        }

        Ok(session)
    }

    /// Execute a planned installation with options.
    ///
    /// When `review` is true, prompts the user to approve each change (creates and updates).
    pub fn execute_install_with_options(
        &self,
        _plan: ActionPlan,
        profiles: Vec<String>,
        strategy: ConflictStrategy,
        review: bool,
    ) -> Result<Session> {
        let dotfiles_path = self.dotfiles_absolute_path()?;
        let storage = AuditStorage::new(&dotfiles_path);
        storage.init()?;

        let mut session = Session::new("link", profiles.clone());
        let mut conflict_ctx = ConflictContext::new(strategy);
        let mut review_ctx = ReviewContext::new(review);

        // Run prehooks
        for hook in &self.prehooks {
            match hook.run_capture() {
                Ok(result) => {
                    let action = crate::audit::Action::new(
                        AuditActionType::HookExecuted {
                            command: result.command.clone(),
                            hook_type: crate::audit::HookType::PreInstall,
                            exit_code: result.exit_code,
                        },
                        None,
                    );

                    // Save hook output as logs
                    let log_content = format_hook_logs(&result);
                    if !log_content.is_empty() {
                        let _ = storage.save_logs(&action.id, &log_content);
                    }

                    session.add_action(action);

                    if !result.success() {
                        tracing::error!(
                            command = %result.command,
                            exit_code = result.exit_code,
                            "Prehook failed"
                        );
                    }
                }
                Err(err) => {
                    tracing::error!(error = %err, "Failed to run prehook");
                }
            }
        }

        let dot_copy_dir = self.path.join(".dots");
        fs::create_dir_all(&dot_copy_dir)?;

        // Execute each dot installation
        for (key, dot) in self.dots.iter() {
            // Use build_copy_path (doesn't require file to exist) for pre-check
            let copy_path = dot.build_copy_path();
            let target = match dot.target() {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(dot = %key, error = %e, "Skipping dot due to target error");
                    continue;
                }
            };
            let source = match dot.source() {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(dot = %key, error = %e, "Skipping dot due to source error");
                    continue;
                }
            };

            // Check for local modifications BEFORE rendering
            let pre_render_content = if copy_path.exists() {
                fs::read(&copy_path).ok()
            } else {
                None
            };

            // Render template
            match dot.install(
                &self.vars,
                self.get_auto_ignored_files(key),
                self.profile_enabled.as_slice(),
            ) {
                Err(err) => {
                    tracing::error!(dot = %key, error = %err, "Failed to render dot");
                    continue;
                }
                Ok(link_result) => {
                    // Check if we overwrote local modifications (only when not in review mode,
                    // as review mode will handle all changes anyway)
                    if !review && strategy != ConflictStrategy::DotfileWins {
                        if let Some(ref old_content) = pre_render_content {
                            if let Ok(new_content) = fs::read(&copy_path) {
                                if old_content != &new_content {
                                    // Content changed - user had local modifications
                                    let old_str = String::from_utf8_lossy(old_content).to_string();
                                    let new_str = String::from_utf8_lossy(&new_content).to_string();

                                    let local_mod_conflict = Conflict {
                                        dot_name: key.clone(),
                                        target_path: target.clone(),
                                        source_path: source.clone(),
                                        rendered_path: copy_path.clone(),
                                        dotfile_content: old_str.clone(),
                                        system_content: new_str.clone(),
                                    };

                                    let resolution = conflict_ctx.resolve(&local_mod_conflict)?;

                                    // Record the conflict resolution as an action
                                    let audit_resolution = match &resolution {
                                        ConflictResolution::UseDotfile
                                        | ConflictResolution::UseDotfileForAll => {
                                            crate::audit::ConflictResolution::UseDotfile
                                        }
                                        ConflictResolution::UseSystem
                                        | ConflictResolution::UseSystemForAll => {
                                            crate::audit::ConflictResolution::UseSystem
                                        }
                                        ConflictResolution::Skip | ConflictResolution::SkipAll => {
                                            crate::audit::ConflictResolution::KeepSystem
                                        }
                                    };

                                    let conflict_action = crate::audit::Action::new(
                                        AuditActionType::ConflictResolved {
                                            target: target.clone(),
                                            resolution: audit_resolution,
                                            dotfile_hash: crate::audit::content_hash(&new_content),
                                            system_hash: crate::audit::content_hash(old_content),
                                        },
                                        Some(key.clone()),
                                    );
                                    session.add_action(conflict_action);

                                    match resolution {
                                        ConflictResolution::UseSystem
                                        | ConflictResolution::UseSystemForAll => {
                                            fs::write(&copy_path, old_content)?;
                                            tracing::info!(dot = %key, "Kept local modifications");
                                            continue;
                                        }
                                        ConflictResolution::Skip | ConflictResolution::SkipAll => {
                                            fs::write(&copy_path, old_content)?;
                                            continue;
                                        }
                                        _ => {
                                            // UseDotfile - keep the new rendered content
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // If in review mode, check if user wants to apply this change
                    let should_proceed = if review {
                        let new_content = fs::read_to_string(&copy_path).unwrap_or_default();
                        let target_exists = target.exists() && !target.is_symlink();
                        let target_is_our_symlink = if target.is_symlink() {
                            target
                                .canonicalize()
                                .map(|p| p.starts_with(&dot_copy_dir))
                                .unwrap_or(false)
                        } else {
                            false
                        };

                        // Only prompt for actual changes (creates or updates)
                        match link_result {
                            LinkResult::Created | LinkResult::Updated => {
                                let review_change = if target_exists {
                                    // It's an update to an existing file
                                    let old_content =
                                        fs::read_to_string(&target).unwrap_or_default();
                                    Some(ReviewChange::update(
                                        Some(key.clone()),
                                        target.clone(),
                                        old_content,
                                        new_content,
                                    ))
                                } else if target_is_our_symlink {
                                    // Already our symlink, but content changed
                                    if let Some(ref old) = pre_render_content {
                                        let old_str = String::from_utf8_lossy(old).to_string();
                                        Some(ReviewChange::update(
                                            Some(key.clone()),
                                            target.clone(),
                                            old_str,
                                            new_content,
                                        ))
                                    } else {
                                        // No old content to compare, skip review
                                        None
                                    }
                                } else {
                                    // It's a new file
                                    Some(ReviewChange::create(
                                        Some(key.clone()),
                                        target.clone(),
                                        new_content,
                                    ))
                                };

                                match review_change {
                                    Some(change) => review_ctx.review(&change)?,
                                    None => true, // No change to review, proceed
                                }
                            }
                            _ => true, // Unchanged or Ignored - no prompt needed
                        }
                    } else {
                        true
                    };

                    if !should_proceed {
                        // User skipped this change in review mode
                        // Restore old content if we had any
                        if let Some(ref old_content) = pre_render_content {
                            fs::write(&copy_path, old_content)?;
                        }
                        continue;
                    }

                    // Determine action type and record
                    let (action_type, should_symlink) = match link_result {
                        LinkResult::Created => {
                            let content = fs::read(&copy_path).unwrap_or_default();
                            let hash = crate::audit::content_hash(&content);

                            let action = crate::audit::Action::new(
                                AuditActionType::FileCreate {
                                    target: target.clone(),
                                    source: source.clone(),
                                    content_hash: hash.clone(),
                                },
                                Some(key.clone()),
                            );
                            let _ = storage.save_after_content(&action.id, &content);

                            (Some(action), true)
                        }
                        LinkResult::Updated => {
                            let before = fs::read(&target).ok();
                            let after = fs::read(&copy_path).unwrap_or_default();
                            let before_hash = before
                                .as_ref()
                                .map(|b| crate::audit::content_hash(b))
                                .unwrap_or_default();
                            let after_hash = crate::audit::content_hash(&after);

                            let action = crate::audit::Action::new(
                                AuditActionType::FileUpdate {
                                    target: target.clone(),
                                    source: source.clone(),
                                    before_hash,
                                    after_hash,
                                },
                                Some(key.clone()),
                            );

                            if let Some(ref b) = before {
                                let _ = storage.save_before_content(&action.id, b);
                            }
                            let _ = storage.save_after_content(&action.id, &after);

                            (Some(action), true)
                        }
                        LinkResult::Unchanged => (None, true),
                        LinkResult::Ignored => (None, false),
                    };

                    // Add to session
                    if let Some(action) = action_type {
                        session.add_action(action);
                    }

                    // Check for conflicts before symlinking (unless in review mode, where we already asked)
                    let should_symlink = if should_symlink && !review {
                        match Conflict::detect(key, &copy_path, &target, &source) {
                            Ok(Some(conflict)) => {
                                let resolution = conflict_ctx.resolve(&conflict)?;

                                // Record the conflict resolution
                                let audit_resolution = match &resolution {
                                    ConflictResolution::UseDotfile
                                    | ConflictResolution::UseDotfileForAll => {
                                        crate::audit::ConflictResolution::UseDotfile
                                    }
                                    ConflictResolution::UseSystem
                                    | ConflictResolution::UseSystemForAll => {
                                        crate::audit::ConflictResolution::UseSystem
                                    }
                                    ConflictResolution::Skip | ConflictResolution::SkipAll => {
                                        crate::audit::ConflictResolution::KeepSystem
                                    }
                                };

                                let dotfile_hash = fs::read(&copy_path)
                                    .map(|c| crate::audit::content_hash(&c))
                                    .unwrap_or_default();
                                let system_hash = fs::read(&target)
                                    .map(|c| crate::audit::content_hash(&c))
                                    .unwrap_or_default();

                                let conflict_action = crate::audit::Action::new(
                                    AuditActionType::ConflictResolved {
                                        target: target.clone(),
                                        resolution: audit_resolution,
                                        dotfile_hash,
                                        system_hash,
                                    },
                                    Some(key.clone()),
                                );
                                session.add_action(conflict_action);

                                match resolution {
                                    ConflictResolution::UseDotfile
                                    | ConflictResolution::UseDotfileForAll => {
                                        crate::paths::backup_and_unlink(&target)?;
                                        true
                                    }
                                    ConflictResolution::UseSystem
                                    | ConflictResolution::UseSystemForAll => {
                                        conflict.apply_use_system()?;
                                        let _ = dot.install(
                                            &self.vars,
                                            self.get_auto_ignored_files(key),
                                            self.profile_enabled.as_slice(),
                                        );
                                        crate::paths::backup_and_unlink(&target)?;
                                        true
                                    }
                                    ConflictResolution::Skip | ConflictResolution::SkipAll => false,
                                }
                            }
                            Ok(None) => true,
                            Err(e) => {
                                tracing::warn!(dot = %key, error = %e, "Error detecting conflict");
                                true
                            }
                        }
                    } else if should_symlink && review {
                        // In review mode, we already confirmed - backup if needed
                        if target.exists() && !target.is_symlink() {
                            crate::paths::backup_and_unlink(&target)?;
                        } else if target.is_symlink() {
                            // Check if it points to our .dots/ directory
                            if let Ok(link_target) = target.canonicalize() {
                                if !link_target.starts_with(&dot_copy_dir) {
                                    crate::paths::backup_and_unlink(&target)?;
                                }
                            }
                        }
                        true
                    } else {
                        false
                    };

                    // Create symlink if needed
                    if should_symlink {
                        if let Err(e) = dot.symlink() {
                            tracing::error!(dot = %key, error = %e, "Failed to create symlink");
                        }
                    }
                }
            }
        }

        // Print summaries
        conflict_ctx.print_summary();
        review_ctx.print_summary();

        // Run posthooks
        for hook in &self.posthooks {
            match hook.run_capture() {
                Ok(result) => {
                    let action = crate::audit::Action::new(
                        AuditActionType::HookExecuted {
                            command: result.command.clone(),
                            hook_type: crate::audit::HookType::PostInstall,
                            exit_code: result.exit_code,
                        },
                        None,
                    );

                    let log_content = format_hook_logs(&result);
                    if !log_content.is_empty() {
                        let _ = storage.save_logs(&action.id, &log_content);
                    }

                    session.add_action(action);

                    if !result.success() {
                        tracing::error!(
                            command = %result.command,
                            exit_code = result.exit_code,
                            "Posthook failed"
                        );
                    }
                }
                Err(err) => {
                    tracing::error!(error = %err, "Failed to run posthook");
                }
            }
        }

        // Clean up orphaned symlinks
        let absolute_path_to_dot = self.dotfiles_absolute_path()?;
        let previous_state = BombadilState::read(absolute_path_to_dot.clone());
        let new_state = BombadilState::from(self);

        if let Ok(previous_state) = previous_state {
            let diff = previous_state.symlinks.difference(&new_state.symlinks);
            for orphan in diff {
                if orphan.exists() {
                    if let Ok(canonicalized) = orphan.canonicalize() {
                        // In review mode, prompt for orphan removal
                        let should_remove = if review {
                            let change = ReviewChange::delete(
                                None,
                                orphan.clone(),
                                fs::read_to_string(&canonicalized).unwrap_or_default(),
                            );
                            review_ctx.review(&change)?
                        } else {
                            true
                        };

                        if should_remove {
                            if let Ok(()) = unlink(orphan) {
                                if canonicalized.is_dir() {
                                    let _ = fs::remove_dir_all(&canonicalized);
                                } else {
                                    let _ = fs::remove_file(&canonicalized);
                                }

                                session.add_action(crate::audit::Action::new(
                                    AuditActionType::SymlinkRemove {
                                        target: orphan.clone(),
                                        was_pointing_to: canonicalized,
                                    },
                                    None,
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Save new state
        new_state.write()?;

        // Complete and save session
        session.complete();
        if !session.is_empty() {
            storage.save_session(&session)?;
        }

        Ok(session)
    }

    /// Helper to compute backup path for a target file
    fn backup_path(&self, target: &Path) -> PathBuf {
        let target_as_non_absolute = if target.is_absolute() {
            target.strip_prefix("/").unwrap_or(target)
        } else {
            target
        };
        self.path.join(".backups").join(target_as_non_absolute)
    }

    /// Helper to hash a file if it exists
    fn hash_file_if_exists(&self, path: &Path) -> String {
        if path.exists() {
            if let Ok(content) = fs::read(path) {
                return crate::audit::content_hash(&content);
            }
        }
        String::new()
    }

    /// Unlink dotfiles according to previous state
    pub fn uninstall(&self) -> Result<()> {
        let mut success_paths: Vec<&PathBuf> = Vec::new();
        let mut error_paths: Vec<&anyhow::Error> = Vec::new();

        // Remove symlink from previous state
        let path = self.dotfiles_absolute_path()?;
        let previous_state = BombadilState::read(path)?;
        let remove_result = previous_state.remove_targets();

        remove_result
            .iter()
            .for_each(|remove_result| match remove_result {
                Ok(path) => success_paths.push(path),
                Err(e) => error_paths.push(e),
            });

        if !success_paths.is_empty() {
            println!("{}", "Removed symlinks:".green());
            success_paths.iter().for_each(|path| {
                let path_string = format!("\t{:?}", path).green();
                println!("{}", path_string);
            });
        }

        if !error_paths.is_empty() {
            println!("{}", "Error removing symlinks:".red());
            error_paths.iter().for_each(|path| {
                let path_string = format!("\t{:?}", path).red();
                println!("{}", path_string);
            });
        }

        Ok(())
    }

    /// Watch dotfiles and automatically run link on changes
    pub async fn watch(profiles: Vec<String>) -> Result<()> {
        let mut bombadil = Bombadil::from_settings(Mode::Gpg)?;
        bombadil.enable_profiles(profiles.iter().map(String::as_str).collect())?;

        let mut init = InitConfig::default();
        init.on_error(PrintDebug(io::stderr()));

        let dotfiles_path = &bombadil.dotfiles_absolute_path()?;

        let mut runtime = RuntimeConfig::default();
        runtime.action_throttle(Duration::from_secs(1));

        // Ignore stuff like .git dirs
        let ignore_files = ignore_files::from_origin(dotfiles_path).await;
        let ignore_filter = IgnoreFilter::new(dotfiles_path, &ignore_files.0).await?;
        runtime.filterer(Arc::new(IgnoreFilterer(ignore_filter)));

        runtime.pathset([dotfiles_path]);

        runtime.on_action(move |action: WatchAction| {
            let mut b = Bombadil::from_settings(Mode::Gpg).expect("Failed to get settings");
            b.enable_profiles(profiles.iter().map(String::as_str).collect())
                .expect("Failed to enable profiles");

            async move {
                for event in action.events.iter() {
                    // Select only relevant events (creations, modifications, deletions)
                    if event.tags.iter().any(|t| {
                        matches!(t, &Tag::FileEventKind(FileEventKind::Create(_)))
                            || matches!(t, &Tag::FileEventKind(FileEventKind::Modify(_)))
                            || matches!(t, &Tag::FileEventKind(FileEventKind::Remove(_)))
                    }) {
                        println!("{}", "Detected changes, re-linking dots".green());
                        // Finally, install the dots like usual
                        b.install().map_err(|e| RuntimeError::Handler {
                            ctx: "bombadil install",
                            err: e.to_string(),
                        })?;
                        break;
                    }
                }

                let sigs = action
                    .events
                    .iter()
                    .flat_map(|event| event.signals())
                    .collect::<Vec<_>>();

                // Stop gently on Ctrl-C and kill -15
                if sigs
                    .iter()
                    .any(|sig| sig == &MainSignal::Interrupt || sig == &MainSignal::Terminate)
                {
                    action.outcome(Outcome::Exit);
                } else {
                    action.outcome(Outcome::if_running(Outcome::DoNothing, Outcome::Start));
                }

                Ok::<_, RuntimeError>(())
            }
        });

        let watchexec = Watchexec::new(init, runtime.clone())?;
        watchexec.main().await??;
        Ok(())
    }

    /// Add a gpg secret encrypted variable to the target variable file
    pub fn add_secret<S: AsRef<Path> + ?Sized>(
        &self,
        key: &str,
        value: &str,
        var_file: &S,
    ) -> Result<()> {
        if let Some(gpg) = &self.gpg {
            gpg.push_secret(key, value, var_file)
        } else {
            Err(anyhow!("No gpg_user_id in bombadil settings"))
        }
    }

    /// Pretty print current bombadil variables
    pub fn display_vars(&self) {
        self.vars
            .variables
            .iter()
            .for_each(|(key, value)| println!("{} = {}", key.red(), value))
    }

    /// Enable a dotfile profile by merging its settings with the default profile
    pub fn enable_profiles(&mut self, profile_keys: Vec<&str>) -> Result<()> {
        if profile_keys.is_empty() {
            return Ok(());
        }

        self.profile_enabled = profile_keys.iter().map(ToString::to_string).collect();

        let mut profiles: Vec<Profile> = profile_keys
            .iter()
            // unwrap here is safe cause allowed profile keys are checked by clap
            .map(|profile_key| self.profiles.get(&profile_key.to_string()).unwrap())
            .cloned()
            .collect();

        let sub_profiles: Vec<Profile> = profiles
            .iter()
            .flat_map(|profile| {
                profile
                    .extra_profiles
                    .iter()
                    .flat_map(|sub_profile| self.profiles.get(sub_profile))
                    .collect::<Vec<&Profile>>()
            })
            .cloned()
            .collect();

        profiles.extend(sub_profiles);

        // Merge profile dots
        for profile in profiles.iter() {
            profile.dots.iter().for_each(|(key, dot_override)| {
                // Dot exist let's override
                if let Some(dot) = self.dots.get_mut(key) {
                    if let Some(source) = &dot_override.source {
                        dot.source.clone_from(source)
                    }

                    if let Some(target) = &dot_override.target {
                        dot.target.clone_from(target)
                    }

                    if let Some(vars) = &dot_override.vars {
                        dot.vars.clone_from(vars)
                    }

                    if let Some(hard_copy_target) = &dot_override.hard_copy_target {
                        dot.hard_copy_target = Some(hard_copy_target.clone());
                    }

                    if let Some(hard_copy_permissions) = &dot_override.hard_copy_permissions {
                        dot.hard_copy_permissions = Some(*hard_copy_permissions);
                    }

                    if let (None, None, None, None, None) = (
                        &dot_override.source,
                        &dot_override.target,
                        &dot_override.vars,
                        &dot_override.hard_copy_target,
                        &dot_override.hard_copy_permissions,
                    ) {
                        let warning = format!(
                            "Skipping {}, no `source`, `target`, `vars`, `hard_copy_target`, or `hard_copy_permissions` to override",
                            key
                        )
                        .yellow();
                        eprintln!("{}", warning);
                    }
                // Nothing to override, let's create a new dot entry
                } else if let (Some(source), Some(target)) =
                    (&dot_override.source, &dot_override.target)
                {
                    let source = source.clone();
                    let target = target.clone();
                    let ignore = dot_override.ignore.clone();

                    self.dots.insert(
                        key.to_string(),
                        Dot {
                            source,
                            target,
                            ignore,
                            vars: Dot::default_vars(),
                            hard_copy_target: dot_override.hard_copy_target.clone(),
                            hard_copy_permissions: dot_override.hard_copy_permissions,
                        },
                    );
                } else {
                    if dot_override.source.is_none() {
                        let warning = format!("`source` field missing for {}", key).yellow();
                        eprintln!("{}", warning);
                    }

                    if dot_override.target.is_none() {
                        let warning = format!("`target` field missing for {}", key).yellow();
                        eprintln!("{}", warning);
                    }
                }
            });

            // Add profile vars
            let variables = Variables::from_paths(&self.path, &profile.vars)?;
            self.vars.extend(variables);
            // Add Profile pre hooks
            let prehooks = profile
                .prehooks
                .iter()
                .map(|command| command.as_ref())
                .map(|command| {
                    Hook::new(
                        self.path.clone(),
                        command,
                        profile.run_hooks_in_dotfiles_dir,
                    )
                })
                .collect::<Vec<Hook>>();
            self.prehooks.extend(prehooks);

            // Add profile post hooks
            let posthooks = profile
                .posthooks
                .iter()
                .map(|command| command.as_ref())
                .map(|command| {
                    Hook::new(
                        self.path.clone(),
                        command,
                        profile.run_hooks_in_dotfiles_dir,
                    )
                })
                .collect::<Vec<Hook>>();
            self.posthooks.extend(posthooks);
        }

        Ok(())
    }

    fn check_dotfile_dir(&self) -> Result<()> {
        if !self.path.exists() {
            return Err(anyhow!(
                "Dotfiles base path : {}, not found",
                self.path.display(),
            ));
        }

        if !self.path.is_dir() {
            let err = format!(
                "{} {:?} {}",
                "Provided dotfiles directory".red(),
                &self.path,
                "is not a directory".red()
            );
            return Err(anyhow!(err));
        }

        Ok(())
    }

    /// Load Bombadil settings from a `bombadil.toml`
    pub fn from_settings(mode: Mode) -> Result<Bombadil> {
        let config = Settings::get()?;
        let path = config.get_dotfiles_path()?;

        let run_hooks_in_dotfiles_dir = config.run_hooks_in_dotfiles_dir();

        let gpg = match mode {
            Mode::Gpg => config.gpg_user_id.map(|user_id| Gpg::new(&user_id)),
            Mode::NoGpg => None,
        };

        // Resolve variables from path
        let mut vars = Variables::from_paths(&path, &config.settings.vars)?;

        // Replace % reference with their ref value
        vars.resolve_ref();

        // Resolve hooks from settings
        let posthooks = config
            .settings
            .posthooks
            .iter()
            .map(|cmd| Hook::new(path.clone(), cmd, run_hooks_in_dotfiles_dir))
            .collect();

        let prehooks = config
            .settings
            .prehooks
            .iter()
            .map(|cmd| Hook::new(path.clone(), cmd, run_hooks_in_dotfiles_dir))
            .collect();
        let dots = config.settings.dots;
        let profiles = config.profiles;

        Ok(Self {
            path,
            dots,
            vars,
            prehooks,
            posthooks,
            profiles,
            gpg,
            profile_enabled: vec![],
        })
    }

    /// Pretty print metadata, possible values are Dots, PreHooks, PostHook, Path, Profiles, Vars, Secrets
    pub fn print_metadata(
        &self,
        metadata_type: MetadataType,
        writer: &mut impl Write,
    ) -> io::Result<()> {
        let rows = match metadata_type {
            MetadataType::Dots => self
                .dots
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{}: {} => {}",
                        k,
                        self.path.join(&v.source).display(),
                        v.target().unwrap_or_else(|_| v.target.clone()).display()
                    )
                })
                .collect(),
            MetadataType::PreHooks => self.prehooks.iter().map(|h| h.command.clone()).collect(),
            MetadataType::PostHooks => self.posthooks.iter().map(|h| h.command.clone()).collect(),
            MetadataType::Path => vec![self.path.display().to_string()],
            MetadataType::Profiles => {
                let mut profiles = vec!["default".to_string()];
                profiles.extend(self.profiles.keys().cloned());
                profiles
            }
            MetadataType::Vars => self
                .vars
                .variables
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect(),
            MetadataType::Secrets => self
                .vars
                .secrets
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect(),
        };

        if !rows.is_empty() {
            writer.write_all(rows.join("\n").as_bytes())?;
            writer.flush()?;
        }

        Ok(())
    }

    fn dotfiles_absolute_path(&self) -> Result<PathBuf> {
        dirs::home_dir()
            .ok_or_else(|| anyhow!("$HOME dir not found"))
            .map(|path| path.join(&self.path))
    }

    fn get_auto_ignored_files(&self, dot_key: &str) -> Vec<PathBuf> {
        let dot_origin = self.dots.get(dot_key);
        let origin_source = dot_origin.map(|dot| &dot.source);

        let mut ignored: Vec<PathBuf> = self
            .profiles
            .iter()
            .filter_map(|(_, profile)| profile.dots.get(dot_key))
            .filter(|dot| dot.vars.is_some())
            .filter_map(|dot| dot.resolve_var_path(origin_source))
            .collect();

        let _ = dot_origin.map(|dot| dot.resolve_var_path().map(|path| ignored.push(path)));

        ignored
    }
}

pub enum MetadataType {
    Dots,
    PreHooks,
    PostHooks,
    Path,
    Profiles,
    Vars,
    Secrets,
}

/// Format hook stdout/stderr for storage as logs.
fn format_hook_logs(result: &hook::HookResult) -> String {
    let mut log = String::new();

    if !result.stdout.is_empty() {
        log.push_str("=== stdout ===\n");
        for line in &result.stdout {
            log.push_str(line);
            log.push('\n');
        }
    }

    if !result.stderr.is_empty() {
        if !log.is_empty() {
            log.push('\n');
        }
        log.push_str("=== stderr ===\n");
        for line in &result.stderr {
            log.push_str(line);
            log.push('\n');
        }
    }

    log
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::unlink;
    use crate::Mode::NoGpg;
    use cmd_lib::{init_builtin_logger, run_cmd};
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use sealed_test::prelude::*;
    use speculoos::prelude::*;
    use std::ffi::OsStr;
    use std::fs::OpenOptions;
    use std::io::BufWriter;
    use std::{env, fs};

    fn setup(dotfiles: &str) {
        let home_dir = env::current_dir().unwrap().canonicalize().unwrap();
        env::set_var("HOME", home_dir);
        init_builtin_logger();
        run_cmd!(
            mkdir .config;
            tree -a;
        )
        .unwrap();

        Bombadil::link_self_config(Some(PathBuf::from(dotfiles))).unwrap();
    }

    #[sealed_test(files = ["tests/dotfiles_simple"], before = setup("dotfiles_simple"))]
    fn self_link_works() {
        let link = dirs::config_dir().unwrap().join(BOMBADIL_CONFIG);

        assert_that!(link).exists();
    }

    #[sealed_test(files = ["tests/dotfiles_simple"], before = setup("dotfiles_simple"))]
    fn install_single_file_works() -> Result<()> {
        Bombadil::from_settings(NoGpg)?.install()?;

        let target = fs::read_link(".config/template.css")?;
        let expected = env::current_dir()?.join("dotfiles_simple/.dots/template.css");

        assert_that!(target).is_equal_to(expected);

        let target = std::fs::read_to_string(target)?;

        assert_eq!(
            target,
            indoc! {
                ".class {
                    color: #de1f1f
                }
                "
            }
        );

        Ok(())
    }

    #[sealed_test(files = ["tests/dotfiles_create_dir"], before = setup("dotfiles_create_dir"))]
    fn install_creates_missing_directories() -> Result<()> {
        Bombadil::from_settings(NoGpg)?.install()?;

        let link = env::current_dir()?.join(".config/sub/dir/template.css");
        let target = std::fs::read_to_string(link)?;

        assert_eq!(
            target,
            indoc! {
                ".class {
                    color: #de1f1f
                }
                "
            }
        );

        Ok(())
    }

    #[sealed_test(files = ["tests/dotfiles_invalid_dot"], before = setup("dotfiles_invalid_dot"))]
    fn install_should_fail_and_continue() -> Result<()> {
        // Act
        Bombadil::from_settings(NoGpg)?.install()?;
        run_cmd!(tree -a;)?;
        // Assert
        assert_that!(PathBuf::from(".config/template.css")).exists();
        assert_that!(PathBuf::from(".config/invalid")).does_not_exist();
        Ok(())
    }

    #[sealed_test(files = ["tests/dotfiles_simple"], before = setup("dotfiles_simple"))]
    fn uninstall_works() -> Result<()> {
        Bombadil::link_self_config(Some(PathBuf::from("dotfiles_simple")))?;
        let bombadil = Bombadil::from_settings(NoGpg)?;

        bombadil.install()?;
        assert_that!(PathBuf::from(".config/template.css")).exists();

        bombadil.uninstall()?;
        assert_that!(PathBuf::from(".config/template.css")).does_not_exist();
        Ok(())
    }

    #[sealed_test(files = ["tests/dotfiles_simple"], before = setup("dotfiles_simple"))]
    fn posthook_ok() -> Result<()> {
        let bombadil = Bombadil::from_settings(NoGpg)?;

        // Act
        bombadil.install()?;

        // Assert
        assert_that!(PathBuf::from(".config/posthook/file").exists());

        Ok(())
    }

    #[sealed_test(files = ["tests/dotfiles_simple"], before = setup("dotfiles_simple"))]
    fn prehook_ok() -> Result<()> {
        let bombadil = Bombadil::from_settings(NoGpg)?;

        // Act
        bombadil.install()?;

        // Assert
        assert_that!(PathBuf::from(".config/prehook_file")).exists();

        Ok(())
    }

    #[sealed_test(files = ["tests/dotfiles_with_meta"], before = setup("dotfiles_with_meta"))]
    fn meta_var_works() -> Result<()> {
        // Act
        let bombadil = Bombadil::from_settings(NoGpg)?;

        // Assert
        assert_that!(bombadil.vars.variables.get("red"))
            .is_some()
            .is_equal_to(&"#FF0000".to_string());

        assert_that!(bombadil.vars.variables.get("black"))
            .is_some()
            .is_equal_to(&"#000000".to_string());

        assert_that!(bombadil.vars.variables.get("green"))
            .is_some()
            .is_equal_to(&"#008000".to_string());

        Ok(())
    }

    #[sealed_test(files = [ "tests/dotfiles_with_meta" ], before = setup("dotfiles_with_meta"))]
    fn should_print_metadata() -> Result<()> {
        let bombadil = Bombadil::from_settings(NoGpg)?;

        let mut content = vec![];
        let mut writer = BufWriter::new(&mut content);

        // Act
        bombadil.print_metadata(MetadataType::Vars, &mut writer)?;
        let result = String::from_utf8(writer.get_ref().to_vec())?;
        let result = result.as_str();

        // Assert
        assert_that!(result).contains("black: #000000");
        assert_that!(result).contains("green: #008000");
        assert_that!(result).contains("red: #FF0000");
        assert_that!(result).contains("meta_red: #FF0000");

        Ok(())
    }

    #[sealed_test(files = [ "tests/dotfiles_with_nested_dir" ], before = setup("dotfiles_with_nested_dir"))]
    fn should_get_auto_ignored_files() -> Result<()> {
        let bombadil = Bombadil::from_settings(NoGpg)?;

        let ignored_files = bombadil.get_auto_ignored_files("sub_dir");
        let ignored_files: Vec<&str> = ignored_files
            .iter()
            .filter_map(|path| path.file_name())
            .filter_map(OsStr::to_str)
            .collect();

        assert_that!(ignored_files).contains("vars.toml");

        Ok(())
    }

    #[sealed_test]
    fn should_unlink_dir() -> Result<()> {
        run_cmd!(
            mkdir "directory";
            ln -sf "directory" "linked_directory";
        )?;

        unlink("linked_directory")?;

        assert_that!(PathBuf::from("directory")).exists();
        assert_that!(PathBuf::from("linked_directory")).does_not_exist();

        Ok(())
    }

    #[sealed_test]
    fn should_unlink_file() -> Result<()> {
        run_cmd!(
            echo "Hello Tom" > "file";
            ln -sf file link;
        )?;

        unlink("link")?;

        assert_that!(PathBuf::from("file")).exists();
        assert_that!(PathBuf::from("link")).does_not_exist();

        Ok(())
    }

    #[sealed_test(files = ["tests/dot_files_with_imports"], before = setup("dot_files_with_imports"))]
    fn should_merge_import() -> Result<()> {
        // Arrange
        let bombadil = Bombadil::from_settings(NoGpg)?;

        assert_that!(bombadil.dots.get("maven")).is_some();
        assert_that!(bombadil.vars.variables.get("hello"))
            .is_some()
            .is_equal_to(&"world".to_string());

        assert_that!(bombadil.dots.get("relative_import/maven_relative")).is_some();
        assert_that!(
            bombadil
                .dots
                .get("relative_import/maven_relative")
                .unwrap()
                .source
        )
        .is_equal_to(PathBuf::from("relative_import/settings.xml"));
        Ok(())
    }

    #[sealed_test(files = ["tests/dotfiles_with_profile_context"], before = setup("dotfiles_with_profile_context"))]
    fn should_have_profile_context() -> Result<()> {
        // Arrange
        let mut bombadil = Bombadil::from_settings(NoGpg)?;
        bombadil.enable_profiles(vec!["fancy"])?;

        // Act
        bombadil.install()?;
        let target = fs::read_link(".config/template.css")?;
        let content = fs::read_to_string(target)?;

        // Assert
        assert_that!(content).is_equal_to(".class {color: #de1f1f}\n".to_string());

        Ok(())
    }

    #[sealed_test(files = ["tests/dotfiles_backup"], before = setup("dotfiles_backup"))]
    fn should_move_existing_file_to_backup() -> Result<()> {
        let bombadil = Bombadil::from_settings(NoGpg)?;

        let original_path = PathBuf::from(".config/deep/test/dir/template.css");
        assert_that!(original_path).does_not_exist();

        fs::create_dir_all(".config/deep/test/dir")?;
        OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(original_path.clone())?;

        let expected_backup_path = env::current_dir()?.join(format!(
            "dotfiles_backup/.backups{}/.config/deep/test/dir/template.css",
            env::current_dir()?.display()
        ));

        assert_that!(expected_backup_path).does_not_exist();

        bombadil.install()?;

        assert_that!(expected_backup_path).exists();

        let target = std::fs::read_to_string(original_path)?;
        assert_eq!(
            target,
            indoc! {
                ".class {
                    color: #de1f1f
                }
                "
            }
        );

        Ok(())
    }
}
