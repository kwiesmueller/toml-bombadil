+++
title = "Template Variables"
description = "Inject dynamic values into your dotfiles using Tera templates and TOML var files."
date = 2026-03-22
updated = 2026-03-22
draft = false
weight = 30
template = "docs/page.html"

[extra]
lead = "Bombadil renders every source file through the Tera template engine before deploying it. Variables defined in TOML var files are available in your dotfiles as {{ variable_name }}."
toc = true
top = false
+++

## Basic example

**`kitty/vars.toml`**

```toml
font_size    = "13"
font_family  = "JetBrains Mono"
background   = "#1e1e2e"
foreground   = "#cdd6f4"
```

**`kitty/kitty.conf`** (source template)

```
font_family      {{ font_family }}
font_size        {{ font_size }}
background       {{ background }}
foreground       {{ foreground }}
```

After `bombadil sync`, the rendered file at `~/.config/kitty/kitty.conf`
contains the literal values:

```
font_family      JetBrains Mono
font_size        13
background       #1e1e2e
foreground       #cdd6f4
```

---

## Declaring var files

```toml
[dot.vars]
vars = ["vars.toml", "vars/colors.toml"]
```

List var files in the `[dot.vars]` section of `dots.toml`. Paths are relative
to the `dots.toml` file. Files are loaded in order; later files override keys
defined in earlier ones.

---

## Var file format

```toml
# vars/colors.toml
background = "#1e1e2e"
foreground = "#cdd6f4"
accent     = "#cba6f7"
font_size  = "13"
```

Var files are plain TOML files where every value is a string. Keys become
directly available as `{{ key }}` in any template within the dot.

---

## Using variables in templates

Bombadil uses [Tera](https://tera.netlify.app/) for template rendering. Any
Tera expression is valid inside your dotfile sources. Common patterns:

```
# Simple substitution
font-size: {{ font_size }}px

# Conditional
{% if theme == "dark" %}
background = #1e1e2e
{% else %}
background = #eff1f5
{% endif %}

# Default value fallback
opacity = {{ opacity | default(value="1.0") }}
```

---

## GPG-encrypted secrets

Sensitive values (API keys, tokens, passwords) can be stored encrypted and
referenced the same way as regular variables.

```bash
bombadil add-secret GITHUB_TOKEN ghp_xxxxxxxxxxxx
```

This encrypts the value with your GPG key (configured via `gpg_user_id` in
`bombadil.toml`) and stores it. During sync, Bombadil decrypts secrets and
injects them alongside regular variables:

```
# ~/.config/gh/config.yml template
oauth_token: {{ GITHUB_TOKEN }}
```

No plaintext secrets are written to disk in your dotfiles repository.

---

## Profile-specific var files

Profiles let you override individual variable values for a specific machine or
context without duplicating your entire var file.

```toml
# dots.toml
[dot.vars]
vars = ["vars.toml"]       # base variables, always loaded

[dot.profiles.work]
vars = ["vars/work.toml"]  # merged on top of base when 'work' profile is active
```

**`vars/work.toml`**

```toml
font_size = "14"           # larger font on the work monitor
background = "#002b36"     # different theme at the office
```

When you run `bombadil sync --profile work`, Bombadil loads `vars.toml` first
and then merges `vars/work.toml` on top, so only the keys present in
`work.toml` are overridden. All other variables keep their base values.
