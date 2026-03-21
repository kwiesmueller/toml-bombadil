# E2E Testing and Interactive Exploration

This directory contains infrastructure for running end-to-end tests in isolated containers
and for interactively exploring bombadil with example dotfiles.

## Prerequisites

- **podman** (recommended) or **docker**
- **Rust toolchain** (for running tests via cargo)

## Running E2E Tests

The tests run inside containers for complete isolation from your host system.

```bash
# Run all e2e tests
cargo test --test e2e

# Run a specific test
cargo test --test e2e test_full_strategy

# Run with output visible
cargo test --test e2e -- --nocapture
```

### What the tests verify

- **Full strategy**: File and directory symlink creation
- **Templating**: Tera variable substitution
- **Local vars**: Per-dot variable overrides
- **Ignore patterns**: Excluding files from linking
- **Profiles**: Profile-specific dots and inheritance
- **Imports**: Configuration composition
- **Hard copy**: Copying files with specific permissions
- **Hooks**: Pre/post link script execution

## Interactive Exploration

Use `explore.sh` to launch an interactive container where you can safely experiment
with bombadil. Changes inside the container don't affect your host system.

```bash
cd examples/e2e
./explore.sh
```

### Options

```bash
./explore.sh              # Default: read-only mode
./explore.sh --edit       # Changes persist to host (for developing examples)
./explore.sh --profile laptop  # Pre-apply a profile before starting
./explore.sh --rebuild    # Rebuild the container image first
```

### Inside the Container

When the container starts, you'll have:

```
~/dotfiles/       - Bombadil configuration (copied from examples/dotfiles)
~/base/           - Original files (copied from examples/base)
~/expectations/   - Expected results (mounted read-only)
```

Try these commands:

```bash
# Apply dotfiles
cd ~/dotfiles
bombadil link --force

# Apply with a profile
bombadil link --force --profiles laptop

# See what was created
tree ~/.config

# Check a specific file
cat ~/.config/full-simple/config.conf

# See templated variables were substituted
cat ~/.config/full-simple/templated.conf

# Compare against expectations
diff ~/.config/full-simple/config.conf ~/expectations/default/.config/full-simple/config.conf
```

### Available Profiles

The example dotfiles include these profiles:

- **laptop** - Adds laptop-specific config, uses smaller font size
- **work** - Inherits from laptop, adds work-specific config, overrides some sources

```bash
# Try different profiles
bombadil link --force --profiles laptop
bombadil link --force --profiles work
```

## Building the Container Image

The container image is built automatically when running tests or `explore.sh`.
To manually rebuild:

```bash
# Using podman
podman build -t bombadil-e2e-test -f Containerfile ../..

# Using docker
docker build -t bombadil-e2e-test -f Containerfile ../..
```

## How Container Isolation Works

1. **Tests** (`cargo test --test e2e`):
   - Each test spawns a fresh container
   - `examples/` is mounted read-only
   - Files are copied to writable locations inside the container
   - Container is destroyed after each test

2. **Exploration** (`./explore.sh`):
   - Default mode: Examples mounted read-only, copied to writable `~/`
   - Edit mode (`--edit`): Examples mounted read-write for development
   - Container is interactive (`-it`) and removed on exit (`--rm`)

## Developing New Examples

1. Add source files to `../dotfiles/`
2. Update `../dotfiles/bombadil.toml` with new dot entries
3. Add expected results to `../expectations/default/`
4. Write tests in `../../tests/e2e.rs`
5. Use `./explore.sh --edit` to test changes interactively

## Troubleshooting

### SELinux Issues (Fedora/RHEL)

The container uses `--security-opt label=disable` to avoid SELinux permission issues
with volume mounts. This is handled automatically.

### Container Image Not Found

If tests fail with "image not found", rebuild the image:

```bash
podman build -t bombadil-e2e-test -f examples/e2e/Containerfile .
```

### Permission Denied

Ensure the examples directory is readable:

```bash
chmod -R a+rX examples/
```
