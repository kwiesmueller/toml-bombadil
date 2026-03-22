<p align="center">
  <img
    width="250"
    src="./asset/logo.png"
    alt="Toml Bombadil - A dotfile manager written in rust"
  />
</p>

<p align="center">
  <a href="https://crates.io/crates/toml-bombadil">
    <img src="https://img.shields.io/crates/v/toml-bombadil.svg" alt="crates.io">
  </a>
  <a href="https://github.com/kwiesmueller/toml-bombadil/blob/main/LICENSE">
    <img src="https://img.shields.io/github/license/kwiesmueller/toml-bombadil" alt="License">
  </a>
</p>

<h1></h1>

> **v4 alpha** — this is a major rewrite of the original [oknozor/toml-bombadil](https://github.com/oknozor/toml-bombadil). The config format has changed. See [Migration from v3](#migration-from-v3) below.

**Bombadil** is a dotfile manager written in Rust. It symlinks your dotfiles, injects template variables, manages system packages across package managers, and keeps a full audit trail of every change it makes.

## Why bombadil?

Maintaining dotfiles across multiple machines and environments is messy. Bombadil solves this by keeping your dotfiles as templates and generating the actual configs at link time — injecting per-machine variables, applying patches to system-owned base files, or appending your customizations into existing files without overwriting them.

In v4 this extends to packages: each dotfile directory carries its own `dots.toml` declaring which packages are needed, how to install them on each package manager, and which tags gate their inclusion. A single `bombadil sync` applies dotfiles and packages together, records every operation, and can revert any of them individually.

## Installation

```bash
cargo install toml-bombadil
```

> AUR and other distribution packages track the upstream v3 release. Install via cargo for v4.

## Quick start (v4)

**1. Create a `dots.toml` in your dotfiles directory:**

```toml
[dot]
name = "zsh"

[dot.files]
"zshrc"  = "~/.zshrc"
"zshenv" = "~/.zshenv"
# Extended form with ignore patterns:
"config/" = { target = "~/.config/zsh/", ignore = ["*.bak"] }

[dot.packages.ripgrep]
[dot.packages.ripgrep.install]
dnf    = "ripgrep"
apt    = "ripgrep"
brew   = "ripgrep"
cargo  = "ripgrep"

[dot.profiles.work]
vars = ["profiles/work.toml"]
```

**2. Register your dotfiles directory and sync:**

```bash
bombadil install ~/dotfiles
bombadil sync
```

`sync` applies all `dots.toml` files found under your dotfiles directory: it installs packages first, then symlinks files.

## Config format overview

Bombadil v4 uses two config files:

- **`bombadil.toml`** (root, in `~/.config/`) — global settings, profiles, GPG key.
- **`dots.toml`** (one per dotfile subdirectory) — file mappings, packages, hooks for that logical unit.

### File mapping strategies

| Strategy | Use case |
|---|---|
| `full` (default) | Symlink file or directory to target |
| `patch` | Apply line-based patches against a base file (e.g. `/etc/sway/config`) |
| `semantic_patch` | Deep-merge JSON / YAML / TOML / INI files |
| `inject` | Append or prepend to existing files with idempotent markers |

Variable injection uses [Tera](https://keats.github.io/tera/) templates. Source files can reference `{{ var_name }}` and var files supply the values.

## Command reference

| Command | Description |
|---|---|
| `bombadil install [path]` | Register a dotfiles directory |
| `bombadil sync` | Apply dotfiles + packages (v4 unified command) |
| `bombadil link` | Symlink dotfiles only (v3-compatible) |
| `bombadil unlink` | Remove all managed symlinks |
| `bombadil migrate <dir>` | Convert v3 `bombadil.toml` imports to v4 `dots.toml` files |
| `bombadil packages install` | Install configured packages |
| `bombadil packages list` | List configured packages and their status |
| `bombadil drift` | Show packages installed but not in config |
| `bombadil log` | Show audit history of past sessions |
| `bombadil inspect <id>` | Inspect a specific action (details, diff, content) |
| `bombadil revert <id>` | Revert a specific action |
| `bombadil get <resource>` | Query configured dots, hooks, vars, profiles, secrets, platform |
| `bombadil validate` | Scan for potential unencrypted secrets |
| `bombadil add-secret` | Add a GPG-encrypted secret variable |
| `bombadil watch` | Watch dotfiles and re-link on changes |
| `bombadil schema` | Generate JSON Schema for editor validation |
| `bombadil generate-completions <shell>` | Generate shell completions (bash, zsh, fish, elvish) |

## Migration from v3

Run `bombadil migrate` to convert a v3 `bombadil.toml` (with `import` entries) into per-directory `dots.toml` files:

```bash
bombadil migrate ~/dotfiles
# Preview without writing:
bombadil migrate ~/dotfiles --dry-run
# Write output to a separate directory:
bombadil migrate ~/dotfiles --output ~/dotfiles-v4
```

The migrator reads each imported config file, converts its `[settings.dots.*]` entries into the v4 `[dot.files]` format, and writes a `dots.toml` next to the imported file. Existing `dots.toml` files are never overwritten.

After migrating, update `~/.config/bombadil.toml` to drop the `import` list (the engine now discovers `dots.toml` files automatically) and run `bombadil sync`.

## Shell completions

```bash
bombadil generate-completions zsh > ~/.zfunc/_bombadil
bombadil generate-completions bash > /etc/bash_completion.d/bombadil
```

## Contributing

Found a bug or want to propose a feature? Open an [issue](https://github.com/kwiesmueller/toml-bombadil/issues) or submit a pull request.

## License

MIT — see [LICENSE](LICENSE).
