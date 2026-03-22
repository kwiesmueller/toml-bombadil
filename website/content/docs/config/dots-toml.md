+++
title = "dots.toml Reference"
description = "Full annotated reference for the per-directory dots.toml configuration file."
date = 2026-03-22
updated = 2026-03-22
draft = false
weight = 10
template = "docs/page.html"

[extra]
lead = "Every directory inside your dotfiles repository can contain a dots.toml file. Bombadil discovers these files automatically and uses them to manage symlinks, variables, packages, and hooks for that directory."
toc = true
top = false
+++

## Overview

Place a `dots.toml` file at the root of each logical group of dotfiles. Bombadil
discovers every `dots.toml` under your `dotfiles_dir` and processes them in
dependency order.

```
~/dotfiles/
  zsh/
    dots.toml
    zshrc
    zshenv
  kitty/
    dots.toml
    kitty.conf
```

---

## `[dot]` — Dot metadata

```toml
[dot]
name       = "zsh"              # optional — defaults to the directory name
depends_on = ["base"]           # run the 'base' dot before this one
tags       = ["linux"]          # only include this dot when tag 'linux' is active
prehooks   = ["echo pre-sync"]
posthooks  = ["source ~/.zshrc"]
```

| Field | Type | Default | Description |
|---|---|---|---|
| `name` | string | directory name | Human-readable identifier used in logs and dependency references. |
| `depends_on` | array of strings | `[]` | Names of other dots that must be processed first. |
| `tags` | array of strings | `[]` | Dot is skipped unless at least one of these tags is active during sync. |
| `prehooks` | array of strings | `[]` | Shell commands run before this dot's files are deployed. |
| `posthooks` | array of strings | `[]` | Shell commands run after this dot's files are deployed. |

---

## `[dot.files]` — File mappings

Each entry maps a **source path** (relative to `dots.toml`) to a **destination**.

### Simple form

```toml
[dot.files]
"zshrc"  = "~/.zshrc"
"zshenv" = "~/.zshenv"
```

The source is rendered through Tera (variable injection) and the result is
symlinked to the target path. This is the default [full strategy](../../strategies/full/).

### Extended form

Use an inline table when you need more control:

```toml
[dot.files]
# Copy an entire directory, skipping backup files
"config/" = { target = "~/.config/zsh/", ignore = ["*.bak", "*.tmp"] }

# Create a regular copy instead of a symlink
"theme.css" = { target = "~/.theme.css", copy = true }

# Write an additional hard copy to a second location
"wayland.desktop" = {
  target              = "~/.wayland.desktop",
  hard_copy_target    = "/usr/share/wayland-sessions/mine.desktop",
  hard_copy_permissions = 0o644
}

# Use a non-default strategy
"sway" = { strategy = "patch", base = "/etc/sway/config", patches = "sway/patches/", target = "~/.config/sway/config" }
```

See [File Targets](../file-targets/) for a full description of every field,
and [Strategies](../../strategies/) for details on `patch`, `inject`, and
`semantic-patch`.

---

## `[dot.vars]` — Template variables

```toml
[dot.vars]
vars = ["vars.toml", "vars/colors.toml"]
```

Lists TOML files whose key-value pairs are injected into every file in this dot
during Tera rendering. See [Variables](../variables/) for details.

---

## `[dot.packages.*]` — Package declarations

```toml
[dot.packages.ripgrep]
[dot.packages.ripgrep.install]
dnf    = "ripgrep"
apt    = "ripgrep"
brew   = "ripgrep"
pacman = "ripgrep"
cargo  = "ripgrep"
```

Each sub-table declares one package and provides per-package-manager install
instructions. Bombadil reads the active package manager for the current system
and installs only the relevant entry.

Extended form with repo installation:

```toml
[dot.packages.kubectl]
[dot.packages.kubectl.install]
dnf  = { package = "kubectl", repo = "k8s/kubectl.repo" }
brew = "kubectl"
```

When `repo` is set, Bombadil installs the repo file before installing the
package. Full package documentation is covered in the Packages section.

---

## `[dot.profiles.*]` — Named profiles

```toml
[dot.profiles.work]
vars = ["vars/work.toml"]

[dot.profiles.work.files]
"zshrc.work" = "~/.zshrc"   # overrides the base zshrc mapping when 'work' is active
```

Profiles let you layer machine- or context-specific overrides on top of the
base dot configuration. Activate a profile with `bombadil sync --profile work`.
The profile's `vars` are merged on top of the base `[dot.vars]` list, and any
file mappings in `[dot.profiles.<name>.files]` override the base
`[dot.files]` entries for the same target.

Full profile documentation is covered in the Profiles section.
