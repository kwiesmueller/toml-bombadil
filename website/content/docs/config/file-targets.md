+++
title = "File Targets"
description = "All FileTarget options for controlling how individual files are deployed."
date = 2026-03-22
updated = 2026-03-22
draft = false
weight = 20
template = "docs/page.html"

[extra]
lead = "Each entry in [dot.files] maps a source path to a destination. The simple form is a plain string; the extended form is an inline table that unlocks additional options."
toc = true
top = false
+++

## Simple form

```toml
[dot.files]
"kitty.conf" = "~/.config/kitty/kitty.conf"
```

The source file is rendered through Tera and a symlink is created at the target
path. This covers the vast majority of dotfile deployments.

---

## `target` — Destination path

```toml
[dot.files]
"kitty.conf" = { target = "~/.config/kitty/kitty.conf" }
```

`target` is the only required field in the extended form. It supports `~/` for
the home directory and must be an absolute path after expansion. The simple
string form is shorthand for `{ target = "..." }`.

---

## `ignore` — Exclude patterns for directory copies

```toml
[dot.files]
"config/" = { target = "~/.config/zsh/", ignore = ["*.bak", "*.tmp", ".DS_Store"] }
```

When the source is a directory, `ignore` lists glob patterns for files that
should not be copied or linked. Bombadil walks the directory recursively and
skips any entry whose filename matches one of the patterns. Useful for keeping
editor backup files and OS metadata out of `~/.config`.

---

## `copy` — Create a regular file copy

```toml
[dot.files]
"theme.css" = { target = "~/.theme.css", copy = true }
```

By default Bombadil creates a symlink. Setting `copy = true` writes a real copy
of the rendered file to the target path instead. Use this when the consuming
application does not follow symlinks, or when you want the deployed file to be
independently editable without touching your dotfiles repository.

---

## `hard_copy_target` — Write an additional copy to a second location

```toml
[dot.files]
"wayland.desktop" = {
  target              = "~/.wayland.desktop",
  hard_copy_target    = "/usr/share/wayland-sessions/mine.desktop",
  hard_copy_permissions = 0o644
}
```

`hard_copy_target` writes a second, independent copy of the rendered file to an
additional path. This is particularly useful for **SDDM** and other display
managers that read session files from system directories and do not follow
symlinks. With this setup the symlink at `~/.wayland.desktop` stays up to date
for your user session, while the hard copy at
`/usr/share/wayland-sessions/mine.desktop` is what SDDM actually reads.

### `hard_copy_permissions`

```toml
hard_copy_permissions = 0o644
```

Sets the Unix file permissions for the hard copy as an octal integer. This is
separate from the permissions of the symlink or main target. Useful when the
destination directory is system-owned and requires specific permissions (e.g.
`0o644` for files under `/usr/share/`).

---

## `strategy` — Deployment strategy

```toml
[dot.files]
"sway" = { strategy = "patch", base = "/etc/sway/config", patches = "sway/patches/", target = "~/.config/sway/config" }
```

Override the default `full` strategy for an individual file. See the
[Strategies](../../strategies/) section for full details on `patch`, `inject`,
and `semantic-patch`.

---

## Field reference

| Field | Type | Default | Description |
|---|---|---|---|
| `target` | string | — | Destination path. Required in extended form. |
| `ignore` | array of strings | `[]` | Glob patterns to exclude when copying a directory. |
| `copy` | bool | `false` | Write a real copy instead of a symlink. |
| `hard_copy_target` | string | — | Additional path to receive a real copy. |
| `hard_copy_permissions` | octal int | — | Unix permissions for `hard_copy_target`. |
| `strategy` | string | `"full"` | Deployment strategy: `full`, `patch`, `inject`, `semantic-patch`. |
