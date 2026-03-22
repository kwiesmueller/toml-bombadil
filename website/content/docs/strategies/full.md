+++
title = "Full Strategy"
description = "The default strategy: render a template then symlink the result."
date = 2026-03-22
updated = 2026-03-22
draft = false
weight = 10
template = "docs/page.html"

[extra]
lead = "The full strategy is the default for every file in [dot.files]. Bombadil renders the source through Tera, writes the result to a staging directory, and creates a symlink from the target path to the staged file."
toc = true
top = false
+++

## How it works

Given this layout in your dotfiles repository:

```
~/dotfiles/kitty/
  dots.toml
  kitty.conf       ← source template
  vars.toml
```

**`vars.toml`**

```toml
font_size   = "13"
font_family = "JetBrains Mono"
background  = "#1e1e2e"
```

**`kitty/kitty.conf`** (source template)

```
font_family      {{ font_family }}
font_size        {{ font_size }}
background       {{ background }}
```

**`kitty/dots.toml`**

```toml
[dot]
name = "kitty"

[dot.vars]
vars = ["vars.toml"]

[dot.files]
"kitty.conf" = "~/.config/kitty/kitty.conf"
```

After `bombadil sync`:

1. Bombadil renders `kitty/kitty.conf` through Tera, substituting all `{{ ... }}`
   expressions, and writes the result to `.dots/kitty/kitty.conf` inside your
   dotfiles directory.
2. A symlink is created: `~/.config/kitty/kitty.conf` → `.dots/kitty/kitty.conf`.

The rendered file at `~/.config/kitty/kitty.conf` contains:

```
font_family      JetBrains Mono
font_size        13
background       #1e1e2e
```

---

## Re-running sync

Every call to `bombadil sync` re-renders all templates. Changing a value in
`vars.toml` and running sync immediately updates every dotfile that uses that
variable — without touching the symlinks themselves.

---

## Opting out of template rendering

If a file contains `{{` or `{%` syntax that is not meant for Tera (e.g. a Go
template), set `copy = true` to skip rendering and deploy the file as-is:

```toml
[dot.files]
"go-template.yaml" = { target = "~/.config/app/template.yaml", copy = true }
```

See [File Targets](../../config/file-targets/) for the full list of options.
