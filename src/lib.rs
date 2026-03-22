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
pub mod migrate;
pub mod sync;

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
            let content = fs::read_to_string(&state_path)
                .with_context(|| format!("reading state file {}", state_path.display()))?;
            let mut state: BombadilState = toml::from_str(&content)
                .map_err(|err| anyhow!("{} : {}", "Previous state format error".red(), err))?;
            state.path = state_path;
            Ok(state)
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

        let mut symlinks: HashSet<PathBuf> = current
            .dots
            .iter()
            .filter_map(|dot| dot.1.target().ok())
            .collect();

        // Also collect targets from v4 dots
        for dot in current.v4_dots.values() {
            if let Some(t) = dot.target.as_ref().map(|t| config::resolve_path(t)) {
                symlinks.insert(t);
            }
        }

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
                Err(ref err) if target.exists() => {
                    tracing::warn!(source = %source.display(), error = %err, "Template rendering failed, copying raw");
                    self.update_raw(source, target)
                }
                Err(err) => {
                    tracing::warn!(source = %source.display(), error = %err, "Template rendering failed, copying raw");
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
            let mut dot_copy = fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(target)?;
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
            let mut dot_copy = fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(target)?;

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
    // A list of dotfiles to link for this instance (v3)
    dots: HashMap<String, Dot>,
    // Variables for the tera template context (v3)
    vars: Variables,
    // Pre-hook commands, run before `bombadil-link`
    prehooks: Vec<Hook>,
    // Post-hook commands, run after `bombadil-link`
    posthooks: Vec<Hook>,
    // Available profiles (v3)
    profiles: HashMap<String, Profile>,
    // Profiles enabled for this instance
    profile_enabled: Vec<String>,
    // A GPG user id, linking to user encryption/decryption key via gnupg
    gpg: Option<Gpg>,

    // ── v4 fields ──────────────────────────────────────────────────────
    // v4 config (set when loaded via Bombadil::load())
    v4_config: Option<config::Config>,
    // v4 dots (merged base + profiles)
    v4_dots: HashMap<String, config::Dot>,
    // Flat variable map for v4 rendering
    v4_vars: HashMap<String, String>,
    // Decrypted GPG secrets for v4
    v4_secrets: HashMap<String, String>,
    // Absolute path to dotfiles directory
    dotfiles_dir: PathBuf,
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
                                    backup_location: None,
                                    source_path: None,
                                    source_before_hash: None,
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
                                    backup_location: None,
                                    source_path: None,
                                    source_before_hash: None,
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
                                            backup_location: None,
                                            source_path: None,
                                            source_before_hash: None,
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
                                        backup_location: None,
                                        source_path: None,
                                        source_before_hash: None,
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

    /// Enable a dotfile profile by merging its settings with the default profile
    pub fn enable_profiles(&mut self, profile_keys: Vec<&str>) -> Result<()> {
        if profile_keys.is_empty() {
            return Ok(());
        }

        self.profile_enabled = profile_keys.iter().map(ToString::to_string).collect();

        let mut profiles: Vec<Profile> = profile_keys
            .iter()
            // unwrap here is safe cause allowed profile keys are checked by clap
            .map(|profile_key| self.profiles.get(*profile_key).unwrap())
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
            dotfiles_dir: path.clone(),
            path,
            dots,
            vars,
            prehooks,
            posthooks,
            profiles,
            gpg,
            profile_enabled: vec![],
            v4_config: None,
            v4_dots: HashMap::new(),
            v4_vars: HashMap::new(),
            v4_secrets: HashMap::new(),
        })
    }

    /// Load Bombadil from v4 config system.
    ///
    /// Uses `config::load_config_resolved()` for import resolution and
    /// `LoaderRegistry` for format detection.
    pub fn load(mode: Mode) -> Result<Bombadil> {
        let config_path =
            config::config_path().map_err(|e| anyhow!("Failed to find config: {}", e))?;

        let v4_config = config::load_config_resolved(&config_path)
            .map_err(|e| anyhow!("Failed to load config: {}", e))?;

        let dotfiles_dir = config::resolve_dotfiles_dir(&v4_config, &config_path);

        let gpg = match mode {
            Mode::Gpg => v4_config.gpg_user_id.as_ref().map(|uid| Gpg::new(uid)),
            Mode::NoGpg => None,
        };

        let run_hooks_in_dotfiles_dir = v4_config.settings.run_hooks_in_dotfiles_dir;

        // Load variables from configured var file paths
        let mut v4_vars = HashMap::new();
        let mut v4_secrets = HashMap::new();
        for var_path in &v4_config.settings.vars {
            let full_path = dotfiles_dir.join(var_path);
            if full_path.exists() {
                match load_var_file(&full_path, gpg.as_ref()) {
                    Ok((vars, secrets)) => {
                        v4_vars.extend(vars);
                        v4_secrets.extend(secrets);
                    }
                    Err(e) => {
                        eprintln!(
                            "{} {:?} : {}",
                            "Could not load var file".yellow(),
                            full_path,
                            e
                        );
                    }
                }
            }
        }

        // Resolve % references in variables
        resolve_var_refs(&mut v4_vars);

        // Build hooks
        let prehooks = v4_config
            .settings
            .prehooks
            .iter()
            .map(|cmd| Hook::new(dotfiles_dir.clone(), cmd, run_hooks_in_dotfiles_dir))
            .collect();

        let posthooks = v4_config
            .settings
            .posthooks
            .iter()
            .map(|cmd| Hook::new(dotfiles_dir.clone(), cmd, run_hooks_in_dotfiles_dir))
            .collect();

        // Convert v4 dots for the struct
        let v4_dots = v4_config.settings.dots.clone();

        // Build v4 profiles map converted to v3 Profile for compatibility
        let profiles: HashMap<String, Profile> = v4_config
            .profiles
            .iter()
            .map(|(k, p)| {
                let v3_dots: HashMap<String, settings::dots::DotOverride> = p
                    .dots
                    .iter()
                    .map(|(dk, dov)| {
                        (
                            dk.clone(),
                            settings::dots::DotOverride {
                                source: dov.source.clone(),
                                target: dov.target.clone(),
                                ignore: dov.ignore.clone(),
                                vars: dov.vars.clone(),
                                hard_copy_target: dov.hard_copy_target.clone(),
                                hard_copy_permissions: dov.hard_copy_permissions,
                            },
                        )
                    })
                    .collect();

                (
                    k.clone(),
                    Profile {
                        dots: v3_dots,
                        packages: HashMap::new(),
                        package_tags: p.package_tags.clone(),
                        excluded_package_tags: p.package_exclude_tags.clone(),
                        extra_profiles: p.extra_profiles.clone(),
                        prehooks: p.prehooks.clone(),
                        posthooks: p.posthooks.clone(),
                        vars: p.vars.clone(),
                        run_hooks_in_dotfiles_dir: p.run_hooks_in_dotfiles_dir,
                    },
                )
            })
            .collect();

        // Also need v3-compatible vars and dots for backwards compat methods
        let vars = Variables {
            variables: v4_vars.clone(),
            secrets: v4_secrets.clone(),
        };

        // Build v3-compatible dots from v4 dots
        let v3_dots = v4_dots
            .iter()
            .filter_map(|(k, d)| v3_dot_from_v4(d).map(|dot| (k.clone(), dot)))
            .collect();

        Ok(Self {
            dotfiles_dir: dotfiles_dir.clone(),
            path: dotfiles_dir,
            dots: v3_dots,
            vars,
            prehooks,
            posthooks,
            profiles,
            gpg,
            profile_enabled: vec![],
            v4_config: Some(v4_config),
            v4_dots,
            v4_vars,
            v4_secrets,
        })
    }

    /// Get the v4 config, if loaded via `Bombadil::load()`.
    pub fn v4_config(&self) -> Option<&config::Config> {
        self.v4_config.as_ref()
    }

    /// Get the dotfiles directory path.
    pub fn dotfiles_path(&self) -> &Path {
        &self.dotfiles_dir
    }

    /// Install dotfiles using v4 strategy dispatch.
    ///
    /// Dispatches each dot to its configured strategy (Full, Patch, Inject, SemanticPatch).
    /// Falls back to Full strategy for simple dots.
    ///
    /// NOTE: Dead code — superseded by execute_install_with_options. Kept to avoid
    /// large-block deletion complexity; will be removed in a follow-up cleanup.
    #[allow(dead_code)]
    pub fn install_v4(&self, strategy: ConflictStrategy) -> Result<()> {
        self.check_dotfile_dir()?;

        // Run prehooks
        for hook in &self.prehooks {
            if let Err(err) = hook.run() {
                eprintln!("{}", err);
            }
        }

        let dots_dir = self.dotfiles_dir.join(".dots");
        fs::create_dir_all(&dots_dir)?;

        let conflict_ctx = ConflictContext::new(strategy);

        // Build tera context for rendering
        let context =
            dots::render::build_context(&self.v4_vars, &self.v4_secrets, &self.profile_enabled);

        // Install each v4 dot using strategy dispatch
        for (key, dot) in &self.v4_dots {
            let dot_strategy = dot.strategy.clone();

            // Build per-dot context with local vars
            let dot_context = self.build_dot_context(dot, &context);

            let installer: Box<dyn dots::DotInstaller> = match dot_strategy {
                config::DotStrategy::Full => Box::new(dots::strategy::full::FullInstaller),
                config::DotStrategy::Patch => Box::new(dots::strategy::patch::PatchInstaller),
                config::DotStrategy::Inject => Box::new(dots::strategy::inject::InjectInstaller),
                config::DotStrategy::SemanticPatch => {
                    let format = dot
                        .target
                        .as_ref()
                        .map(|t| dots::strategy::semantic::SemanticInstaller::detect_format(t))
                        .unwrap_or_default();
                    Box::new(dots::strategy::semantic::SemanticInstaller::new(format))
                }
            };

            match installer.install(dot, &self.dotfiles_dir, &dot_context) {
                Ok(result) => {
                    let (source_display, target_display) =
                        dot_display_paths(dot, &self.dotfiles_dir);
                    match result {
                        dots::InstallResult::Created => {
                            println!(
                                "Created - {} => {}",
                                source_display.blue(),
                                target_display.green()
                            );
                        }
                        dots::InstallResult::Updated => {
                            println!("{} => {}", source_display.blue(), target_display.yellow());
                        }
                        dots::InstallResult::Unchanged => {
                            println!("Unchanged - {} => {}", source_display, target_display);
                        }
                        dots::InstallResult::Ignored => {}
                        dots::InstallResult::Skipped => {}
                    }
                }
                Err(err) => {
                    eprintln!("Error installing dot '{}': {}", key, err);
                }
            }
        }

        // Handle hard copy targets as post-step
        for dot in self.v4_dots.values() {
            if let Some(hard_copy_target) = &dot.hard_copy_target {
                if let Some(source) = &dot.source {
                    let copy_path = self.dotfiles_dir.join(".dots").join(source);
                    if copy_path.exists() {
                        let target = config::resolve_path(hard_copy_target);
                        if let Some(parent) = target.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        let _ = fs::copy(&copy_path, &target);

                        if let Some(perms) = dot.hard_copy_permissions {
                            use std::os::unix::fs::PermissionsExt;
                            let _ = fs::set_permissions(&target, fs::Permissions::from_mode(perms));
                        }
                    }
                }
            }
        }

        // Print conflict summary
        conflict_ctx.print_summary();

        // Run posthooks
        for hook in &self.posthooks {
            if let Err(err) = hook.run() {
                eprintln!("Failed to run posthook: {}", err);
            }
        }

        // State tracking
        let absolute_path = &self.dotfiles_dir;
        let previous_state = BombadilState::read(absolute_path.to_owned());
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
                            tracing::info!(target = ?canonicalized, symlink = ?orphan, "Deleted orphaned symlink");
                        }
                    }
                }
            }
        }

        new_state.write()?;

        Ok(())
    }

    /// Build a per-dot tera::Context with local vars overlaid on the base context.
    /// NOTE: Dead code — only called by install_v4 which is itself dead code.
    #[allow(dead_code)]
    fn build_dot_context(&self, dot: &config::Dot, base_ctx: &tera::Context) -> tera::Context {
        let mut ctx = base_ctx.clone();

        // Load local vars if configured
        let local_vars_path = if let Some(v) = &dot.vars {
            Some(self.dotfiles_dir.join(v))
        } else if let Some(source) = &dot.source {
            // Check for vars.toml next to the source
            let source_path = self.dotfiles_dir.join(source);
            let vars_path = if source_path.is_dir() {
                source_path.join("vars.toml")
            } else {
                source_path
                    .parent()
                    .map(|p| p.join("vars.toml"))
                    .unwrap_or_default()
            };
            if vars_path.exists() {
                Some(vars_path)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(vars_path) = local_vars_path {
            if vars_path.exists() {
                if let Ok((local_vars, _)) = load_var_file(&vars_path, self.gpg.as_ref()) {
                    for (k, v) in local_vars {
                        ctx.insert(&k, &v);
                    }
                }
            }
        }

        ctx
    }

    /// Enable profiles using v4 config.
    pub fn enable_profiles_v4(&mut self, profile_keys: Vec<&str>) -> Result<()> {
        if profile_keys.is_empty() {
            return Ok(());
        }

        self.profile_enabled = profile_keys.iter().map(ToString::to_string).collect();

        if let Some(ref v4_config) = self.v4_config {
            for profile_key in &profile_keys {
                let profile = v4_config
                    .profiles
                    .get(*profile_key)
                    .ok_or_else(|| anyhow!("Profile '{}' not found", profile_key))?;

                // Merge dot overrides
                for (key, dot_override) in &profile.dots {
                    if let Some(existing) = self.v4_dots.get_mut(key) {
                        apply_v4_dot_override(existing, dot_override);
                    } else if let (Some(source), Some(target)) =
                        (&dot_override.source, &dot_override.target)
                    {
                        // Create new dot entry from override
                        self.v4_dots.insert(
                            key.clone(),
                            config::Dot {
                                source: Some(source.clone()),
                                target: Some(target.clone()),
                                ..Default::default()
                            },
                        );
                    }
                }

                // Add profile vars
                for var_path in &profile.vars {
                    let full_path = self.dotfiles_dir.join(var_path);
                    if full_path.exists() {
                        if let Ok((vars, secrets)) = load_var_file(&full_path, self.gpg.as_ref()) {
                            self.v4_vars.extend(vars.clone());
                            self.v4_secrets.extend(secrets.clone());
                            self.vars.variables.extend(vars);
                            self.vars.secrets.extend(secrets);
                        }
                    }
                }

                // Add profile hooks
                let run_in_dotfiles = profile.run_hooks_in_dotfiles_dir;
                let prehooks: Vec<Hook> = profile
                    .prehooks
                    .iter()
                    .map(|cmd| Hook::new(self.dotfiles_dir.clone(), cmd, run_in_dotfiles))
                    .collect();
                self.prehooks.extend(prehooks);

                let posthooks: Vec<Hook> = profile
                    .posthooks
                    .iter()
                    .map(|cmd| Hook::new(self.dotfiles_dir.clone(), cmd, run_in_dotfiles))
                    .collect();
                self.posthooks.extend(posthooks);

                // Sub-profile recursion is handled by v3 enable_profiles() call below
            }
        }

        // Also enable in v3 path for backwards compatibility
        self.enable_profiles(profile_keys)?;

        Ok(())
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

// ─────────────────────────────────────────────────────────────────────────────
// v4 helper functions
// ─────────────────────────────────────────────────────────────────────────────

/// Load a TOML variable file and separate plain vars from GPG-encrypted secrets.
///
/// Returns `(variables, secrets)` where secrets are values prefixed with `gpg:`
/// that have been decrypted using the provided GPG key.
fn load_var_file(
    path: &Path,
    gpg: Option<&Gpg>,
) -> Result<(HashMap<String, String>, HashMap<String, String>)> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading var file {}", path.display()))?;

    let variables: HashMap<String, String> =
        toml::from_str(&content).with_context(|| format!("parsing var file {}", path.display()))?;

    let mut secrets = HashMap::new();
    if let Some(gpg) = gpg {
        for (key, value) in &variables {
            if value.starts_with(gpg::GPG_PREFIX) {
                let encrypted = value.strip_prefix(gpg::GPG_PREFIX).unwrap();
                match gpg.decrypt_secret(encrypted) {
                    Ok(decrypted) => {
                        secrets.insert(key.clone(), decrypted);
                    }
                    Err(e) => {
                        eprintln!("{} {}: {}", "Failed to decrypt secret".yellow(), key, e);
                    }
                }
            }
        }
    }

    Ok((variables, secrets))
}

/// Resolve `%reference` patterns in variables.
///
/// If a variable value starts with `%`, it references another variable's value.
/// e.g., `red = "%meta_red"` will resolve to the value of `meta_red`.
fn resolve_var_refs(vars: &mut HashMap<String, String>) {
    let refs: Vec<(String, String)> = vars
        .iter()
        .filter(|(_, v)| v.starts_with('%'))
        .map(|(k, v)| (k.clone(), v[1..].to_string()))
        .collect();

    for (key, ref_key) in refs {
        if let Some(value) = vars.get(&ref_key).cloned() {
            vars.insert(key, value);
        } else {
            eprintln!(
                "{}",
                format!("Reference %{} not found in settings", &ref_key).yellow()
            );
        }
    }
}

/// Convert a v4 `config::Dot` to a v3 `settings::dots::Dot` for backwards compatibility.
fn v3_dot_from_v4(dot: &config::Dot) -> Option<Dot> {
    let source = dot.source.clone()?;
    let target = dot.target.clone()?;
    Some(Dot {
        source,
        target,
        ignore: dot.ignore.clone(),
        vars: dot.vars.clone().unwrap_or_else(Dot::default_vars),
        hard_copy_target: dot.hard_copy_target.clone(),
        hard_copy_permissions: dot.hard_copy_permissions,
    })
}

/// Apply a v4 profile dot override to an existing v4 dot.
fn apply_v4_dot_override(dot: &mut config::Dot, overrides: &config::DotOverride) {
    if let Some(new_source) = &overrides.source {
        dot.source = Some(new_source.clone());
    }
    if let Some(new_target) = &overrides.target {
        dot.target = Some(new_target.clone());
    }
    if let Some(new_strategy) = &overrides.strategy {
        dot.strategy = new_strategy.clone();
    }
    if !overrides.ignore.is_empty() {
        dot.ignore = overrides.ignore.clone();
    }
    if let Some(new_vars) = &overrides.vars {
        dot.vars = Some(new_vars.clone());
    }
    if let Some(new_hct) = &overrides.hard_copy_target {
        dot.hard_copy_target = Some(new_hct.clone());
    }
    if let Some(new_hcp) = &overrides.hard_copy_permissions {
        dot.hard_copy_permissions = Some(*new_hcp);
    }
}

/// Extract display-friendly source and target paths from a v4 dot.
fn dot_display_paths(dot: &config::Dot, dotfiles_dir: &Path) -> (String, String) {
    let source_display = dot
        .source
        .as_ref()
        .map(|s| dotfiles_dir.join(s).display().to_string())
        .unwrap_or_else(|| "<no source>".to_string());
    let target_display = dot
        .target
        .as_ref()
        .map(|t: &PathBuf| t.display().to_string())
        .unwrap_or_else(|| "<no target>".to_string());
    (source_display, target_display)
}

// v3 integration tests removed — superseded by tests/e2e.rs (container-based e2e).
// Fixtures that were only used by those tests (tests/dotfiles_*) have been deleted.

// v3 integration tests have been removed.
// All CUJs are covered by the container-based e2e suite in tests/e2e.rs.
