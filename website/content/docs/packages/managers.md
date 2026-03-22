+++
title = "Package Managers"
description = "Supported package managers and what Bombadil does for each."
date = 2026-03-22
updated = 2026-03-22
draft = false
weight = 20
template = "docs/page.html"

[extra]
lead = "Bombadil auto-detects which package manager is available and runs the appropriate install command. Each manager has a key name used in dots.toml."
toc = true
top = false
+++

## Auto-detection

Bombadil walks the list of managers for which a package has an entry and picks the first one whose binary is found on `$PATH` (via `which <manager>`). The order of detection follows the list below. Only one manager is used per package per run.

If no supported manager is found for a package, Bombadil skips it and records a warning in the audit log.

## Supported managers

### `dnf`

**Detection:** `which dnf`

**Install command:** `dnf install -y <package>`

**Repo support:** When the entry is a table with a `repo` key, Bombadil copies the file at that relative path to `/etc/yum.repos.d/<filename>` before installing.

```toml
[dot.packages.kubectl.install]
dnf = { package = "kubectl", repo = "k8s/kubectl.repo" }
```

### `apt`

**Detection:** `which apt`

**Install command:** `apt install -y <package>`

**Repo support:** When `repo_url` and `gpg_key` are provided, Bombadil fetches the GPG key, adds it via `apt-key`, writes the source list entry to `/etc/apt/sources.list.d/`, and runs `apt update` before installing.

```toml
[dot.packages.docker.install]
apt = { package = "docker-ce", repo_url = "https://download.docker.com/linux/ubuntu", gpg_key = "https://download.docker.com/linux/ubuntu/gpg" }
```

### `brew`

**Detection:** `which brew`

**Install command:** `brew install <package>`

Homebrew formulae and casks are both supported — pass the cask name as-is (e.g. `"docker"` for Docker Desktop). Bombadil does not currently differentiate `brew install` from `brew install --cask`; use the exact name as you would on the command line.

```toml
[dot.packages.ripgrep.install]
brew = "ripgrep"
```

### `pacman`

**Detection:** `which pacman`

**Install command:** `pacman -S --noconfirm <package>`

AUR helpers are not auto-detected. If you use `yay` or `paru`, invoke them via a prehook instead.

```toml
[dot.packages.ripgrep.install]
pacman = "ripgrep"
```

### `cargo`

**Detection:** `which cargo`

**Install command:** `cargo install <crate>`

The crate name must match exactly as it appears on crates.io. Bombadil does not pass `--locked` by default; add `--locked` in a prehook if you need reproducible builds.

```toml
[dot.packages.bat.install]
cargo = "bat"
```

### `go`

**Detection:** `which go`

**Install command:** `go install <module>@latest`

The value must be a fully qualified Go module path. Bombadil always appends `@latest`; pin to a specific version by including it in the path.

```toml
[dot.packages.gopls.install]
go = "golang.org/x/tools/gopls"

# Pin to a specific version:
[dot.packages.air.install]
go = "github.com/air-verse/air@v1.52.3"
```

### `flatpak`

**Detection:** `which flatpak`

**Install command:** `flatpak install -y <app-id>`

The value must be the full Flatpak application ID. Bombadil does not configure remotes; add the remote (e.g. Flathub) via a prehook if it is not already present.

```toml
[dot.packages.obsidian.install]
flatpak = "md.obsidian.Obsidian"
```

### `binary`

Planned. Not yet implemented. Intended for downloading pre-compiled binaries from GitHub Releases or direct URLs and placing them on `$PATH`.

## Installing packages

```bash
# Install all packages for the active manager
bombadil packages install

# Install only packages tagged 'cli' or 'tools'
bombadil packages install --tags cli,tools

# Sync: install missing packages, optionally remove unlisted ones
bombadil packages sync
bombadil packages sync --prune

# List all declared packages and their install status
bombadil packages list

# Update all installed packages
bombadil packages update

# Remove a package
bombadil packages remove ripgrep
```
