+++
title = "Inject Strategy"
description = "Append or prepend content into an existing file using idempotent markers."
date = 2026-03-22
updated = 2026-03-22
draft = false
weight = 30
template = "docs/page.html"

[extra]
lead = "The inject strategy appends or prepends a block of content into a file that already exists — for example a shared ~/.bashrc managed by another tool. Bombadil wraps the injected content in marker comments so the operation is idempotent: running sync multiple times never duplicates the block."
toc = true
top = false
+++

## Example: extending a shared ~/.bashrc

```toml
# shell/dots.toml
[dot.files]
bashrc = {
  strategy = "inject",
  target   = "~/.bashrc",
  append   = "shell/bashrc.append",
  marker   = "# MANAGED BY BOMBADIL"
}
```

**`shell/bashrc.append`**

```bash
export EDITOR=nvim
export PATH="$HOME/.local/bin:$PATH"

alias ll='ls -lah'
alias gs='git status'
```

After `bombadil sync`, `~/.bashrc` gains the following block at the bottom:

```bash
# MANAGED BY BOMBADIL BEGIN
export EDITOR=nvim
export PATH="$HOME/.local/bin:$PATH"

alias ll='ls -lah'
alias gs='git status'
# MANAGED BY BOMBADIL END
```

Running `bombadil sync` again replaces the existing marked block rather than
appending a second copy.

---

## Prepend instead of append

```toml
[dot.files]
profile = {
  strategy = "inject",
  target   = "/etc/profile",
  prepend  = "shell/profile.prepend",
  marker   = "# MANAGED BY BOMBADIL"
}
```

Use `prepend` to insert the block at the top of the file instead. The marker
comments work the same way.

---

## Field reference

| Field | Type | Default | Description |
|---|---|---|---|
| `strategy` | string | — | Must be `"inject"`. |
| `target` | string | — | Path of the file to inject into. |
| `append` | string | — | Source file whose content is appended. Mutually exclusive with `prepend`. |
| `prepend` | string | — | Source file whose content is prepended. Mutually exclusive with `append`. |
| `marker` | string | — | Comment text used to delimit the managed block. Must be unique within the target file. |

---

## When to use inject

Use the **inject** strategy when:
- The target file is managed by another tool, your distribution, or a framework
  (e.g. Oh My Zsh modifies `~/.zshrc`).
- You want to contribute a block of configuration without taking ownership of
  the entire file.
- The target file is system-wide (e.g. `/etc/profile`) and you only want to add
  a small section.

Use the **full** strategy when you own and manage the entire file yourself.
