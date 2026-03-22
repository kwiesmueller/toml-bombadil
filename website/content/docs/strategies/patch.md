+++
title = "Patch Strategy"
description = "Apply declarative patches to a system-owned base file."
date = 2026-03-22
updated = 2026-03-22
draft = false
weight = 20
template = "docs/page.html"

[extra]
lead = "The patch strategy reads a system-owned base file, applies a set of declarative patches from your dotfiles, and writes the result to a user-controlled target. Use this when you want to customise a shared config without forking the whole file."
toc = true
top = false
+++

## Example: customising the system sway config

```toml
# sway/dots.toml
[dot.files]
sway = {
  strategy = "patch",
  base     = "/etc/sway/config",
  patches  = "sway/patches/",
  target   = "~/.config/sway/config"
}
```

Bombadil reads `/etc/sway/config` as the base, applies every patch file found
in `sway/patches/` in alphabetical order, and writes the result to
`~/.config/sway/config`.

---

## Patch file format

Patch files are TOML files placed in the `patches` directory. They are applied
in filename order, so prefix them with numbers to control sequencing.

**`sway/patches/00-term.toml`**

```toml
# Replace a line matching a regex
[[set]]
match = "^set \\$term .*"
value = "set $term kitty"

# Insert lines after a matching line
[[insert]]
after = "^### Key bindings"
lines = "bindsym $mod+Return exec $term"
```

### `[[set]]` — Replace a matching line

| Field | Type | Description |
|---|---|---|
| `match` | string (regex) | A regular expression matched against each line of the base file. |
| `value` | string | The replacement line written in place of every matching line. |

### `[[insert]]` — Insert lines after a match

| Field | Type | Description |
|---|---|---|
| `after` | string (regex) | Insert after the first line that matches this regular expression. |
| `lines` | string | One or more lines to insert. Use `\n` to insert multiple lines. |

---

## Multiple patch files

```
sway/patches/
  00-term.toml       ← set $term to kitty
  10-gaps.toml       ← enable gaps
  20-colours.toml    ← apply theme colours
```

Bombadil applies patch files in lexicographic order. Numbering files with a
two-digit prefix gives you explicit control over the order.

---

## When to use patch vs. full

Use the **patch** strategy when:
- The base file is owned by the system (`/etc/`) or another package.
- You want to track only your changes, not the entire file.
- The base file may be updated by system upgrades and you want to inherit those
  updates automatically.

Use the **full** strategy when you own the entire file and want to manage it
from scratch using template variables.
