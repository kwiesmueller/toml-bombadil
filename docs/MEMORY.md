# Bombadil Consolidation Memory

## Implementation Status (as of 2026-03-21)

**Phase 1 complete** — commit `9af0802`
**Phase 2 complete** — commit `cdf5013` (audit recording + hook execution in execute_plan)
**Phase 3 complete** — commit `cdf5013` (real PackageManager integration)
**Feature parity + migrate command** — commit `18c8329`

Recent commits on `main`:
- `cdf5013`: Phase 2+3 — audit recording, hook execution, real PackageManager integration
- `18c8329`: v4 feature parity + bombadil migrate command

**Test status**: 246 lib tests + 18 e2e tests + 1 doc-test = 265 total, all passing.

## Project Structure
- `src/config/` - v4 config types with JsonSchema (schema.rs, loader.rs, toml_loader.rs)
- `src/core/` - v4 error types with miette (error.rs, mod.rs)
- `src/dots/` - v4 dot strategies (strategy/full.rs, patch.rs, inject.rs, semantic/)
- `src/audit/` - Action-based auditing system (full audit system)
- `src/sync/` - NEW: SyncEngine, SyncPlan, build_sync_plan(), execute_plan()
- `src/packages/managers/` - v4 package manager trait implementations
- `src/packages/drift.rs` - v4 drift detection
- `archive/` - Legacy code (dots_legacy.rs, state.rs) moved here for reference
- `docs/plan.md` - Full consolidation plan with all design decisions
- `docs/example-dotfiles/` - Reference dots.toml examples for all features

## New Config Model (dots.toml)

Each `dots.toml` declares one named dot. File-map model (not single source/target):

```toml
[dot]
name = "zsh"
depends_on = ["other-dot-name"]
tags = ["gui"]          # empty = always included
prehooks = [...]
posthooks = [...]

[dot.files]
"zshrc"    = "~/.zshrc"                                         # Simple
"themes/"  = { target = "~/.config/zsh/themes/", ignore = ["*.bak"] }  # Extended

[dot.packages.zsh]
install.dnf = "zsh"     # optional; key name used as fallback
posthooks = ["chsh -s $(which zsh)"]

[dot.profiles.work]
vars = ["profiles/work.toml"]
files = { "work.zshrc" = "~/.zshrc" }
```

Key types in `src/config/schema.rs`:
- `DotFile { dot: DotDefinition }` — top-level of dots.toml
- `DotDefinition` — name, files: IndexMap<String, FileTarget>, packages, depends_on, tags, hooks, profiles
- `FileTarget` — untagged enum: `Simple(String)` | `Extended(FileTargetOptions)` — confirmed working with TOML
- `DotPackage` — optional install methods, tags, hooks
- `DotProfileOverride` — vars, files, hooks overrides
- `Profile.active_tags: Vec<String>` — tags declared active for this profile

## SyncEngine (src/sync/)

- `SyncOptions { profile, dry_run, extra_tags, only_dots, only_packages, prune_packages }`
- `build_sync_plan(config_path, options)` → `SyncPlan`:
  1. Load bombadil.toml + discover all dots.toml
  2. Resolve active profile → active tag set
  3. Filter dots by tags
  4. Topological sort on `depends_on` (cycles → ConfigInvalid error)
  5. Per-file action planning (Create/Update/Unchanged/Backup)
  6. Skip propagation (DependencyUnavailable/DependencySkipped)
  7. Interleave with per-dot and global hooks
- `execute_plan(plan)` → runs hooks (Hook::run_capture), creates symlinks/copies, records audit Session
- CLI: `bombadil bombadil-sync [--dry-run] [--tags ...] [--only-dots] [--only-packages] [--prune-packages]`
  (temporary name; will become `bombadil sync` when old `link` is deleted in Phase 4)

## Profile & Machine Model

- All machine configs are named profiles committed to repo
- `bombadil init <path> --profile <name>` → links bombadil.toml + writes `.active_profile` (gitignored)
- `.active_profile` contains just the profile name, persists per-device
- `bombadil sync` reads `.active_profile` automatically (no --profile flag needed)
- `Profile.active_tags` declares tags before platform auto-detection (for fresh installs)
- Var load order (later wins): global vars → platform vars → profile vars → dot vars

## Key Types (existing, unchanged)

- `DotInstaller` trait: `install(&self, dot: &Dot, dotfiles_dir: &Path, vars: &tera::Context)`
- `PackageManager` trait: `name()`, `is_available()`, `is_installed()`, `install()`, `list_installed()`
- `BombadilError::Io { context: String, source: io::Error }` — field is `context`, not `message`
- `tera::Context::into_json()` returns `Value` directly (not `Result`)

## Remaining Phases

- **Phase 4**: Delete v3 modules (src/settings/, src/paths/, src/templating.rs, src/error.rs,
  bulk of src/lib.rs). Rename BombadilSync → Sync in CLI.
  Add `bombadil init` command with --profile flag + .active_profile writing.
- **Phase 5**: E2E test coverage for all CUJs (full table in docs/plan.md)
- **Phase 6**: Code quality pass (clippy clean, no TODOs, schema up to date)

## CLI Commands (current + new)

Existing (from prior work):
- `bombadil schema [config|patch|semantic-patch]`
- `bombadil drift [--show-extra]`
- `bombadil log / inspect / revert` (audit commands)

New (Phase 1):
- `bombadil bombadil-sync` — new sync engine (will become `bombadil sync` in Phase 4)

Planned (Phase 4):
- `bombadil init <path> --profile <name>` — replaces `install`, writes .active_profile
- `bombadil new <path> [--target] [--name]` — scaffold new dot with boilerplate dots.toml
- `bombadil sync` — final name for the new sync command

## New Schema Types (commit 18c8329)

- `PkgManagerInstall` enum in `config/schema.rs`: `Simple(String)` | `Extended { package, repo, repo_url, gpg_key }`
  - Replaces `Option<String>` for `dnf/apt/brew/pacman` in `InstallMethods`
  - `.package_name()` → the install name; `.repo_file()` → optional repo file path
- `FileTargetOptions` now has `hard_copy_target: Option<String>` + `hard_copy_permissions: Option<u32>`
- `PlannedFile` has `hard_copy_target: Option<PathBuf>` + `hard_copy_permissions: Option<u32>`
- `PlannedPackage` has `repo_file: Option<PathBuf>`
- `plan_packages_for_dot()` now takes `dotfiles_dir: &Path` (third arg)
- Manager selection fixed: only picks a manager if the package explicitly configures it

## migrate command

- `bombadil migrate <dotfiles-dir> [--output <out-dir>] [--dry-run]`
- Never modifies files in `<dotfiles-dir>`; writes `dots.toml` into `<out-dir>`
- Handles: dot import files, top-level package files, hard_copy_target, extended pkg manager configs
- Warns for: binary/git/source installs, disabled packages, repo_url/gpg_key configs

## Important Code Locations

- `src/lib.rs:2600` — `v3_dot_from_v4()` bridge (to delete in Phase 4)
- `src/lib.rs:398` — `v4_dots: HashMap<String, config::Dot>` (to delete in Phase 4)
- `src/bin/bombadil.rs:452` — BombadilSync arm; Migrate arm added
- `src/sync/plan.rs:280` — `build_sync_plan()` entry point
- `src/config/mod.rs:194` — `discover_dot_files()` entry point
- `src/migrate/mod.rs` — migration logic (migrate(), generate_dots_toml(), etc.)
