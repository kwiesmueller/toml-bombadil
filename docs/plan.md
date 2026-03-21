# Bombadil Consolidation Plan

## Goal

Produce a clean, consolidated codebase with no v3/v4 distinction. All features ship in one MVP:
audited sync, conflict resolution, package management, drift detection, profiles, and secrets.
All CUJs verified by sandboxed e2e tests. All legacy code deleted.

---

## Principles

- One command activates everything: `bombadil sync`
- Every file and package operation is an auditable `Action` in a `Session`
- No global state, no `lazy_static`, no bridge/compat code
- Modules are small, single-purpose, and tested independently
- Errors use `miette` for rich diagnostics; no `anyhow` going forward
- `tracing` for structured debug/info output; no `println!` in library code

---

## Path Resolution

All path fields in config (`source`, `import`, `base`, `patches`, `vars`) follow this precedence:

| Prefix | Meaning |
|--------|---------|
| `"zshrc"` | Relative to the current config file's directory |
| `"/shared/zshrc"` | Relative to the dotfiles root (`/` = root anchor) |
| `"~/.config/..."` | Home directory expansion |
| Absolute filesystem path | Used as-is (only valid for `base` in patch strategy) |

At import-merge time, all relative source paths are rebased to dotfiles-root-relative paths.
`/`-prefixed paths are rebased to the dotfiles root rather than the file's directory.
After merging, every source path in the resolved config is dotfiles-root-relative.

---

## Module Structure (target state)

```
src/
├── lib.rs               # Re-exports only; no logic
├── config/              # Config types + loading (already v4, keep + clean)
│   ├── mod.rs           # load_config_resolved(), resolve_dotfiles_dir(), etc.
│   ├── schema.rs        # Config, Dot, Package, Profile, Settings types
│   ├── loader.rs        # ConfigLoader trait + LoaderRegistry
│   └── toml_loader.rs   # TOML impl
├── core/                # BombadilError (miette) — keep as-is
├── sync/                # NEW: orchestrates the sync command
│   ├── mod.rs           # SyncEngine, SyncOptions
│   ├── plan.rs          # build_sync_plan() → SyncPlan
│   └── execute.rs       # execute_plan() → Session (with full audit recording)
├── dots/                # Dot strategies — keep, clean up
│   ├── mod.rs           # DotInstaller trait, InstallResult
│   ├── render.rs        # Tera template rendering
│   └── strategy/
│       ├── full.rs      # Full file replacement (symlink)
│       ├── patch.rs     # Line-based patch
│       ├── inject.rs    # Append/prepend injection
│       └── semantic/    # Structured format patches (JSON/YAML/TOML/INI)
├── packages/            # Package management — migrate to config:: types
│   ├── mod.rs           # PackageOrchestrator
│   ├── managers/        # Individual manager impls (Dnf, Apt, Brew, …)
│   └── drift.rs         # Drift detection
├── audit/               # Audit system — keep as-is
├── conflict/            # Conflict detection + resolution
├── secrets/             # GPG secrets
├── platform/            # Platform context auto-detection
└── validate.rs          # Secret detection in files
```

**Deleted** (legacy):
- `src/settings/` → replaced by `src/config/`
- `src/paths/mod.rs` → replaced by path resolution in `sync/plan.rs`
- `src/templating.rs` → replaced by `src/dots/render.rs`
- `src/error.rs` → replaced by `src/core/`
- `src/dots.rs`, `src/git.rs`, `src/state.rs` (already deleted, in archive)
- `src/lib.rs` logic → moved to respective modules; lib.rs becomes re-exports

**Removed from Cargo.toml**: `config` crate, `lazy_static`

---

## The `sync` Command

Replaces both `bombadil link` and `bombadil packages sync`. Single entrypoint for applying the full
desired state: dotfiles + packages.

### CLI

```
bombadil sync [OPTIONS]
    --dry-run              Show plan without executing
    --force                DotfileWins conflict strategy (skip interactive)
    --tags <tags>          Override active tags (comma-separated; default: from profile + auto-detected)
    --prune-packages       Remove packages installed but not in config
    --only-dots            Skip package operations
    --only-packages        Skip dot operations
```

The active profile is read from `.active_profile` (written by `bombadil init`). No `--profile`
flag on `sync` — the profile choice persists on the device via that file.

---

## Profiles & Machine Configs

### Model

Everything is a profile. Machine-specific configs (e.g. `fedora-kde-hidpi`, `macbook-work`) are
profiles committed to the repo. There is no hostname-based auto-detection.

```toml
# bombadil.toml
[profiles.fedora-kde]
active_tags   = ["gui", "kde", "wayland", "fedora"]
vars          = ["systems/fedora.toml"]

[profiles.fedora-kde-hidpi]
inherits      = ["fedora-kde"]
vars          = ["machines/hidpi.toml"]   # overrides specific vars

[profiles.work-macbook]
active_tags   = ["gui", "macos", "brew"]
vars          = ["systems/macos.toml", "machines/work.toml"]
package_exclude_tags = []
```

`active_tags` declares the tags that filter package/dot includes for this profile. They supplement
auto-detected platform tags (os, distro, desktop) which are merged in at runtime. For a fresh
install where the desktop environment is not yet detected, `active_tags` ensures correct filtering.

`inherits` lists profiles to compose. Later-listed profiles and the profile itself override earlier
ones (last-wins merge on vars + active_tags + hooks).

### `.active_profile`

`bombadil init <dotfiles-path> --profile <name>` does two things:
1. Links `bombadil.toml` to `$XDG_CONFIG_HOME/bombadil/bombadil.toml`
2. Writes `.active_profile` in the dotfiles directory with the chosen profile name

`.active_profile` is gitignored — it persists the profile choice on this device.

`bombadil sync` reads `.active_profile` automatically. To change profiles:
```
bombadil init --profile <new-name>
# or: directly edit .active_profile
```

### Var Loading Order (later wins)

1. `settings.vars` (global)
2. Auto-detected platform vars from `systems/<platform>.toml` (if file exists)
3. `profiles.<active>.vars`
4. Inherited profile vars (in inheritance order)
5. Dot-level `vars` (for that dot's templates only)

All vars are available as Tera variables in templates. Platform variables (`{{ os }}`, `{{ distro
}}`, `{{ desktop }}`) are always injected from `PlatformContext` regardless of var files.

---

### Hook Model

Hooks are **per-item** (per-dot or per-package), not global. This ensures a package's setup script
runs right after that package installs, not at the end of all operations.

Global hooks (`settings.prehooks` / `settings.posthooks`) remain supported as bookends — they run
before the first item and after the last. Profile-level hooks are merged into the global set when
that profile is activated.

```toml
# editor/nvim/dots.toml
[dot]
name      = "nvim"
prehooks  = ["mkdir -p ~/.local/share/nvim"]    # runs before this dot is installed
posthooks = ["nvim --headless '+Lazy sync' +qa 2>/dev/null || true"]

[dot.files]
"init.lua" = "~/.config/nvim/init.lua"
"lua/"     = "~/.config/nvim/lua/"

[dot.packages.neovim]
install.dnf  = "neovim"
install.brew = "neovim"
posthooks    = ["nvim --headless '+checkhealth' +qa 2>/dev/null || true"]
```

### Dependency Ordering

Dots support explicit `depends_on` to declare ordering constraints. The plan builder does a
topological sort; cycles are a config error caught at plan-build time (never at execution time).

```toml
# terminal/zsh/plugins/dots.toml
[dot]
name       = "zsh-plugins"
depends_on = ["zsh"]   # "zsh" is the dot.name in terminal/zsh/dots.toml

[dot.files]
"plugins.zsh" = "~/.config/zsh/plugins.zsh"
```

If no `depends_on` is set, declaration order in the config is preserved. Packages are always applied
after all dots (pending a future `depends_on` field on packages if needed).

### Execution Flow

```
SyncEngine::new(config, dotfiles_dir, options)
  │
  ├── build_sync_plan()              → SyncPlan (ordered Vec<SyncItem>)
  │     ├── resolve profiles → merged config view
  │     ├── topological sort dots by depends_on
  │     ├── For each dot: compute source/target/copy_path, detect conflict → PlannedDot
  │     ├── For each package: check is_installed() → PlannedPackage
  │     └── Attach global hooks as first/last items
  │
  ├── [dry-run: print plan and exit]
  │
  └── execute_plan(plan)             → Session
        ├── AuditStorage::init()
        ├── Global prehooks → Action::HookExecuted (each)
        ├── For each PlannedDot (in dependency order):
        │     ├── dot.prehooks → Action::HookExecuted (each)
        │     ├── installer.install() → Action (FileCreate/FileUpdate/Unchanged/Backup)
        │     └── dot.posthooks → Action::HookExecuted (each)
        ├── Orphan cleanup → unlink symlinks no longer in plan
        ├── For each PlannedPackage:
        │     ├── package.prehooks → Action::HookExecuted (each)
        │     ├── manager.install() → Action (PackageInstalled/PackageSkipped/PackageFailed)
        │     └── package.posthooks → Action::HookExecuted (each)
        ├── Global posthooks → Action::HookExecuted (each)
        ├── session.persist()
        └── return Session
```

---

## Data Types

### SyncOptions

```rust
pub struct SyncOptions {
    /// Active profile name (read from .active_profile; set by `bombadil init --profile`).
    pub profile: Option<String>,
    pub dry_run: bool,
    pub conflict_strategy: ConflictStrategy,
    /// Override active tags (supplements or replaces profile-declared tags).
    pub extra_tags: Vec<String>,
    pub prune_packages: bool,
    pub only_dots: bool,
    pub only_packages: bool,
}
```

### SyncPlan

The plan is an ordered flat list of items. Ordering respects `depends_on` (topological sort).
Hooks are embedded within their owning item — a `Hook` item with no parent is a global hook.

```rust
pub struct SyncPlan {
    /// Ordered sequence of items to execute (global hooks + dots + packages interleaved).
    pub items: Vec<SyncItem>,
}

pub enum SyncItem {
    Hook(PlannedHook),
    Dot(PlannedDot),
    Package(PlannedPackage),
}

pub struct PlannedHook {
    pub command: String,
    pub run_in_dotfiles_dir: bool,
    /// Owning dot or package name, None = global hook.
    pub owner: Option<String>,
    pub hook_phase: HookPhase,
}

pub enum HookPhase { Pre, Post }

pub struct PlannedDot {
    pub name: String,
    /// Namespace = source directory path relative to dotfiles root, e.g. "terminal/zsh".
    /// Displayed as //terminal/zsh in output; used to navigate to the config.
    pub namespace: String,
    pub dot: Dot,          // resolved (profile overrides applied)
    pub action: DotAction, // Create / Update / Unchanged / Backup+Overwrite / Skip
    /// Resolved file operations: (source_abs, target_abs, copy_mode) per file entry.
    pub files: Vec<PlannedFile>,
}

pub struct PlannedFile {
    pub source: PathBuf,
    pub target: PathBuf,
    /// true = regular file copy; false = symlink
    pub copy: bool,
    pub action: DotAction,
}

pub struct PlannedPackage {
    pub name: String,
    pub package: Package,
    pub action: PackageAction, // Install / AlreadyInstalled / Skip / Prune
}
```

The execution engine iterates `plan.items` in order. No special-casing — a `SyncItem::Hook` simply
runs its command and records the action; a `SyncItem::Dot` runs the installer.

### Session (existing, no change to type)

The `Session` type in `src/audit/session.rs` records the executed actions. No changes needed.

### Config Schema

A `dots.toml` file declares one **named dot** — a collection of file mappings plus packages,
hooks, and metadata. The dot name (`[dot].name`) is the stable identifier used in `depends_on`.

```rust
/// Top-level of a dots.toml file.
pub struct DotFile {
    pub dot: Dot,
}

pub struct Dot {
    /// Stable logical name used in depends_on and audit output.
    /// Defaults to the source directory name; warn if missing.
    pub name: String,

    /// file source (relative to dots.toml location) → target path (~/... or absolute)
    /// Value is FileTarget: simple string or extended options.
    #[serde(default)]
    pub files: IndexMap<String, FileTarget>,

    /// Vars files for Tera substitution (relative to dots.toml location).
    #[serde(default)]
    pub vars: Vec<PathBuf>,

    /// Packages installed before this dot's files are applied.
    #[serde(default)]
    pub packages: IndexMap<String, Package>,

    /// Dots that must be fully applied before this one.
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// Tags: dot is included only if at least one tag is in the active set.
    /// An empty tags list means always-include.
    #[serde(default)]
    pub tags: Vec<String>,

    #[serde(default)]
    pub prehooks: Vec<String>,
    #[serde(default)]
    pub posthooks: Vec<String>,

    /// Per-profile overrides (vars, additional files, tags).
    #[serde(default)]
    pub profiles: IndexMap<String, DotProfileOverride>,
}

/// FileTarget: simple string path or extended table.
/// #[serde(untagged)] works because TOML string vs inline-table are distinct types.
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
pub enum FileTarget {
    Simple(String),             // "~/.zshrc"
    Extended(FileTargetOptions),
}

pub struct FileTargetOptions {
    pub target: String,
    /// Glob patterns to exclude from directory copies.
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Copy as a regular file instead of symlinking.
    #[serde(default)]
    pub copy: bool,
}

pub struct DotProfileOverride {
    #[serde(default)]
    pub vars: Vec<PathBuf>,
    #[serde(default)]
    pub files: IndexMap<String, FileTarget>,
    #[serde(default)]
    pub prehooks: Vec<String>,
    #[serde(default)]
    pub posthooks: Vec<String>,
}
```

Fields to add/change on `Package`:

```rust
pub struct Package {
    /// Manager-specific install names. Optional — if absent, key name is the canonical name.
    #[serde(default)]
    pub install: Option<InstallMethods>,

    /// Tags: package is included only if at least one tag is in the active set.
    #[serde(default)]
    pub tags: Vec<String>,

    #[serde(default)]
    pub prehooks: Vec<String>,
    #[serde(default)]
    pub posthooks: Vec<String>,
}
```

### Dependency unavailability — visible skip with cascade

If a dot's dependency is excluded from the plan (wrong profile, platform, tags) or itself skipped,
the dependent dot is **kept in the plan but marked `⊘ skipped`** with the full reason chain.

Plan output:
```
[  4] +  //terminal/zsh: ~/.zshrc
[  5] ⊘  //terminal/zsh/plugins: skipped — depends on terminal/zsh (not in plan)
[  6] ⊘  //terminal/zsh/plugins/foo: skipped — depends on terminal/zsh/plugins (skipped)
```

Execution:
```
  ✓ [r3r87] +  //terminal/zsh: ~/.zshrc
  ⊘ [vsqxz]    //terminal/zsh/plugins — terminal/zsh not available
  ⊘ [pt208]    //terminal/zsh/plugins/foo — terminal/zsh/plugins skipped
```

Data model:
```rust
pub enum DotAction {
    Create,
    Update,
    Unchanged,
    Backup { original: PathBuf },
    Skip { reason: SkipReason },
}

pub enum SkipReason {
    /// Direct dependency not in plan (excluded by profile/platform/tags).
    DependencyUnavailable { dependency: String },
    /// Direct dependency was itself skipped; carries its reason for chain display.
    DependencySkipped { dependency: String, cause: Box<SkipReason> },
    /// Conflict resolution strategy chose to skip.
    ConflictStrategy,
}
```

The plan builder propagates skips in a second pass after topological sort: any dot whose dependency
has `action = Skip { .. }` is also marked `Skip { DependencySkipped { cause: .. } }`. The reason
chain is always fully traced to the root cause.

---

## Package System

### Packages inside dots

A dot can declare its own packages. They install before the dot's files are applied (software before
config), after the dot's prehooks (which can set up repos/keys):

```toml
# terminal/zsh/dots.toml
[dot]
name     = "zsh"
prehooks = ["sudo dnf copr enable user/zsh-nightly -y"]

[dot.files]
"zshrc"  = "~/.zshrc"
"zshenv" = "~/.zshenv"

[dot.packages.zsh]
install.dnf = "zsh"

[dot.packages.zsh-completions]
install.dnf = "zsh-completions"
```

Execution order within a dot: `prehooks → packages → files → posthooks`.

### Generic package references

The package key is the canonical cross-platform name. Manager-specific names are overrides;
when absent, a future registry resolves the canonical name per platform:

```toml
[dot.packages.zsh]
# No install block — canonical name "zsh" resolves via registry on all platforms

[dot.packages.kubectl]
install.dnf = "kubernetes-client"   # non-obvious name, needs explicit override
# brew, apt, etc. auto-resolved from registry as "kubectl"
```

The `install` block on a `Package` is optional. Resolution order:
1. Explicit manager field (e.g. `install.dnf = "kubernetes-client"`)
2. Registry lookup by package key name
3. Error with diagnostic listing available managers

**Registry implementation is deferred** — schema must support it from day one (optional `install`
block) but the lookup is a stub returning the key name until the registry is built.

### `PackageOrchestrator`

Renamed from `PackageManager` (avoids clash with the per-manager trait). Accepts
`&HashMap<String, config::Package>`, uses the `PackageManager` trait impls, records each
install/skip/fail as an `Action`. Drift detection stays in `packages/drift.rs`.

---

## CLI Surface (final)

```
bombadil
├── init <path> --profile <name>   Link bombadil.toml + write .active_profile
├── new <path> [--target] [--name] Scaffold a new dot at <path> with boilerplate dots.toml
├── sync                     Apply full desired state (dots + packages)
├── unlink                   Remove all managed symlinks
├── watch                    Auto-sync on file changes
│
├── log [-n N] [--file PATH] Show action history
├── inspect <id>             Show action details / diff / content
├── revert <id>              Revert an action
│
├── packages
│   ├── list                 List configured packages and status
│   ├── remove <name>        Remove a specific package
│   └── drift                Show packages installed but not in config
│
├── secrets
│   └── add                  Add encrypted secret
│
├── get <what>               Print metadata (vars, profiles, dotfiles-dir)
├── validate                 Check for unencrypted secrets in dotfiles
└── schema [type]            Print JSON schema for config types
```

**Removed**: `link` (→ `sync`), `install` (→ `init`), `packages install` (→ `sync`), `packages sync` (→ `sync`)

### `dots.toml` discovery

`dots.toml` files are discovered by **recursive traversal** of the dotfiles root. Any directory
containing a `dots.toml` is a managed dot. No explicit import chains between parent and child dots
are needed — containment in the directory tree is sufficient.

```
dotfiles/
├── bombadil.toml          ← root config, declares global settings/profiles/vars
├── terminal/
│   └── zsh/
│       ├── dots.toml      ← auto-discovered as child of root
│       ├── zshrc
│       └── plugins/
│           ├── dots.toml  ← auto-discovered as child of terminal/zsh
│           └── foo.zsh
└── editor/
    └── nvim/
        ├── dots.toml      ← auto-discovered
        └── init.lua
```

The `import` field in `bombadil.toml` is still supported for referencing configs outside the
dotfiles root (e.g. shared configs, machine-specific overlays), but is not needed for the normal
in-tree case. Explicit imports always take precedence over discovered dots when names collide.

### `bombadil init`

```
bombadil init <dotfiles-path> --profile <name>
```

1. Symlinks `<dotfiles-path>/bombadil.toml` → `$XDG_CONFIG_HOME/bombadil/bombadil.toml`
2. Writes `<dotfiles-path>/.active_profile` containing `<name>`

`.active_profile` is gitignored. To switch profiles: `bombadil init --profile <new>` or edit the
file directly.

### `bombadil new`

Scaffolds a new dot directory with a pre-filled `dots.toml`. No other file needs updating —
the new dot is automatically discovered on the next `sync`.

```
$ bombadil new terminal/zsh --target ~/.zshrc

  Created  terminal/zsh/
  Created  terminal/zsh/dots.toml
  Copied   ~/.zshrc → terminal/zsh/zshrc   (existing file captured as starting point)

  Ready. Run 'bombadil sync' to apply.
```

Generated `dots.toml`:
```toml
[dot]
name = "zsh"   # derived from directory name; use --name to override

[dot.files]
"zshrc" = "~/.zshrc"   # source relative to this file → target
```

With `--target` pointing to an existing system file, the current file is copied in as the starting
point — no manual capture step. For a directory target: `--target ~/.config/zsh/` copies the
whole directory and generates a directory mapping entry.

The name defaults to the leaf directory name. Pass `--name` to override if the default would
collide with an existing dot name.

---

## E2E Test Coverage (target)

Each test runs in an isolated temp directory with a full bombadil.toml and asserts filesystem state.

| CUJ | Test Name |
|-----|-----------|
| Full strategy: single file | `test_sync_full_file` |
| Full strategy: directory | `test_sync_full_directory` |
| Variable substitution | `test_sync_variable_substitution` |
| Profile activation | `test_sync_profile_activates_dot` |
| Profile inheritance (extra_profiles) | `test_sync_profile_inheritance` |
| Profile source override | `test_sync_profile_source_override` |
| Ignore patterns | `test_sync_ignore_patterns` |
| Hard copy (not symlink, with perms) | `test_sync_hard_copy` |
| Import resolution | `test_sync_imports` |
| Global prehook runs before all items | `test_sync_global_prehook` |
| Global posthook runs after all items | `test_sync_global_posthook` |
| Per-dot prehook runs before that dot only | `test_sync_dot_prehook` |
| Per-dot posthook runs after that dot only | `test_sync_dot_posthook` |
| Per-package posthook runs after package install | `test_sync_package_posthook` |
| Dot depends_on: ordered correctly | `test_sync_dot_depends_on` |
| depends_on cycle → config error | `test_sync_depends_on_cycle_error` |
| Hook output captured in audit | `test_sync_hook_audit_capture` |
| Conflict: DotfileWins | `test_sync_conflict_dotfile_wins` |
| Conflict: Skip | `test_sync_conflict_skip` |
| Orphan symlink cleanup | `test_sync_orphan_cleanup` |
| Dry-run shows plan, no changes | `test_sync_dry_run` |
| Patch strategy | `test_sync_patch_strategy` |
| Inject strategy | `test_sync_inject_strategy` |
| Semantic patch: JSON | `test_sync_semantic_json` |
| Semantic patch: YAML | `test_sync_semantic_yaml` |
| Semantic patch: TOML | `test_sync_semantic_toml` |
| Audit: log lists actions | `test_audit_log` |
| Audit: inspect shows action detail | `test_audit_inspect` |
| Audit: revert restores previous state | `test_audit_revert` |
| Packages: drift detects unmanaged | `test_packages_drift` |
| Validate: detects secrets | `test_validate_detects_secrets` |
| Platform context in templates | `test_platform_context` |
| `new`: scaffolds dots.toml with correct name | `test_new_creates_boilerplate` |
| `new --target <file>`: captures existing file | `test_new_captures_existing_file` |
| `new --target <dir>`: captures existing directory | `test_new_captures_existing_dir` |

---

## Implementation Phases

### Phase 0: Commit current state
Commit the ~50 uncommitted files cleanly. All tests pass. This is the baseline.

### Phase 1: `sync` orchestrator
Implement `src/sync/` module:
- `SyncEngine`, `SyncOptions`, `SyncPlan`, `PlannedDot`, `PlannedPackage`, `PlannedHook`
- `build_sync_plan()` using v4 config types and v4 dot strategies
- `execute_plan()` with full audit recording (mirrors what `execute_install_with_options` does)
- Wire `bombadil sync` CLI command to `SyncEngine`
- Tests: all existing lib tests + e2e tests must still pass

### Phase 2: Delete v3 modules
With `SyncEngine` in place:
- Remove `src/settings/`
- Remove `src/paths/mod.rs`
- Remove `src/templating.rs`
- Remove `src/error.rs`
- Clean up `src/lib.rs` (remove all v3 methods: `plan_install`, `execute_install_with_options`,
  `enable_profiles_v4` bridge, `v3_dot_from_v4`, `DotVar` trait, etc.)
- Remove `config` + `lazy_static` from Cargo.toml
- Confirm: 0 warnings, all tests pass

### Phase 3: Migrate packages to v4 types
- Move `packages/mod.rs` to use `config::Package` instead of `settings::packages::Package`
- Remove `settings::packages` if not already gone after Phase 2
- Audit all package operations (install/skip/fail/remove → Action)
- `bombadil packages drift` working end-to-end

### Phase 4: E2E test coverage
Add e2e tests for all CUJs in the table above. Each test:
- Creates an isolated `TempDir` with a minimal dotfiles layout
- Invokes the host bombadil binary (or library directly)
- Asserts the resulting filesystem state and/or audit log

### Phase 5: Code quality pass
- `cargo clippy -- -D warnings` clean
- `cargo fmt --check` clean
- Audit public API surface: remove unnecessary `pub`, add doc comments on key types
- Review all `TODO` / `FIXME` comments — resolve or create tracking notes
- Verify `schema/bombadil.schema.json` is up to date

---

## Decision Log

| Decision | Rationale |
|----------|-----------|
| `sync` replaces `link` + `packages sync` | One command for desired state; mirrors how tools like nix, ansible, etc. work |
| No `Bombadil` struct after cleanup | It was a stateful accumulator; `SyncEngine` is constructed per-invocation from config |
| Audit every action | Makes all state transitions observable and reversible — core design principle |
| Keep `audit/` as-is | It's clean, tested, and decoupled — no changes needed |
| Delete lib.rs logic | 2943-line God file; logic belongs in `sync/`, `packages/`, etc. |
| `packages/mod.rs` becomes `PackageOrchestrator` | `PackageManager` clashes with the trait of the same name |

---

## Resolved Decisions

| Question | Decision |
|----------|----------|
| `bombadil watch` scope | Triggers `sync --only-dots` (not packages); auto-relinks on file change |
| Keep `unlink` separate? | Yes — explicit destructive operation, not part of `sync` |
| Rename `install` → `init`? | Yes — rename to `init` to match DESIGN.md and common convention |
| Hooks per-item or global? | Per-item (dot and package each have prehooks/posthooks); global hooks are bookends |
| Dot dependency ordering | Explicit `depends_on: Vec<String>` on Dot; topological sort at plan-build time; cycles = config error |
| `sync` scope flags | `--only-dots` and `--only-packages` flags supported |
| Profile selection at runtime | `bombadil init --profile <name>` writes `.active_profile`; `sync` reads it automatically. No `--profile` flag on `sync` |
| Machine configs in repo? | Yes — named profiles per device are committed; `.active_profile` (gitignored) persists the active choice |
| Hostname-based auto-detection? | No — explicit `bombadil init --profile <name>` is simpler and more reliable |
| Dot = file collection | One `dots.toml` = one named dot with N file mappings; no per-file naming needed |
| `depends_on` references | By `dot.name` (stable logical ID), not source paths; default name = directory name |
| `FileTarget` enum variant | `#[serde(untagged)]`: string vs inline-table are distinct TOML types — confirmed working |
| Namespace in display | Always the source folder relative to dotfiles root (`//terminal/zsh`); navigational, not a reference |
| `dots.toml` discovery | Recursive traversal of dotfiles root; child dirs with their own `dots.toml` are excluded from parent's file traversal |
