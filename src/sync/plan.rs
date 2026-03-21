//! Sync plan construction.
//!
//! `build_sync_plan()` takes the resolved config + discovered dots and produces
//! an ordered `SyncPlan` ready for execution or display. Dependency ordering
//! (topological sort on `depends_on`) and tag filtering are resolved here,
//! never at execution time.

use crate::config::{
    discover_dot_files, resolve_dotfiles_dir, load_config_resolved, DotDefinition, DotFile,
    FileTarget, Profile,
};
use crate::core::{BombadilError, Result};
use indexmap::IndexMap;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// Sync options provided by the caller (typically from CLI flags).
#[derive(Debug, Clone, Default)]
pub struct SyncOptions {
    /// Active profile name. When `None`, no profile is active.
    pub profile: Option<String>,

    /// Show plan without executing.
    pub dry_run: bool,

    /// Override active tags (unioned with profile-declared tags).
    pub extra_tags: Vec<String>,

    /// Remove packages installed but not in config.
    pub prune_packages: bool,

    /// Skip package operations.
    pub only_dots: bool,

    /// Skip dot operations.
    pub only_packages: bool,
}

/// An ordered plan of items to execute during `bombadil sync`.
///
/// Items are ordered to respect `depends_on` constraints. Global hooks are
/// embedded as `SyncItem::Hook` at the front and back.
#[derive(Debug, Default)]
pub struct SyncPlan {
    pub items: Vec<SyncItem>,
    /// Combined active tag set (profile tags + extra_tags + auto-detected).
    pub active_tags: HashSet<String>,
    /// Dotfiles root directory.
    pub dotfiles_dir: PathBuf,
}

impl SyncPlan {
    /// Number of items that will actually execute (not skipped).
    pub fn active_count(&self) -> usize {
        self.items
            .iter()
            .filter(|i| !matches!(i, SyncItem::Dot(d) if matches!(d.action, DotAction::Skip { .. })))
            .count()
    }
}

/// One item in the sync plan.
#[derive(Debug)]
pub enum SyncItem {
    Hook(PlannedHook),
    Dot(PlannedDot),
    Package(PlannedPackage),
}

/// A hook to run at a specific point in the plan.
#[derive(Debug, Clone)]
pub struct PlannedHook {
    pub command: String,
    /// Owning dot/package name; `None` = global hook.
    pub owner: Option<String>,
    pub phase: HookPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookPhase {
    Pre,
    Post,
}

/// A dot to install, with per-file planned operations.
#[derive(Debug)]
pub struct PlannedDot {
    /// Stable logical name (from `dot.name`).
    pub name: String,
    /// Source directory path relative to dotfiles root (used for display).
    /// Displayed as `//terminal/zsh` in output.
    pub namespace: String,
    /// Resolved file operations for this dot.
    pub files: Vec<PlannedFile>,
    /// Overall dot-level action (derived from file actions).
    pub action: DotAction,
    /// Packages that must be installed before this dot's files.
    pub packages: Vec<PlannedPackage>,
}

/// A single file mapping to be applied.
#[derive(Debug)]
pub struct PlannedFile {
    pub source: PathBuf,
    pub target: PathBuf,
    /// `true` = regular file copy; `false` = symlink (default).
    pub copy: bool,
    pub action: DotAction,
}

/// Action to perform for a dot or file.
#[derive(Debug, Clone)]
pub enum DotAction {
    /// Target doesn't exist — create symlink/copy.
    Create,
    /// Target exists and differs — update.
    Update,
    /// Target already matches — no-op.
    Unchanged,
    /// Target exists and is not managed — backup then overwrite.
    Backup { original: PathBuf },
    /// Skipped — reason explains why.
    Skip { reason: SkipReason },
}

impl DotAction {
    pub fn is_skip(&self) -> bool {
        matches!(self, DotAction::Skip { .. })
    }

    pub fn indicator(&self) -> &'static str {
        match self {
            DotAction::Create => "+",
            DotAction::Update => "~",
            DotAction::Unchanged => "=",
            DotAction::Backup { .. } => "!",
            DotAction::Skip { .. } => "⊘",
        }
    }
}

/// Reason a dot was skipped.
#[derive(Debug, Clone)]
pub enum SkipReason {
    /// A direct dependency is not in the plan (excluded by tags/profile).
    DependencyUnavailable { dependency: String },
    /// A direct dependency was itself skipped; carries the reason chain.
    DependencySkipped {
        dependency: String,
        cause: Box<SkipReason>,
    },
    /// Tags didn't match the active tag set.
    TagMismatch,
}

impl SkipReason {
    pub fn root_cause(&self) -> &str {
        match self {
            SkipReason::DependencyUnavailable { dependency } => dependency,
            SkipReason::DependencySkipped { cause, .. } => cause.root_cause(),
            SkipReason::TagMismatch => "(tag mismatch)",
        }
    }
}

/// A package to install.
#[derive(Debug)]
pub struct PlannedPackage {
    pub name: String,
    /// Optional manager-specific install name override.
    pub install_name: Option<String>,
    pub action: PackageAction,
    pub prehooks: Vec<String>,
    pub posthooks: Vec<String>,
}

/// Action to perform for a package.
#[derive(Debug, Clone)]
pub enum PackageAction {
    Install,
    AlreadyInstalled,
    /// No compatible manager available.
    Skip { reason: String },
    /// Remove (when `--prune-packages` is active).
    Prune,
}

// ─────────────────────────────────────────────────────────────────────────────
// Plan builder
// ─────────────────────────────────────────────────────────────────────────────

/// Build a `SyncPlan` from the resolved config and discovered dots.
///
/// Steps:
/// 1. Load `bombadil.toml` via `config_path`.
/// 2. Discover all `dots.toml` files under `dotfiles_dir`.
/// 3. Resolve the active profile and build the active tag set.
/// 4. Filter dots by tags.
/// 5. Topological sort by `depends_on` (cycles → error).
/// 6. For each dot, compute per-file actions.
/// 7. Second pass: propagate skip reasons through dependency chains.
/// 8. Wrap with global pre/post hooks.
pub fn build_sync_plan(config_path: &Path, options: &SyncOptions) -> Result<SyncPlan> {
    let config = load_config_resolved(config_path)?;
    let dotfiles_dir = resolve_dotfiles_dir(&config, config_path);

    // Resolve active profile
    let active_profile: Option<&Profile> = options
        .profile
        .as_ref()
        .and_then(|name| config.profiles.get(name.as_str()));

    // Build active tag set: profile.active_tags + extra_tags
    let mut active_tags: HashSet<String> = HashSet::new();
    if let Some(profile) = active_profile {
        active_tags.extend(profile.active_tags.iter().cloned());
        // Include tags from inherited profiles
        for parent_name in &profile.inherits {
            if let Some(parent) = config.profiles.get(parent_name.as_str()) {
                active_tags.extend(parent.active_tags.iter().cloned());
            }
        }
    }
    active_tags.extend(options.extra_tags.iter().cloned());

    debug!(active_tags = ?active_tags, "resolved active tag set");

    // Discover and filter dots
    let discovered = discover_dot_files(&dotfiles_dir);
    debug!(count = discovered.len(), "discovered dots.toml files");

    // Map name → (namespace, DotDefinition) for dependency resolution
    // Also apply profile overrides to each dot
    let mut name_to_dot: IndexMap<String, (String, DotDefinition)> = IndexMap::new();

    for (rel_dir, dot_file) in &discovered {
        let namespace = rel_dir.to_string_lossy().to_string();
        let mut dot_def = dot_file.dot.clone();

        // Default name to directory name if not explicitly set
        if dot_def.name.is_none() {
            let default_name = rel_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| namespace.clone());
            warn!(
                namespace = %namespace,
                default_name = %default_name,
                "dots.toml missing 'name' field, defaulting to directory name"
            );
            dot_def.name = Some(default_name);
        }

        // Apply active profile overrides from dots.toml [dot.profiles.X]
        if let Some(profile_name) = &options.profile {
            if let Some(override_) = dot_def.profiles.get(profile_name.as_str()) {
                let override_ = override_.clone();
                // Merge var files (profile vars take precedence by being appended)
                dot_def.vars.extend(override_.vars);
                // Merge file mappings (profile additions/replacements)
                for (src, target) in override_.files {
                    dot_def.files.insert(src, target);
                }
                dot_def.prehooks.extend(override_.prehooks);
                dot_def.posthooks.extend(override_.posthooks);
            }
        }

        let name = dot_def.name.clone().unwrap(); // set above
        if name_to_dot.contains_key(&name) {
            warn!(name = %name, "duplicate dot name detected, later entry wins");
        }
        name_to_dot.insert(name, (namespace, dot_def));
    }

    // Filter by tags
    let name_to_dot: IndexMap<String, (String, DotDefinition)> = name_to_dot
        .into_iter()
        .filter(|(_, (_, dot_def))| is_dot_included(dot_def, &active_tags))
        .collect();

    debug!(
        included = name_to_dot.len(),
        "dots after tag filtering"
    );

    // Topological sort
    let sorted_names = topological_sort(&name_to_dot)?;

    // Build PlannedDot items
    let mut planned_dots: Vec<PlannedDot> = Vec::new();
    // Track which names are in plan and their skip status for dependency propagation
    let mut dot_actions: HashMap<String, DotAction> = HashMap::new();

    // Also track names that were discovered but filtered out by tags (for DependencyUnavailable)
    let all_discovered_names: HashSet<String> = {
        let mut set = HashSet::new();
        for (_, dot_file) in &discovered {
            if let Some(name) = &dot_file.dot.name {
                set.insert(name.clone());
            } else {
                // use dir name as fallback for tracking
                if let Some(name) = dot_file.dot.name.clone() {
                    set.insert(name);
                }
            }
        }
        set
    };

    for name in &sorted_names {
        let (namespace, dot_def) = &name_to_dot[name];
        let abs_dir = dotfiles_dir.join(namespace);

        // Check dependencies: if any dep is missing from plan or was skipped, skip this dot
        let skip_reason = check_dependency_skip(name, dot_def, &dot_actions, &name_to_dot);

        let action = if let Some(reason) = skip_reason {
            DotAction::Skip { reason }
        } else if options.only_packages {
            // --only-packages: skip all dots
            DotAction::Skip {
                reason: SkipReason::TagMismatch, // reusing for "excluded by flag"
            }
        } else {
            // Compute file-level actions
            DotAction::Create // placeholder; files determine actual action
        };

        let files = if action.is_skip() {
            Vec::new()
        } else {
            plan_files_for_dot(dot_def, &abs_dir, &dotfiles_dir)
        };

        // Derive dot-level action from file actions (or use Skip)
        let dot_action = if action.is_skip() {
            action.clone()
        } else {
            derive_dot_action(&files)
        };

        // Plan packages for this dot
        let packages = plan_packages_for_dot(dot_def, options);

        let planned = PlannedDot {
            name: name.clone(),
            namespace: namespace.clone(),
            files,
            action: dot_action.clone(),
            packages,
        };

        dot_actions.insert(name.clone(), dot_action);
        planned_dots.push(planned);
    }

    // Build the final item list with hooks interleaved
    let mut items: Vec<SyncItem> = Vec::new();

    // Global prehooks (from settings + active profile)
    let global_prehooks = collect_global_hooks(&config, active_profile, HookPhase::Pre);
    for cmd in global_prehooks {
        items.push(SyncItem::Hook(PlannedHook {
            command: cmd,
            owner: None,
            phase: HookPhase::Pre,
        }));
    }

    // Dot items with per-dot hooks
    for planned_dot in planned_dots {
        // Per-dot prehooks (only if dot is not skipped)
        if !planned_dot.action.is_skip() {
            let (_, dot_def) = &name_to_dot[&planned_dot.name];
            for cmd in &dot_def.prehooks {
                items.push(SyncItem::Hook(PlannedHook {
                    command: cmd.clone(),
                    owner: Some(planned_dot.name.clone()),
                    phase: HookPhase::Pre,
                }));
            }
        }

        // Packages (before files)
        if !options.only_dots {
            for pkg in &planned_dot.packages {
                for cmd in &pkg.prehooks {
                    items.push(SyncItem::Hook(PlannedHook {
                        command: cmd.clone(),
                        owner: Some(pkg.name.clone()),
                        phase: HookPhase::Pre,
                    }));
                }
                items.push(SyncItem::Package(PlannedPackage {
                    name: pkg.name.clone(),
                    install_name: pkg.install_name.clone(),
                    action: pkg.action.clone(),
                    prehooks: pkg.prehooks.clone(),
                    posthooks: pkg.posthooks.clone(),
                }));
                for cmd in &pkg.posthooks {
                    items.push(SyncItem::Hook(PlannedHook {
                        command: cmd.clone(),
                        owner: Some(pkg.name.clone()),
                        phase: HookPhase::Post,
                    }));
                }
            }
        }

        let name = planned_dot.name.clone();
        let dot_def_prehooks_len = {
            let (_, dot_def) = &name_to_dot[&name];
            dot_def.prehooks.len()
        };
        let is_skip = planned_dot.action.is_skip();
        items.push(SyncItem::Dot(planned_dot));

        // Per-dot posthooks
        if !is_skip {
            let (_, dot_def) = &name_to_dot[&name];
            for cmd in &dot_def.posthooks {
                items.push(SyncItem::Hook(PlannedHook {
                    command: cmd.clone(),
                    owner: Some(name.clone()),
                    phase: HookPhase::Post,
                }));
            }
        }
        let _ = dot_def_prehooks_len; // suppress unused warning
    }

    // Global posthooks
    let global_posthooks = collect_global_hooks(&config, active_profile, HookPhase::Post);
    for cmd in global_posthooks {
        items.push(SyncItem::Hook(PlannedHook {
            command: cmd,
            owner: None,
            phase: HookPhase::Post,
        }));
    }

    Ok(SyncPlan {
        items,
        active_tags,
        dotfiles_dir,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Whether a dot should be included given the active tag set.
///
/// - Empty `tags` = always include.
/// - Non-empty `tags` = include if at least one tag is in `active_tags`.
/// - If `active_tags` is empty (no profile), include all dots that have no tags.
fn is_dot_included(dot_def: &DotDefinition, active_tags: &HashSet<String>) -> bool {
    if dot_def.tags.is_empty() {
        return true;
    }
    // If no active tags are set, only include untagged dots
    if active_tags.is_empty() {
        return false;
    }
    dot_def.tags.iter().any(|t| active_tags.contains(t))
}

/// Topological sort of dot names by `depends_on`.
///
/// Returns names in dependency order (dependencies first).
/// Detects cycles and returns a `BombadilError::ConfigInvalid`.
fn topological_sort(
    name_to_dot: &IndexMap<String, (String, DotDefinition)>,
) -> Result<Vec<String>> {
    // Kahn's algorithm
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut dependents: HashMap<String, Vec<String>> = HashMap::new(); // dep → [nodes that depend on it]

    for (name, (_, dot_def)) in name_to_dot {
        in_degree.entry(name.clone()).or_insert(0);
        for dep in &dot_def.depends_on {
            if !name_to_dot.contains_key(dep.as_str()) {
                // Dependency is not in the filtered plan — will be caught as skip later
                debug!(name = %name, dep = %dep, "dependency not in plan");
                continue;
            }
            *in_degree.entry(name.clone()).or_insert(0) += 1;
            dependents
                .entry(dep.clone())
                .or_default()
                .push(name.clone());
        }
    }

    let mut queue: VecDeque<String> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(name, _)| name.clone())
        .collect();

    // Sort for determinism
    let mut queue_sorted: Vec<String> = queue.drain(..).collect();
    queue_sorted.sort();
    queue = queue_sorted.into();

    let mut sorted = Vec::new();

    while let Some(name) = queue.pop_front() {
        sorted.push(name.clone());
        if let Some(dependents_list) = dependents.get(&name) {
            let mut newly_ready: Vec<String> = Vec::new();
            for dep_name in dependents_list {
                if let Some(deg) = in_degree.get_mut(dep_name) {
                    *deg -= 1;
                    if *deg == 0 {
                        newly_ready.push(dep_name.clone());
                    }
                }
            }
            newly_ready.sort();
            for name in newly_ready {
                queue.push_back(name);
            }
        }
    }

    if sorted.len() != name_to_dot.len() {
        let cycle_nodes: Vec<_> = name_to_dot
            .keys()
            .filter(|n| !sorted.contains(n))
            .cloned()
            .collect();
        return Err(BombadilError::ConfigInvalid {
            message: format!(
                "Cycle detected in dot dependencies: {}",
                cycle_nodes.join(", ")
            ),
            help: Some(
                "Check 'depends_on' fields — they must form a directed acyclic graph".to_string(),
            ),
        });
    }

    Ok(sorted)
}

/// Check if a dot should be skipped due to missing/skipped dependencies.
fn check_dependency_skip(
    name: &str,
    dot_def: &DotDefinition,
    dot_actions: &HashMap<String, DotAction>,
    name_to_dot: &IndexMap<String, (String, DotDefinition)>,
) -> Option<SkipReason> {
    for dep in &dot_def.depends_on {
        match dot_actions.get(dep.as_str()) {
            None if !name_to_dot.contains_key(dep.as_str()) => {
                // Dependency not in plan at all (filtered out by tags)
                return Some(SkipReason::DependencyUnavailable {
                    dependency: dep.clone(),
                });
            }
            Some(DotAction::Skip { reason }) => {
                return Some(SkipReason::DependencySkipped {
                    dependency: dep.clone(),
                    cause: Box::new(reason.clone()),
                });
            }
            _ => {}
        }
    }
    None
}

/// Plan file operations for a dot given its definition and source directory.
fn plan_files_for_dot(
    dot_def: &DotDefinition,
    abs_dot_dir: &Path,
    dotfiles_dir: &Path,
) -> Vec<PlannedFile> {
    let _ = dotfiles_dir; // reserved for future /path anchor resolution
    let mut files = Vec::new();

    for (source_str, target) in &dot_def.files {
        let source = abs_dot_dir.join(source_str);
        let target_str = target.target_path();
        let target_path = crate::config::resolve_path(std::path::Path::new(target_str));
        let is_copy = target.is_copy();

        // Determine action based on target state
        let action = if !target_path.exists() {
            DotAction::Create
        } else if target_path.is_symlink() {
            // Check if it already points to the right place
            let link_dest = std::fs::read_link(&target_path).ok();
            if link_dest.as_deref() == Some(&source) {
                DotAction::Unchanged
            } else {
                DotAction::Update
            }
        } else {
            // Unmanaged file exists at target
            DotAction::Backup {
                original: target_path.clone(),
            }
        };

        files.push(PlannedFile {
            source,
            target: target_path,
            copy: is_copy,
            action,
        });
    }

    files
}

/// Derive the overall dot action from its file actions.
///
/// Precedence: Backup > Update > Create > Unchanged.
fn derive_dot_action(files: &[PlannedFile]) -> DotAction {
    if files.is_empty() {
        return DotAction::Unchanged;
    }

    let mut has_backup = false;
    let mut has_update = false;
    let mut has_create = false;

    for file in files {
        match &file.action {
            DotAction::Backup { .. } => has_backup = true,
            DotAction::Update => has_update = true,
            DotAction::Create => has_create = true,
            _ => {}
        }
    }

    if has_backup {
        DotAction::Backup {
            original: PathBuf::new(), // placeholder for dot-level summary
        }
    } else if has_update {
        DotAction::Update
    } else if has_create {
        DotAction::Create
    } else {
        DotAction::Unchanged
    }
}

/// Plan packages from a dot definition.
fn plan_packages_for_dot(dot_def: &DotDefinition, options: &SyncOptions) -> Vec<PlannedPackage> {
    dot_def
        .packages
        .iter()
        .map(|(name, pkg)| {
            // Stub: all packages are planned as Install for now.
            // Phase 3 will integrate actual PackageManager::is_installed() checks.
            PlannedPackage {
                name: name.clone(),
                install_name: pkg
                    .install
                    .as_ref()
                    .and_then(|m| m.dnf.clone().or(m.brew.clone()).or(m.apt.clone())),
                action: PackageAction::Install,
                prehooks: pkg.prehooks.clone(),
                posthooks: pkg.posthooks.clone(),
            }
        })
        .collect()
}

/// Collect global hooks from settings and active profile.
fn collect_global_hooks(
    config: &crate::config::Config,
    active_profile: Option<&Profile>,
    phase: HookPhase,
) -> Vec<String> {
    let mut hooks = match phase {
        HookPhase::Pre => config.settings.prehooks.clone(),
        HookPhase::Post => config.settings.posthooks.clone(),
    };

    if let Some(profile) = active_profile {
        let profile_hooks = match phase {
            HookPhase::Pre => &profile.prehooks,
            HookPhase::Post => &profile.posthooks,
        };
        hooks.extend(profile_hooks.iter().cloned());
    }

    hooks
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_bombadil_toml(dir: &Path, content: &str) {
        fs::write(dir.join("bombadil.toml"), content).unwrap();
    }

    fn write_dots_toml(dir: &Path, rel_path: &str, content: &str) {
        let path = dir.join(rel_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn plan_simple_dot() {
        let dir = TempDir::new().unwrap();
        write_bombadil_toml(dir.path(), "");
        write_dots_toml(
            dir.path(),
            "zsh/dots.toml",
            r#"[dot]
name = "zsh"

[dot.files]
"zshrc" = "~/.zshrc"
"#,
        );
        // Create source file
        fs::write(dir.path().join("zsh/zshrc"), "# zsh config").unwrap();

        let config_path = dir.path().join("bombadil.toml");
        let plan = build_sync_plan(&config_path, &SyncOptions::default()).unwrap();

        let dots: Vec<_> = plan
            .items
            .iter()
            .filter_map(|i| match i {
                SyncItem::Dot(d) => Some(d),
                _ => None,
            })
            .collect();

        assert_eq!(dots.len(), 1);
        assert_eq!(dots[0].name, "zsh");
    }

    #[test]
    fn plan_depends_on_ordering() {
        let dir = TempDir::new().unwrap();
        write_bombadil_toml(dir.path(), "");

        write_dots_toml(
            dir.path(),
            "zsh/dots.toml",
            r#"[dot]
name = "zsh"
[dot.files]
"zshrc" = "~/.zshrc"
"#,
        );
        fs::write(dir.path().join("zsh/zshrc"), "").unwrap();

        write_dots_toml(
            dir.path(),
            "zsh/plugins/dots.toml",
            r#"[dot]
name = "zsh-plugins"
depends_on = ["zsh"]
[dot.files]
"plugins.zsh" = "~/.config/zsh/plugins.zsh"
"#,
        );
        fs::write(dir.path().join("zsh/plugins/plugins.zsh"), "").unwrap();

        let config_path = dir.path().join("bombadil.toml");
        let plan = build_sync_plan(&config_path, &SyncOptions::default()).unwrap();

        let dot_names: Vec<_> = plan
            .items
            .iter()
            .filter_map(|i| match i {
                SyncItem::Dot(d) => Some(d.name.as_str()),
                _ => None,
            })
            .collect();

        // zsh must come before zsh-plugins
        let zsh_pos = dot_names.iter().position(|n| *n == "zsh").unwrap();
        let plugins_pos = dot_names.iter().position(|n| *n == "zsh-plugins").unwrap();
        assert!(zsh_pos < plugins_pos, "zsh must precede zsh-plugins");
    }

    #[test]
    fn plan_depends_on_cycle_errors() {
        let dir = TempDir::new().unwrap();
        write_bombadil_toml(dir.path(), "");

        write_dots_toml(
            dir.path(),
            "a/dots.toml",
            r#"[dot]
name = "a"
depends_on = ["b"]
[dot.files]
"f" = "~/.a"
"#,
        );
        fs::write(dir.path().join("a/f"), "").unwrap();

        write_dots_toml(
            dir.path(),
            "b/dots.toml",
            r#"[dot]
name = "b"
depends_on = ["a"]
[dot.files]
"f" = "~/.b"
"#,
        );
        fs::write(dir.path().join("b/f"), "").unwrap();

        let config_path = dir.path().join("bombadil.toml");
        let result = build_sync_plan(&config_path, &SyncOptions::default());
        assert!(result.is_err(), "cycle should be an error");
    }

    #[test]
    fn plan_tag_filtered_dot_skips_dependents() {
        let dir = TempDir::new().unwrap();
        write_bombadil_toml(dir.path(), "");

        // gui dot — excluded when no active tags
        write_dots_toml(
            dir.path(),
            "kitty/dots.toml",
            r#"[dot]
name = "kitty"
tags = ["gui"]
[dot.files]
"kitty.conf" = "~/.config/kitty/kitty.conf"
"#,
        );
        fs::write(dir.path().join("kitty/kitty.conf"), "").unwrap();

        // kitty-theme depends on kitty — should be skipped with DependencyUnavailable
        write_dots_toml(
            dir.path(),
            "kitty-theme/dots.toml",
            r#"[dot]
name = "kitty-theme"
depends_on = ["kitty"]
[dot.files]
"theme.conf" = "~/.config/kitty/theme.conf"
"#,
        );
        fs::write(dir.path().join("kitty-theme/theme.conf"), "").unwrap();

        let config_path = dir.path().join("bombadil.toml");
        // No active tags → gui dot excluded
        let plan = build_sync_plan(&config_path, &SyncOptions::default()).unwrap();

        // kitty should not appear (filtered by tags)
        // kitty-theme should appear but as Skip { DependencyUnavailable }
        let kitty_theme: Option<&PlannedDot> = plan.items.iter().find_map(|i| match i {
            SyncItem::Dot(d) if d.name == "kitty-theme" => Some(d),
            _ => None,
        });

        assert!(
            kitty_theme.is_some(),
            "kitty-theme should be in plan (as skipped)"
        );
        assert!(
            kitty_theme.unwrap().action.is_skip(),
            "kitty-theme should be skipped"
        );
    }
}
