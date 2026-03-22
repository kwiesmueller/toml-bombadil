+++
title = "Quick Start"
description = "Create a minimal working Bombadil setup from scratch."
date = 2026-03-22
updated = 2026-03-22
draft = false
weight = 20
template = "docs/page.html"

[extra]
lead = "Go from zero to managed dotfiles in five steps."
toc = true
top = false
+++

## Step 1: Install

```bash
cargo install toml-bombadil
```

See [Installation](../installation/) for shell completion setup and distro package notes.

## Step 2: Create a dots.toml

Each subdirectory in your dotfiles repo can have a `dots.toml` that describes the files and packages it owns. Here is a realistic example for a zsh + starship setup:

```toml
[dot]
name = "zsh"

[dot.files]
"zshrc"  = "~/.zshrc"
"zshenv" = "~/.zshenv"

[dot.packages.zsh]
[dot.packages.zsh.install]
dnf    = "zsh"
apt    = "zsh"
brew   = "zsh"

[dot.packages.starship]
[dot.packages.starship.install]
cargo  = "starship"
brew   = "starship"
```

Place this file at `~/dotfiles/terminal/zsh/dots.toml` alongside your `zshrc` and `zshenv` source files.

## Step 3: Register and sync

```bash
bombadil install ~/dotfiles
bombadil sync
```

`sync` walks every subdirectory under `~/dotfiles`, discovers all `dots.toml` files, installs any missing packages using the package manager it detects on your system, then renders templates and creates symlinks for each declared file.

## Step 4: Variable substitution

Variables let you inject values into your dotfiles at render time without editing the source files directly.

**vars.toml** — define your variables:

```toml
font_size = "13"
terminal  = "kitty"
```

**zshrc** — reference them with `{{ }}`:

```
export TERMINAL={{ terminal }}
```

**dots.toml** — point to the var file:

```toml
[dot]
vars = ["vars.toml"]

[dot.files]
"zshrc" = "~/.zshrc"
```

Run `bombadil sync` again. The rendered copy in `.dots/zshrc` will contain `export TERMINAL=kitty`, and `~/.zshrc` symlinks to that rendered file.

## Step 5: Check what happened

```bash
bombadil log       # show the audit history of every sync
bombadil get dots  # list managed dotfiles and their symlink status
```

`bombadil log` shows a timestamped record of every file that was linked or changed, so you always know what Bombadil touched and when.

---

## Next steps

- **Config reference** — full `dots.toml` and `bombadil.toml` field documentation
- **Strategies** — how to manage multiple machines, profiles, and per-host overrides
