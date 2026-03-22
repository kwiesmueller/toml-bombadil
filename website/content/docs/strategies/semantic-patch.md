+++
title = "Semantic Patch Strategy"
description = "Deep-merge structured data into an existing config file."
date = 2026-03-22
updated = 2026-03-22
draft = false
weight = 40
template = "docs/page.html"

[extra]
lead = "The semantic-patch strategy merges your dotfile's values into an existing structured config file rather than replacing it. Bombadil understands the file format (JSON, YAML, TOML, INI) and performs a deep merge, so keys you do not mention are left untouched."
toc = true
top = false
+++

## Example: VS Code settings

Your dotfile declares only the keys you care about:

**`vscode/settings.json`** (source in your dotfiles repo)

```json
{
  "editor.fontSize": 13,
  "editor.fontFamily": "JetBrains Mono",
  "editor.lineHeight": 1.6
}
```

**`vscode/dots.toml`**

```toml
[dot.files]
"vscode/settings.json" = {
  strategy = "semantic-patch",
  target   = "~/.config/Code/User/settings.json"
}
```

After `bombadil sync`, the existing `~/.config/Code/User/settings.json` is
updated: the three keys above are written or overwritten, while every other key
already present in the file is preserved. Bombadil does not truncate or replace
the file.

---

## Supported formats

| Format | Detected by |
|---|---|
| JSON | `.json` extension |
| YAML | `.yaml` / `.yml` extension |
| TOML | `.toml` extension |
| INI | `.ini` / `.cfg` extension |

Bombadil detects the format from the target file's extension.

---

## Deep merge behaviour

Given an existing `~/.config/Code/User/settings.json`:

```json
{
  "editor.fontSize": 12,
  "editor.tabSize": 4,
  "workbench.colorTheme": "Catppuccin Mocha",
  "extensions.autoUpdate": false
}
```

And a source file containing:

```json
{
  "editor.fontSize": 13,
  "editor.fontFamily": "JetBrains Mono"
}
```

The result after sync:

```json
{
  "editor.fontSize": 13,
  "editor.tabSize": 4,
  "workbench.colorTheme": "Catppuccin Mocha",
  "extensions.autoUpdate": false,
  "editor.fontFamily": "JetBrains Mono"
}
```

`editor.fontSize` is overwritten, `editor.fontFamily` is added, and all other
keys are untouched.

---

## Combining with template variables

The source file is rendered through Tera before merging, so you can use
variables inside it:

**`vscode/settings.json`**

```json
{
  "editor.fontSize": {{ font_size }},
  "editor.fontFamily": "{{ font_family }}"
}
```

Declare the variables in `[dot.vars]` as usual. See
[Variables](../../config/variables/) for details.

---

## When to use semantic-patch

Use the **semantic-patch** strategy when:
- The target is a structured data file that other tools also write to (VS Code
  settings, language server configs, etc.).
- You only want to manage a subset of the keys in the file.
- Replacing the entire file would cause another tool to lose its settings.

Use the **full** strategy when you manage the entire file yourself and do not
need to preserve keys written by other tools.
