+++
title = "Installation"
description = "How to install toml-bombadil and set up your dotfiles directory."
date = 2026-03-22
updated = 2026-03-22
draft = false
weight = 10
template = "docs/page.html"

[extra]
lead = "Install Bombadil via Cargo, set up shell completions, and register your dotfiles directory."
toc = true
top = false
+++

## Install via Cargo

```bash
cargo install toml-bombadil
```

This installs the latest v4 alpha from crates.io. Distro packages (AUR and others) currently track v3 — use `cargo install` to get v4.

## Shell completions

Generate completions once and place them where your shell can find them.

**Zsh:**

```bash
bombadil generate-completions zsh > ~/.zfunc/_bombadil
```

Make sure `~/.zfunc` is on your `$fpath` (add `fpath=(~/.zfunc $fpath)` before `compinit` in your `.zshrc`).

**Bash:**

```bash
bombadil generate-completions bash > ~/.local/share/bash-completion/completions/bombadil
```

## Initial setup

Register your dotfiles directory with Bombadil:

```bash
bombadil install ~/dotfiles
```

This creates a symlink at `~/.config/bombadil.toml` pointing to `~/dotfiles/bombadil.toml`, which is how Bombadil finds your configuration on every subsequent run.

## The `.dots/` render cache

Bombadil renders template variables into a `.dots/` directory inside your dotfiles repo before creating symlinks. The actual symlinks in your home directory point into `.dots/`, not directly at your source files.

Add `.dots/` to your `.gitignore` — the rendered copies are ephemeral and should not be committed:

```
# .gitignore
.dots/
```
