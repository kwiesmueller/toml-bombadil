//! Sync orchestrator — builds and executes a `SyncPlan`.
//!
//! # Usage
//!
//! ```no_run
//! use toml_bombadil::sync::{SyncEngine, SyncOptions};
//!
//! let engine = SyncEngine::new("/path/to/bombadil.toml".as_ref());
//! let plan = engine.plan(&SyncOptions::default()).unwrap();
//! let session = engine.execute(plan).unwrap();
//! ```

pub mod plan;

pub use plan::{
    build_sync_plan, DotAction, HookPhase, PackageAction, PlannedDot, PlannedFile, PlannedHook,
    PlannedPackage, SkipReason, SyncItem, SyncOptions, SyncPlan,
};

use crate::core::Result;
use std::path::{Path, PathBuf};
use tracing::instrument;

/// Orchestrates the full sync workflow for a bombadil installation.
pub struct SyncEngine {
    /// Path to the `bombadil.toml` config file.
    config_path: PathBuf,
}

impl SyncEngine {
    pub fn new(config_path: &Path) -> Self {
        Self {
            config_path: config_path.to_path_buf(),
        }
    }

    /// Build a sync plan without executing it.
    ///
    /// This is safe to call for `--dry-run` display.
    #[instrument(skip(self, options))]
    pub fn plan(&self, options: &SyncOptions) -> Result<SyncPlan> {
        build_sync_plan(&self.config_path, options)
    }

    /// Execute a previously-built plan.
    ///
    /// Returns `Ok(())` when the plan executed without fatal errors.
    /// Individual item failures are recorded in the audit log but do not
    /// abort the run unless `--fail-fast` (future option) is set.
    #[instrument(skip(self, plan))]
    pub fn execute(&self, plan: SyncPlan) -> Result<()> {
        execute_plan(plan)
    }
}

/// Execute a `SyncPlan`, applying all items in order.
///
/// Each item produces an audit `Action`. The completed set of actions is
/// persisted as a `Session` at the end of the run.
#[instrument(skip(plan))]
pub fn execute_plan(plan: SyncPlan) -> Result<()> {
    use crate::audit::{Action, ActionType, AuditStorage, HookType, Session};
    use crate::core::BombadilError;
    use crate::hook::Hook;
    use crate::sync::plan::{DotAction, HookPhase, SyncItem};
    use tracing::{info, warn};

    info!(
        dotfiles_dir = %plan.dotfiles_dir.display(),
        items = plan.items.len(),
        "starting sync"
    );

    let storage = AuditStorage::new(&plan.dotfiles_dir);
    storage.init().map_err(|e| BombadilError::Io {
        context: format!("failed to init audit storage: {e}"),
        source: std::io::Error::other(e.to_string()),
    })?;

    let mut session = Session::new("bombadil sync", vec![]);

    for item in &plan.items {
        match item {
            SyncItem::Hook(hook) => {
                let owner = hook.owner.as_deref().unwrap_or("global");
                info!(
                    command = %hook.command,
                    owner = %owner,
                    phase = ?hook.phase,
                    "running hook"
                );

                let h = Hook::new(plan.dotfiles_dir.clone(), &hook.command, true);
                let result = h.run_capture().unwrap_or_else(|e| {
                    warn!(command = %hook.command, error = %e, "hook failed to spawn");
                    crate::hook::HookResult {
                        command: hook.command.clone(),
                        exit_code: -1,
                        stdout: vec![],
                        stderr: vec![],
                    }
                });

                if result.exit_code != 0 {
                    warn!(
                        command = %hook.command,
                        exit_code = result.exit_code,
                        "hook exited with non-zero status"
                    );
                }

                let hook_type = match hook.phase {
                    HookPhase::Pre => HookType::PreInstall,
                    HookPhase::Post => HookType::PostInstall,
                };
                let action = Action::new(
                    ActionType::HookExecuted {
                        command: hook.command.clone(),
                        hook_type,
                        exit_code: result.exit_code,
                    },
                    hook.owner.clone(),
                );
                session.add_action(action);
            }
            SyncItem::Dot(dot) => {
                if let DotAction::Skip { reason } = &dot.action {
                    info!(
                        name = %dot.name,
                        root_cause = %reason.root_cause(),
                        "⊘ skipping dot"
                    );
                    continue;
                }

                info!(
                    name = %dot.name,
                    namespace = %dot.namespace,
                    action = %dot.action.indicator(),
                    files = dot.files.len(),
                    "applying dot"
                );

                for file in &dot.files {
                    let action_types = apply_file(file)?;
                    for at in action_types {
                        let action = Action::new(at, Some(dot.name.clone()));
                        session.add_action(action);
                    }
                }
            }
            SyncItem::Package(pkg) => match &pkg.action {
                PackageAction::AlreadyInstalled => {
                    info!(name = %pkg.name, "package already installed, skipping");
                }
                PackageAction::Skip { reason } => {
                    info!(name = %pkg.name, reason = %reason, "⊘ skipping package");
                }
                PackageAction::Install => {
                    info!(
                        name = %pkg.name,
                        install_name = %pkg.install_name,
                        manager = ?pkg.manager_name,
                        "installing package"
                    );
                    install_package(pkg);
                }
                PackageAction::Prune => {
                    info!(name = %pkg.name, "package prune not yet implemented");
                }
            },
        }
    }

    session.complete();
    if !session.is_empty() {
        storage
            .save_session(&session)
            .map_err(|e| BombadilError::Io {
                context: format!("failed to save audit session: {e}"),
                source: std::io::Error::other(e.to_string()),
            })?;
        info!(
            session_id = %session.id,
            actions = session.actions.len(),
            "sync session saved"
        );
    }

    info!("sync complete");
    Ok(())
}

/// Install a package using the manager recorded in the plan.
///
/// Failures are logged as warnings; a single package failure does not abort the run.
fn install_package(pkg: &PlannedPackage) {
    use crate::packages::managers::{
        apt::Apt, brew::Brew, cargo::Cargo, dnf::Dnf, flatpak::Flatpak, pacman::Pacman,
        PackageManager as PkgMgr,
    };
    use tracing::{info, warn};

    // Handle repo file installation first (for extended package manager configs).
    if let Some(repo_file) = &pkg.repo_file {
        install_repo_file(repo_file, pkg.manager_name.as_deref());
    }

    match pkg.manager_name.as_deref() {
        Some("go") => {
            info!(module = %pkg.install_name, "installing go package");
            let status = std::process::Command::new("go")
                .arg("install")
                .arg(&pkg.install_name)
                .status();
            match status {
                Ok(s) if s.success() => {}
                Ok(s) => warn!(
                    name = %pkg.name,
                    module = %pkg.install_name,
                    exit_code = ?s.code(),
                    "go install failed"
                ),
                Err(e) => warn!(
                    name = %pkg.name,
                    module = %pkg.install_name,
                    error = %e,
                    "failed to spawn go install"
                ),
            }
        }
        Some("binary") => {
            warn!(
                name = %pkg.name,
                install_name = %pkg.install_name,
                "binary download install not yet implemented — install manually"
            );
        }
        Some(mgr_name) => {
            let mgr: Option<Box<dyn PkgMgr>> = match mgr_name {
                "dnf" => Some(Box::new(Dnf)),
                "apt" => Some(Box::new(Apt)),
                "brew" => Some(Box::new(Brew)),
                "pacman" => Some(Box::new(Pacman)),
                "cargo" => Some(Box::new(Cargo)),
                "flatpak" => Some(Box::new(Flatpak)),
                _ => None,
            };
            match mgr {
                Some(m) => {
                    if let Err(e) = m.install(&pkg.install_name) {
                        warn!(
                            name = %pkg.name,
                            install_name = %pkg.install_name,
                            error = %e,
                            "package install failed"
                        );
                    }
                }
                None => {
                    warn!(
                        name = %pkg.name,
                        manager = %mgr_name,
                        "unknown package manager, skipping install"
                    );
                }
            }
        }
        None => {
            warn!(name = %pkg.name, "no package manager available, skipping install");
        }
    }
}

/// Install a repository file before a package (for extended package manager configs).
///
/// Copies the repo file to the manager-appropriate system directory.
/// Failures are logged as warnings — the package install will likely also fail,
/// but we don't abort the entire run.
fn install_repo_file(repo_file: &std::path::Path, manager: Option<&str>) {
    use std::fs;
    use tracing::{info, warn};

    let dest_dir = match manager {
        Some("dnf") | Some("rpm") => std::path::Path::new("/etc/yum.repos.d"),
        Some("apt") => std::path::Path::new("/etc/apt/sources.list.d"),
        _ => {
            warn!(
                repo_file = %repo_file.display(),
                "cannot determine repo destination for manager {:?}",
                manager
            );
            return;
        }
    };

    if !repo_file.exists() {
        warn!(repo_file = %repo_file.display(), "repo file not found, skipping");
        return;
    }

    let filename = repo_file.file_name().unwrap_or_default();
    let dest = dest_dir.join(filename);

    info!(
        src = %repo_file.display(),
        dest = %dest.display(),
        "installing repo file"
    );

    if let Err(e) = fs::copy(repo_file, &dest) {
        warn!(
            src = %repo_file.display(),
            dest = %dest.display(),
            error = %e,
            "failed to install repo file (may need sudo)"
        );
    }
}

/// Apply a single file mapping (symlink or copy) and return the audit actions produced.
fn apply_file(file: &PlannedFile) -> Result<Vec<crate::audit::ActionType>> {
    use crate::audit::ActionType;
    use crate::core::BombadilError;
    use std::fs;
    use tracing::{debug, warn};

    if !file.source.exists() {
        warn!(source = %file.source.display(), "source file not found, skipping");
        return Ok(vec![]);
    }

    // Ensure target parent directory exists
    if let Some(parent) = file.target.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| BombadilError::Io {
                context: format!("failed to create directory {}", parent.display()),
                source: e,
            })?;
        }
    }

    let mut actions: Vec<ActionType> = Vec::new();

    match &file.action {
        DotAction::Backup { original } => {
            // Back up unmanaged file before overwriting
            let backup = original.with_extension("bombadil.bak");
            debug!(
                original = %original.display(),
                backup = %backup.display(),
                "backing up existing file"
            );
            if original.exists() {
                let orig_content = fs::read(original).unwrap_or_default();
                let orig_hash = crate::audit::sha256_hash(&orig_content);
                fs::rename(original, &backup).map_err(|e| BombadilError::Io {
                    context: format!("failed to backup {}", original.display()),
                    source: e,
                })?;
                actions.push(ActionType::Backup {
                    original: original.clone(),
                    backup_location: backup,
                    content_hash: orig_hash,
                });
            }
            create_link_or_copy(file)?;
            actions.push(make_link_action(file));
            apply_hard_copy(file, &mut actions)?;
        }
        DotAction::Update => {
            // Remove old symlink/file before relinking
            if file.target.exists() || file.target.is_symlink() {
                fs::remove_file(&file.target).map_err(|e| BombadilError::Io {
                    context: format!("failed to remove {}", file.target.display()),
                    source: e,
                })?;
            }
            create_link_or_copy(file)?;
            actions.push(make_link_action(file));
            apply_hard_copy(file, &mut actions)?;
        }
        DotAction::Create => {
            create_link_or_copy(file)?;
            actions.push(make_link_action(file));
            apply_hard_copy(file, &mut actions)?;
        }
        DotAction::Unchanged => {
            debug!(target = %file.target.display(), "unchanged, skipping");
        }
        DotAction::Skip { .. } => {}
    }

    Ok(actions)
}

/// Apply the optional hard copy declared in `file.hard_copy_target`.
///
/// Creates a real file at `hard_copy_target` by copying from `source`, then
/// optionally applies `hard_copy_permissions`. Records a `FileCreate` action.
fn apply_hard_copy(file: &PlannedFile, actions: &mut Vec<crate::audit::ActionType>) -> Result<()> {
    use crate::audit::ActionType;
    use crate::core::BombadilError;
    use std::fs;
    use tracing::debug;

    let Some(ref hct) = file.hard_copy_target else {
        return Ok(());
    };

    if let Some(parent) = hct.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| BombadilError::Io {
                context: format!("failed to create directory {}", parent.display()),
                source: e,
            })?;
        }
    }

    debug!(
        source = %file.source.display(),
        hard_copy = %hct.display(),
        "creating hard copy"
    );

    fs::copy(&file.source, hct).map_err(|e| BombadilError::Io {
        context: format!(
            "failed to hard copy {} → {}",
            file.source.display(),
            hct.display()
        ),
        source: e,
    })?;

    if let Some(mode) = file.hard_copy_permissions {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(hct, std::fs::Permissions::from_mode(mode)).map_err(|e| {
                BombadilError::Io {
                    context: format!("failed to set permissions on {}", hct.display()),
                    source: e,
                }
            })?;
        }
    }

    let content = fs::read(&file.source).unwrap_or_default();
    let hash = crate::audit::sha256_hash(&content);
    actions.push(ActionType::FileCreate {
        target: hct.clone(),
        source: file.source.clone(),
        content_hash: hash,
    });

    Ok(())
}

/// Build the audit `ActionType` for a completed link-or-copy operation.
fn make_link_action(file: &PlannedFile) -> crate::audit::ActionType {
    use crate::audit::ActionType;

    if file.copy {
        let content = std::fs::read(&file.source).unwrap_or_default();
        let hash = crate::audit::sha256_hash(&content);
        ActionType::FileCreate {
            target: file.target.clone(),
            source: file.source.clone(),
            content_hash: hash,
        }
    } else {
        ActionType::SymlinkCreate {
            source: file.source.clone(),
            target: file.target.clone(),
        }
    }
}

/// Create a symlink or file copy for a planned file.
fn create_link_or_copy(file: &PlannedFile) -> Result<()> {
    use crate::core::BombadilError;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs as unix_fs;

    if file.copy {
        fs::copy(&file.source, &file.target).map_err(|e| BombadilError::Io {
            context: format!(
                "failed to copy {} → {}",
                file.source.display(),
                file.target.display()
            ),
            source: e,
        })?;
    } else {
        #[cfg(unix)]
        unix_fs::symlink(&file.source, &file.target).map_err(|e| BombadilError::Io {
            context: format!(
                "failed to symlink {} → {}",
                file.source.display(),
                file.target.display()
            ),
            source: e,
        })?;

        #[cfg(not(unix))]
        return Err(BombadilError::ConfigInvalid {
            context: "symlinks require a Unix system".to_string(),
            help: None,
        });
    }
    Ok(())
}
