# Bombadil v4 Implementation Progress

## Status: Migration In Progress

## Decision Log

| Date | Decision | Rationale |
|------|----------|-----------|
| 2024-02-04 | Keep TOML for now | Best JSON Schema tooling for autocomplete |
| 2024-02-04 | Use miette for errors | Rich error reporting with source context |
| 2024-02-04 | Use tracing for logging | Structured logging |
| 2024-02-04 | Use schemars for schema | Generate JSON Schema from Rust types |
| 2025-02-07 | Config format pluggable via LoaderRegistry | All loading goes through ConfigLoader trait |
| 2025-02-07 | Package removal is explicit, never during sync | Safety: `packages remove` with confirmation |
| 2025-02-07 | Replace `config` crate with `toml::from_str` | One fewer dependency, simpler |

## Completed Work

### Stream 4a: Fix Warnings
- [x] Removed unused imports in toml_patch.rs, inject.rs, packages.rs
- [x] Fixed unused variable in patch.rs
- [x] Removed dead `default_vars()` from DotVar trait
- [x] Suppressed thiserror v2 false-positive warnings
- Commit: `f183b66`

### Phase 1: Config Import Resolution
- [x] Rewrote `src/config/mod.rs` with import resolution via LoaderRegistry
- [x] `load_config_resolved()` with recursive import merging
- [x] Cycle detection via visited set
- [x] `merge_config()` matching v3 merge semantics
- [x] `resolve_dotfiles_dir()` with ~ and relative path support
- [x] 18 tests passing
- Commit: `58f3503`

### Phase 2: Rewrite Bombadil Struct
- [x] Added v4 fields to Bombadil struct (v4_config, v4_dots, v4_vars, v4_secrets, dotfiles_dir)
- [x] Implemented `Bombadil::load()` using v4 config system
- [x] Implemented `install_v4()` with strategy dispatch (Full/Patch/Inject/SemanticPatch)
- [x] Implemented `build_dot_context()` for per-dot local variable loading
- [x] Implemented `enable_profiles_v4()` for v4 profile merging
- [x] Helper functions: `load_var_file()`, `resolve_var_refs()`, `v3_dot_from_v4()`, `apply_v4_dot_override()`, `dot_display_paths()`
- [x] v3<->v4 compatibility layer for incremental migration
- [x] Zero warnings, 240 tests passing
- Commit: `2dcae5d`

### State Tracking Fix
- [x] Replaced `::config::Config` deserialization with `toml::from_str` in `BombadilState::read()`
- [x] Updated `BombadilState::from()` to collect targets from both v3 and v4 dots
- Commit: `516c847`

## In Progress

### Phase 3-4: Fix conflict.rs + Update CLI (Agent)
- [ ] Change `Conflict::detect()` to accept `dotfiles_dir: &Path` parameter
- [ ] Remove `use crate::settings::dotfile_dir` from conflict.rs
- [ ] Update CLI to support v4 config profile loading

### Stream 3: Package Removal (Agent)
- [ ] Implement `remove()` on PackageManager backends
- [ ] Wire up `PackagesCommand::Remove` in CLI with confirmation prompt
- [ ] Add `--yes` flag for non-interactive removal

## Pending

### Phase 5-6: Delete v3 Modules
- [ ] Remove `src/settings/` (replaced by `src/config/`)
- [ ] Remove `src/paths/mod.rs` (v4 strategies handle paths)
- [ ] Remove `src/templating.rs` (replaced by `src/dots/render.rs`)
- [ ] Remove `src/error.rs` (replaced by `src/core/error.rs`)
- [ ] Remove `config` crate and `lazy_static` from Cargo.toml

### Stream 4b-d: Schema, Examples, Tests
- [ ] Generate `schema/bombadil.schema.json` via `bombadil schema config`
- [ ] Create `.taplo.toml` for Zed integration
- [ ] Add example configs for all 4 strategies
- [ ] E2E tests for patch, inject, semantic strategies
