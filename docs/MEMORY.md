# Bombadil Consolidation Memory

## Implementation Status (as of 2026-03-21)

**Phase 1 complete** — committed on `main` branch.
- Commit `f30f90f`: consolidation baseline (all prior v4 work committed)
- Commit `9af0802`: Phase 1 — new config schema, dots.toml discovery, SyncEngine

**Test status**: 254 lib tests + 18 e2e tests + 1 doc-test = 273 total, all passing.

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
- `execute_plan(plan)` → creates symlinks/copies, stubs for hooks+packages
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

- **Phase 2**: Integrate audit recording + real hook execution in execute_plan()
- **Phase 3**: Real PackageManager integration (replace stubs in plan_packages_for_dot)
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

## Important Code Locations

- `src/lib.rs:2600` — `v3_dot_from_v4()` bridge (to delete in Phase 4)
- `src/lib.rs:398` — `v4_dots: HashMap<String, config::Dot>` (to delete in Phase 4)
- `src/bin/bombadil.rs:392` — main match block (add BombadilSync arm is already there)
- `src/sync/plan.rs:280` — `build_sync_plan()` entry point
- `src/config/mod.rs:194` — `discover_dot_files()` entry point
