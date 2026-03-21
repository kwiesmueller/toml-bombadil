# Bombadil v4 Implementation Progress

## Status: Core Migration Complete - Cleanup Remaining

Last updated: 2025-02-08

## Quick Resume

**Test status**: 243 lib tests + 18 e2e tests pass, 0 warnings.

**To verify**: `cargo test --lib && cargo test --test e2e -- --test-threads=4`

**To rebuild e2e binary**: `cargo build --release`

**To explore interactively**: `bash examples/e2e/explore.sh --host-binary`

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
| 2025-02-08 | Dot is flat struct, not untagged enum | serde untagged enums silently drop unknown fields |

## Architecture: How v3 and v4 Coexist

The codebase currently runs two parallel systems:

### v4 (new, active for config loading)
- `src/config/` - Config types (`Config`, `Dot`, `Settings`, `Profile`), `LoaderRegistry`, import resolution
- `src/dots/` - Strategy-based installers (`Full`, `Patch`, `Inject`, `SemanticPatch`), template rendering
- `src/core/` - Error types (`BombadilError`)
- `src/audit/` - Action plan, session tracking, storage, revert

### v3 (legacy, still active for install execution)
- `src/settings/` - Old config types, `lazy_static SETTINGS` global
- `src/paths/mod.rs` - `DotPaths` trait using global `dotfile_dir()`
- `src/templating.rs` - Tera template rendering
- `src/lib.rs` - `plan_install()` / `execute_install_with_options()` with full audit integration

### The Bridge
`Bombadil::load()` (v4) builds the struct, loading config via v4 `LoaderRegistry`. It then converts v4 dots to v3 `settings::dots::Dot` via `v3_dot_from_v4()` for backwards compatibility. The CLI calls `Bombadil::load()` → `plan_install()` → `execute_install_with_options()`, which uses the v3 `DotPaths` trait and `templating::Variables` for the actual file operations.

**Critical**: The v3 `lazy_static SETTINGS` global also parses the config at startup. It tolerates v4 config because:
- `settings::dots::Dot` has `#[serde(default)]` on `source`/`target` (so v4 strategy-only dots parse OK)
- `Settings` has `#[serde(deny_unknown_fields)]` but only at the top level (v4 doesn't add top-level fields)
- The `config` crate ignores unknown fields in nested structs

## Completed Work

### Stream 4a: Fix Warnings
- [x] All compilation warnings eliminated
- Commit: `f183b66`

### Phase 1: Config Import Resolution
- [x] `load_config_resolved()` with recursive import merging via `LoaderRegistry`
- [x] Cycle detection, `merge_config()` matching v3 semantics
- [x] `resolve_dotfiles_dir()` with ~ expansion and relative-to-$HOME resolution
- Commit: `58f3503`

### Phase 2: Rewrite Bombadil Struct
- [x] `Bombadil::load()` using v4 config, `install_v4()` with strategy dispatch
- [x] `build_dot_context()`, `enable_profiles_v4()`, var loading, v3 compat layer
- Commit: `2dcae5d`

### Phase 3-4: Fix conflict.rs + Update CLI
- [x] CLI uses `Bombadil::load()` for Link, Unlink, AddSecret, Get commands
- [x] `get_dotfiles_path()` helper for Validate, Log, Inspect, Revert
- [x] `Conflict::detect_in()` with explicit `dotfiles_dir` parameter
- [x] `ImportPath` accepts both v3 `{path = "..."}` and v4 `"..."` formats
- [x] Fixed 3 pre-existing test failures
- Commits: `255fff9`, `1067bf2`

### Critical Fix: Dot Schema (serde untagged enum)
- [x] Merged `Dot::Simple` / `Dot::Full(DotFull)` into single flat `Dot` struct
- [x] This fixed: template rendering, ignore patterns, hard copies, strategy dispatch
- [x] Root cause: `#[serde(untagged)]` tries variants in order; `Simple{source,target}` matched anything with those two fields, silently dropping all other fields
- Commit: `7895635`

### Stream 3: Package Removal
- [x] `remove()` on all PackageManager backends
- [x] CLI `PackagesCommand::Remove` with `--yes` flag
- Commits: `5ccc71b`, `f7b3440`

### Stream 4b-d: Schema, Examples, E2E Tests
- [x] JSON schema generated at `schema/bombadil.schema.json` (652 lines)
- [x] `.taplo.toml` for Zed/Taplo editor integration
- [x] Example configs for all 4 strategies + hard copy + ignore + packages + profiles
- [x] E2E container infrastructure (`Containerfile`, `explore.sh`)
- [x] 18 automated e2e tests using host binary + runtime container (fast, ~2.5s total)
- Commits: `248d299`, `6120ddf`, `e9ee502`

## Remaining Work

### Phase 5-6: Delete v3 Modules (BLOCKED)

**Why blocked**: The primary install path (`plan_install` → `execute_install_with_options` in `lib.rs:649-1600`) uses:
- `DotPaths` trait (`paths/mod.rs`) for source/target/copy path resolution via `dotfile_dir()`
- `Variables::to_dot()` (`templating.rs`) for Tera rendering
- `settings::dots::Dot` for iteration and the `install()` method

This path has full audit integration (ActionPlan, Session, content snapshots, hook capture, conflict detection, review mode, orphan cleanup, state persistence) that `install_v4()` lacks.

**To unblock**, choose one of:
1. **Port audit integration to v4 flow** - Add ActionPlan/Session/storage to `install_v4()`. This is the clean approach but significant work (~500 lines of audit code to port).
2. **Make v3 install path use v4 resolution** - Replace `DotPaths` trait calls with direct path computation using `self.dotfiles_dir` instead of global `dotfile_dir()`. This is simpler but keeps the v3 code longer.
3. **Hybrid** - Keep v3 install for Full strategy (which has audit), use v4 installers for Patch/Inject/Semantic (which don't need audit yet since they're new).

**Files to delete once unblocked**:
- `src/settings/` → replaced by `src/config/`
- `src/paths/mod.rs` → v4 strategies handle paths
- `src/templating.rs` → replaced by `src/dots/render.rs`
- `src/error.rs` → replaced by `src/core/error.rs`
- `config` crate + `lazy_static` from Cargo.toml

**Note**: `settings/packages.rs` is also used by `PackageManager` - the package system needs to be updated to use v4 `config::Package` types.

### Uncommitted Files (from previous sessions)

There are ~50 uncommitted files in the working tree. These are:
- **Modified**: `Cargo.lock`, `Cargo.toml`, `src/hook.rs`, `src/paths/mod.rs`, `src/settings/mod.rs`, `src/settings/profiles.rs`
- **Deleted**: `src/dots.rs`, `src/git.rs`, `src/state.rs` (replaced by new modules)
- **Untracked**: All new v4 modules (`src/audit/`, `src/config/loader.rs`, `src/config/toml_loader.rs`, `src/dots/`, `src/templates/`), example files (`examples/`), `DESIGN.md`, `archive/`

These all compile and test cleanly. They should be committed as part of continuing work.

### Nice-to-Have Improvements

- [ ] Port audit integration to `install_v4()` so v3 install path can be removed
- [ ] Add e2e tests for Patch, Inject, and SemanticPatch strategies (currently only tested via unit tests)
- [ ] Profile inheritance test (`extra_profiles`) - the e2e test exists but profile inheritance through `enable_profiles_v4` needs validation
- [ ] `bombadil link` without `--force` (interactive conflict resolution) - works but not e2e tested
- [ ] `bombadil watch` command - exists but not tested with v4
- [ ] Clean up `src/bin/bombadil.rs` - still has some `Settings::get()` calls for packages commands

## Key Files Reference

| File | Role | Status |
|------|------|--------|
| `src/lib.rs` | Main orchestrator, Bombadil struct, both install paths | Active, hybrid v3+v4 |
| `src/config/mod.rs` | Config loading, import resolution | v4, complete |
| `src/config/schema.rs` | Config types (Dot, Settings, Profile, Package) | v4, complete |
| `src/config/loader.rs` | LoaderRegistry, ConfigLoader trait | v4, complete |
| `src/dots/strategy/` | Full, Patch, Inject, Semantic installers | v4, complete |
| `src/dots/render.rs` | Tera template rendering (v4) | v4, complete |
| `src/audit/` | Action plan, session, storage, revert | v4, complete |
| `src/bin/bombadil.rs` | CLI, uses Bombadil::load() | Migrated to v4 loading |
| `src/settings/` | Legacy v3 config types | v3, to be deleted |
| `src/paths/mod.rs` | Legacy DotPaths trait | v3, to be deleted |
| `src/templating.rs` | Legacy Tera rendering | v3, to be deleted |
| `src/packages/` | Package management | Active, uses v3 Package types |
