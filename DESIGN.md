# Bombadil v2 Design Proposal

## Goals

A unified, low-overhead dotfiles management system that:

1. **Fully manages post-install system state** - packages, configs, secrets
2. **Detects drift** - unmanaged packages, modified configs
3. **Supports partial file modifications** - patches, not just full replacements
4. **Handles conflicts gracefully** - with human review when needed
5. **Works cross-platform** - Fedora, Debian, macOS with config inheritance
6. **Generates system inventory** - for future security scanning
7. **Provides good DX** - autocomplete, low boilerplate, clear errors

---

## Core Concepts

### 1. Dots (Config Files)

Three strategies for managing config files:

```toml
[dots.kitty]
# Strategy 1: Full replacement (current behavior)
source = "kitty/kitty.conf"
target = "~/.config/kitty/kitty.conf"

[dots.sway]
# Strategy 2: Patch-based (NEW)
strategy = "patch"
base = "/etc/sway/config"  # System default to patch against
patches = "sway/patches/"  # Directory of patch files
target = "~/.config/sway/config"

[dots.bashrc]
# Strategy 3: Append/prepend (NEW)
strategy = "inject"
target = "~/.bashrc"
prepend = "shell/bashrc.prepend"
append = "shell/bashrc.append"
marker = "# MANAGED BY BOMBADIL"  # Optional, for idempotent injection
```

### 2. Patch Format

For patch-based configs, use a simple declarative format rather than unified diff:

```toml
# sway/patches/00-variables.toml
[[set]]
# Replace a variable definition
match = "^set \\$term .*"
value = "set $term kitty"

[[set]]
match = "^set \\$menu .*"
value = "set $menu rofi -terminal '$term' -show combi -combi-modes 'drun,run'"

# sway/patches/10-appearance.toml
[[insert]]
# Add lines after a match
after = "^### Idle configuration"
lines = """
client.unfocused #000000 #000000 #888888 #112932 #000000
client.focused #000000 #000000 #cbcbcb #112932 #000000
client.focused_inactive #000000 #000000 #888888 #112932 #000000
titlebar_border_thickness 0
titlebar_padding 2 3
"""

[[delete]]
# Remove lines matching pattern
match = "^output \\* bg /usr/share/backgrounds"

[[insert]]
after = "^output \\* bg"
lines = "output * bg #000000 solid_color"
```

Benefits:
- Human-readable and writable
- Survives base file updates (applies to new versions)
- Clear intent (set X, add Y after Z, remove W)
- Can be validated/linted

### 2b. Semantic Patches (Structured Formats)

For structured formats (JSON, YAML, TOML, INI), use **strategic merge patches** inspired by Kustomize:

```toml
# vscode/patches/settings.toml
[patch]
format = "json"  # or "yaml", "toml", "ini"
base = "/usr/share/code/resources/app/product.json"  # Optional, can patch from scratch
target = "~/.config/Code/User/settings.json"

# Strategic merge - keys are merged, not replaced
[patch.merge]
"editor.fontSize" = 14
"editor.fontFamily" = "JetBrains Mono"
"workbench.colorTheme" = "One Dark Pro"
"[rust]" = { "editor.defaultFormatter" = "rust-lang.rust-analyzer" }

# Explicit delete keys
[patch.delete]
keys = ["telemetry.enableTelemetry", "update.mode"]

# Array operations
[[patch.arrays."editor.rulers"]]
operation = "set"  # replace entire array
value = [80, 120]

[[patch.arrays."files.exclude"]]
operation = "append"
value = ["**/.direnv", "**/.envrc"]
```

For **YAML** (common in k8s, docker-compose, etc.):

```toml
# k8s/patches/deployment.toml
[patch]
format = "yaml"
base = "k8s/base/deployment.yaml"

[patch.merge]
"spec.replicas" = 3
"spec.template.spec.containers[0].resources.limits.memory" = "512Mi"

# JSONPath-style targeting
[[patch.json_patch]]
op = "add"
path = "/spec/template/spec/containers/0/env/-"
value = { name = "DEBUG", value = "true" }
```

**Benefits over line-based patches:**
- **Robust to formatting changes** - whitespace, key ordering don't matter
- **Semantic operations** - "add to array", "merge object", "delete key"
- **Path-based targeting** - JSONPath/jq-style selectors
- **Type-aware** - knows that `3` is a number, not string "3"
- **Composable** - multiple patches merge cleanly

**Strategy selection by file extension:**

| Extension | Default Strategy |
|-----------|------------------|
| `.json`, `.jsonc` | Strategic merge (JSON) |
| `.yaml`, `.yml` | Strategic merge (YAML) |
| `.toml` | Strategic merge (TOML) |
| `.ini`, `.conf` (INI-style) | Strategic merge (INI) |
| Everything else | Line-based patch |

Can override with explicit `format` field.

### 3. Packages

Extend current package system with drift detection:

```toml
[packages.ripgrep]
tags = ["cli", "essential"]
install.dnf = "ripgrep"
install.apt = "ripgrep"
install.brew = "ripgrep"
install.cargo = "ripgrep"

[packages.kubectl]
tags = ["k8s"]
install.dnf = "kubernetes-client"
install.binary = {
  url = "https://dl.k8s.io/release/{{version}}/bin/{{os}}/{{arch}}/kubectl",
  version = "v1.29.0"
}
```

New commands:
```bash
# Detect unmanaged packages (packages installed but not in config)
bombadil packages drift

# Interactive: for each unmanaged package, choose:
# - Add to dotfiles (which tags?)
# - Ignore (add to ignore list)
# - Remove from system
bombadil packages reconcile

# Generate full system inventory
bombadil inventory export --format json > inventory.json
```

### 4. Profiles & Inheritance

Keep current profile system but add explicit inheritance:

```toml
# Base configuration (always applied)
[settings]
dots = { ... }
packages = { ... }

# Fedora-specific overrides
[profiles.fedora]
inherits = []  # No explicit inheritance, just overrides base
packages.enable_tags = ["dnf"]
packages.disable_tags = ["apt", "brew"]

# Work laptop = fedora + some extras
[profiles.work-laptop]
inherits = ["fedora"]
dots.vpn = { source = "vpn/work.conf", target = "~/.config/vpn/config" }
packages.extra = ["slack", "zoom"]

# Personal laptop might inherit fedora but add gaming
[profiles.personal]
inherits = ["fedora"]
packages.enable_tags = ["gaming"]
```

### 5. Conflict Detection & Resolution

Expand beyond link-time conflicts to ongoing monitoring:

```bash
# Check for files modified outside bombadil
bombadil status

# Output:
# Modified: ~/.config/sway/config (system differs from dotfile)
#   Last bombadil link: 2024-01-15
#   System file modified: 2024-01-20
#
# Untracked changes: ~/.config/Code/settings.json
#   Contains additions not in dotfile source

# Interactive resolution
bombadil resolve ~/.config/sway/config
# Shows diff, offers:
# - Keep dotfile version (overwrite system)
# - Keep system version (update dotfile source)
# - Merge (open in $EDITOR with conflict markers)
# - Skip
```

### 6. System Inventory

Track all managed components for security auditing:

```bash
bombadil inventory export
```

Output (JSON):
```json
{
  "generated_at": "2024-01-20T10:30:00Z",
  "platform": {
    "os": "linux",
    "distro": "fedora",
    "version": "39",
    "arch": "x86_64"
  },
  "packages": [
    {
      "name": "ripgrep",
      "version": "14.0.3",
      "install_method": "dnf",
      "tags": ["cli", "essential"],
      "source": "packages/cli.toml:15"
    },
    {
      "name": "kubectl",
      "version": "v1.29.0",
      "install_method": "binary",
      "binary_url": "https://dl.k8s.io/...",
      "tags": ["k8s"]
    }
  ],
  "configs": [
    {
      "name": "sway",
      "strategy": "patch",
      "base": "/etc/sway/config",
      "target": "~/.config/sway/config",
      "patches": ["00-variables.toml", "10-appearance.toml"]
    }
  ]
}
```

---

## Architecture

```
bombadil/
├── src/
│   ├── main.rs              # CLI entry point
│   ├── lib.rs               # Public API
│   │
│   ├── config/              # Configuration parsing
│   │   ├── mod.rs
│   │   ├── schema.rs        # Strongly typed config structs
│   │   ├── loader.rs        # TOML loading with imports
│   │   └── validation.rs    # Config validation
│   │
│   ├── dots/                # Dotfile management
│   │   ├── mod.rs
│   │   ├── strategy/
│   │   │   ├── full.rs      # Full file replacement strategy
│   │   │   ├── patch.rs     # Line-based patch strategy (NEW)
│   │   │   ├── inject.rs    # Append/prepend strategy (NEW)
│   │   │   └── semantic/    # Structured format patches (NEW)
│   │   │       ├── mod.rs
│   │   │       ├── json.rs  # JSON strategic merge
│   │   │       ├── yaml.rs  # YAML strategic merge
│   │   │       ├── toml.rs  # TOML strategic merge
│   │   │       └── ini.rs   # INI strategic merge
│   │   └── render.rs        # Template rendering (tera)
│   │
│   ├── packages/            # Package management
│   │   ├── mod.rs
│   │   ├── managers/        # Package manager implementations
│   │   │   ├── dnf.rs
│   │   │   ├── apt.rs
│   │   │   ├── brew.rs
│   │   │   ├── cargo.rs
│   │   │   └── binary.rs
│   │   ├── drift.rs         # Drift detection (NEW)
│   │   └── inventory.rs     # System inventory (NEW)
│   │
│   ├── conflicts/           # Conflict detection & resolution
│   │   ├── mod.rs
│   │   ├── detect.rs
│   │   ├── resolve.rs
│   │   └── diff.rs
│   │
│   ├── secrets/             # Secret management
│   │   ├── mod.rs
│   │   └── gpg.rs
│   │
│   ├── platform/            # Platform detection
│   │   └── mod.rs
│   │
│   └── error.rs             # Unified error handling
│
├── schema/                  # JSON Schema for config (NEW)
│   └── bombadil.schema.json # For editor autocomplete
│
└── tests/
```

### Error Handling Strategy

Replace mixed anyhow/custom errors with structured approach:

```rust
use thiserror::Error;
use miette::{Diagnostic, SourceSpan};

#[derive(Error, Diagnostic, Debug)]
pub enum BombadilError {
    #[error("Configuration error")]
    #[diagnostic(code(bombadil::config))]
    Config {
        #[source_code]
        src: String,
        #[label("here")]
        span: SourceSpan,
        #[help]
        help: Option<String>,
    },

    #[error("Package '{name}' not found in any configured manager")]
    #[diagnostic(code(bombadil::packages::not_found))]
    PackageNotFound { name: String },

    #[error("Patch failed to apply")]
    #[diagnostic(code(bombadil::dots::patch))]
    PatchFailed {
        patch_file: PathBuf,
        base_file: PathBuf,
        #[help]
        help: String,
    },

    // ... etc
}
```

Use `tracing` for structured logging:

```rust
use tracing::{info, warn, debug, instrument};

#[instrument(skip(vars))]
pub fn install_dot(name: &str, dot: &Dot, vars: &Variables) -> Result<()> {
    debug!(source = %dot.source.display(), "rendering template");
    // ...
    info!(target = %dot.target.display(), "linked successfully");
}
```

### JSON Schema for Autocomplete

Generate JSON Schema from Rust types:

```rust
use schemars::JsonSchema;

#[derive(Deserialize, JsonSchema)]
pub struct Config {
    /// Path to dotfiles directory
    pub dotfiles_dir: PathBuf,
    /// GPG user ID for secret encryption
    pub gpg_user_id: Option<String>,
    pub settings: Settings,
    #[serde(default)]
    pub profiles: HashMap<String, Profile>,
}
```

Then users configure their editor:
```json
// .vscode/settings.json
{
  "json.schemas": [
    {
      "fileMatch": ["bombadil.toml", "packages/*.toml"],
      "url": "./schema/bombadil.schema.json"
    }
  ]
}
```

(For TOML, use taplo with schema support)

---

## CLI Commands

```
bombadil
├── install <path>           # Initial setup (link bombadil.toml)
├── link [--profile ...]     # Apply dotfiles
├── unlink                   # Remove all managed symlinks
├── watch                    # Auto-relink on changes
│
├── status                   # Show drift (modified files, unmanaged packages)
├── resolve [file]           # Interactive conflict resolution
│
├── packages
│   ├── install [pkg]        # Install packages
│   ├── remove <pkg>         # Remove package
│   ├── list                 # List packages
│   ├── sync                 # Sync all packages to declared state
│   ├── drift                # Show unmanaged packages
│   └── reconcile            # Interactive drift resolution
│
├── inventory
│   ├── export               # Export full system inventory
│   └── diff <file>          # Compare current vs previous inventory
│
├── secrets
│   ├── add                  # Add encrypted secret
│   └── list                 # List secret keys (not values)
│
├── validate                 # Check for unencrypted secrets
├── get <what>               # Get metadata (vars, profiles, etc.)
└── init [--template]        # Scaffold new dotfiles repo (NEW)
```

---

## Migration Path

1. **v1.x compatible** - existing bombadil.toml files work unchanged
2. **New features opt-in** - patch strategy, drift detection are additive
3. **Deprecation warnings** - for any breaking changes

---

## Implementation Phases

### Phase 1: Foundation Cleanup
- [ ] Migrate to `tracing` for logging
- [ ] Migrate to `miette` for error reporting
- [ ] Add JSON Schema generation
- [ ] Refactor module structure

### Phase 2: Line-Based Patch Strategy
- [ ] Design patch file format (TOML-based)
- [ ] Implement patch parser
- [ ] Implement patch application engine (regex-based matching)
- [ ] Add `strategy = "patch"` support to dots

### Phase 3: Semantic Patch Strategy
- [ ] Implement JSON strategic merge (serde_json + json_patch)
- [ ] Implement YAML strategic merge (serde_yaml)
- [ ] Implement TOML strategic merge (toml crate)
- [ ] Implement INI strategic merge
- [ ] Auto-detect format by extension
- [ ] Add JSONPath/jq-style path selectors

### Phase 4: Inject Strategy
- [ ] Implement append/prepend with markers
- [ ] Add `strategy = "inject"` support

### Phase 5: Drift Detection
- [ ] Implement package drift detection
- [ ] Add `bombadil status` command
- [ ] Add `bombadil packages drift` command
- [ ] Add `bombadil packages reconcile` command

### Phase 6: System Inventory
- [ ] Design inventory format
- [ ] Implement inventory export
- [ ] Add inventory diff command

### Phase 7: DX Improvements
- [ ] Add `bombadil init` scaffolding
- [ ] Improve config validation with helpful errors
- [ ] Add shell completions improvements

---

## Open Questions

1. **Config format**: TOML vs KDL?
   - TOML: Mature JSON Schema tooling (Taplo), familiar to Rust users
   - KDL: Cleaner syntax for nested structures, better multiline strings, but requires custom schema tooling
   - Could support both?

2. **Line-based patch format**: Is the proposed TOML-based patch format good, or should we use something else (e.g., lua scripts, unified diff, custom DSL)?

3. **Semantic patch standard**: For structured formats, should we:
   - Use RFC 6902 JSON Patch (standard, but verbose)?
   - Use Kustomize-style strategic merge (simpler for common cases)?
   - Custom format optimized for dotfiles use cases?
   - Support multiple via adapters?

4. **Package drift scope**: Should drift detection cover:
   - Only explicitly installed packages?
   - All packages including dependencies?
   - User choice per-run?

5. **Conflict detection timing**:
   - Only at `link` time?
   - Continuous monitoring daemon?
   - On-demand `status` command?

6. **Config inheritance**: Current `extra_profiles` vs explicit `inherits` - which is clearer?

7. **Backwards compatibility**: How strict? Can we make breaking changes in v2.0?

---

## Example: Migrated Sway Config

Before (full file, 267 lines):
```
dotfiles/sway/config  # Full copy of /etc/sway/config with edits
```

After (patches only, ~30 lines total):
```
dotfiles/
├── sway/
│   ├── patches/
│   │   ├── 00-variables.toml    # $term, $menu overrides
│   │   ├── 10-appearance.toml   # colors, titlebar
│   │   ├── 20-input.toml        # touchpad settings
│   │   └── 30-bindings.toml     # custom keybindings
│   └── config.d/                # Additional includes (unchanged)
│       ├── 50-rules-browser.conf
│       └── ...
```

bombadil.toml:
```toml
[dots.sway]
strategy = "patch"
base = "/etc/sway/config"
patches = "sway/patches/"
target = "~/.config/sway/config"

# config.d files are still full replacement
[dots.sway-rules]
source = "sway/config.d/"
target = "~/.config/sway/config.d/"
```
