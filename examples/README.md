# Bombadil Examples and E2E Tests

This directory contains comprehensive examples demonstrating all bombadil features,
along with end-to-end tests that verify correct behavior.

## Directory Structure

```
examples/
├── base/                 # Simulated "before" state (existing configs)
├── dotfiles/             # Complete bombadil configuration
├── expectations/         # Expected "after" state for verification
│   ├── default/          # Results with no profile
│   ├── profile-laptop/   # Results with --profile laptop
│   └── profile-work/     # Results with --profile work
└── e2e/                  # Test infrastructure
    ├── Containerfile     # Minimal image with bombadil binary
    ├── run-tests.sh      # Test runner script
    └── explore.sh        # Interactive exploration mode
```

## Features Demonstrated

### Dot Strategies

| Strategy | Description | Example |
|----------|-------------|---------|
| **full** (default) | Complete file/directory replacement via symlinks | `full-simple/`, `full-directory/` |
| **patch** | Line-based patching with set/insert/delete | `patch-demo/` |
| **semantic_patch** | Format-aware patching (JSON/YAML/TOML/INI) | `semantic-*/` |
| **inject** | Prepend/append to existing files with markers | `inject-demo/` |

### Other Features

- **Variables**: Tera templating with `variables/` files
- **Profiles**: Environment-specific configs (`laptop`, `work`, `minimal`)
- **Imports**: Config composition via `imports/`
- **Hooks**: Pre/post link scripts in `hooks/`
- **Ignore patterns**: Excluding files from linking
- **Hard copy**: Copy with permissions instead of symlink
- **Platform context**: Auto-detected OS, distro, hostname, etc.

## Running E2E Tests

### Local (Recommended)

```bash
# Run all e2e tests
cargo test --test e2e

# Run specific test
cargo test --test e2e test_full_strategy

# Run with output
cargo test --test e2e -- --nocapture

# Using the helper script
./examples/e2e/run-tests.sh
./examples/e2e/run-tests.sh -- --nocapture
```

### In Container

```bash
# Run tests in a container (useful for CI or clean environment)
./examples/e2e/run-tests.sh --container
```

## Interactive Exploration

The `explore.sh` script launches an interactive container where you can safely
experiment with bombadil:

```bash
cd examples/e2e

# Default: read-only overlay (changes don't affect host)
./explore.sh

# Edit mode: changes persist to host (for developing examples)
./explore.sh --edit

# Pre-apply a specific profile
./explore.sh --profile laptop

# Rebuild the container image
./explore.sh --rebuild
```

### Inside the Container

```
~/dotfiles/      - Bombadil configuration (from examples/dotfiles)
~/base/          - Original "before" state (from examples/base)
~/expectations/  - Expected results (from examples/expectations)
```

Try these commands:
```bash
# Apply dotfiles
cd ~/dotfiles && bombadil link

# Apply with a profile
bombadil link --profile laptop

# Compare results against expectations
diff -r ~ ~/expectations/default/

# See the linked structure
tree ~/.config
```

### Mount Modes

- **Default (read-only overlay)**: Changes in the container don't affect host files.
  Uses Podman's overlay mounts or copies files for Docker.

- **Edit mode (`--edit`)**: Examples are mounted read-write. Changes persist to
  the host. Useful for developing new examples while testing them live.

## Test Structure

The e2e tests are written in Rust (`tests/e2e.rs`) and follow this pattern:

1. **TestEnv harness**: Creates an isolated temp directory as HOME
2. **Copy base files**: Simulates existing user configs
3. **Copy dotfiles**: Sets up bombadil configuration
4. **Run bombadil**: Executes `bombadil link` with various options
5. **Verify results**: Checks file existence, content, symlinks, permissions

### Test Categories

- **Full Strategy**: `test_full_strategy_*`
- **Templating**: `test_variable_*`, `test_local_vars_*`
- **Patch Strategy**: `test_patch_strategy_*`
- **Inject Strategy**: `test_inject_*`
- **Ignore Patterns**: `test_ignore_*`
- **Profiles**: `test_profile_*`
- **Imports**: `test_imports_*`
- **Hard Copy**: `test_hard_copy_*`
- **Hooks**: `test_hooks_*`
- **Semantic Patches**: `test_semantic_patch_*` (marked `#[ignore]` if not implemented)

## Updating Expectations

When bombadil behavior changes intentionally:

1. Run tests to see what changed
2. Update the expectation files to match new behavior
3. Verify tests pass with updated expectations
4. Document the behavior change

## Adding New Examples

1. Add source files to `dotfiles/` demonstrating the feature
2. Add any required base files to `base/` (the "before" state)
3. Add expected results to `expectations/default/` (and profile variants)
4. Write corresponding tests in `tests/e2e.rs`
5. Update the main `dotfiles/bombadil.toml` if adding new dots
