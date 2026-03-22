+++
title = "Profiles"
description = "How to define and activate profiles in dots.toml."
date = 2026-03-22
updated = 2026-03-22
draft = false
weight = 10
template = "docs/page.html"

[extra]
lead = "Profiles override vars, files, and hooks for a specific context. Activate one with --profiles work on any Bombadil command."
toc = true
top = false
+++

## Defining a profile

Profiles live inside the same `dots.toml` as the base configuration, under `[dot.profiles.<name>]`.

```toml
[dot]
name = "zsh"
vars = ["vars/base.toml"]

[dot.files]
"zshrc" = "~/.zshrc"

[dot.profiles.work]
vars = ["vars/work.toml"]

[dot.profiles.work.files]
"zshrc.work" = "~/.zshrc"   # replaces the base mapping for ~/.zshrc
```

When the `work` profile is active:

- `vars/work.toml` is loaded **in addition to** `vars/base.toml` (later values win).
- The file mapping `~/.zshrc` is satisfied by `zshrc.work` instead of `zshrc`.
- All other base file mappings remain in effect unchanged.

A profile only needs to declare what it overrides. Anything not mentioned in the profile is inherited from the base `[dot]` section.

## Activating a profile

Pass `--profiles` to any Bombadil command:

```bash
# Apply dotfiles with the work profile active
bombadil link --profiles work

# Apply dotfiles + packages with work and linux profiles active
bombadil sync --profiles work,linux
```

Multiple profiles can be combined with a comma-separated list. When two profiles both override the same key, the last one listed wins.

Bombadil also reads `.active_profile` from your dotfiles directory when you run `bombadil sync` with no flags, so you can persist the active profile across shells:

```bash
echo "work" > ~/dotfiles/.active_profile
bombadil sync   # automatically uses the work profile
```

## Overriding hooks per profile

```toml
[dot]
name = "git"

[dot.profiles.work]
prehooks  = ["cp ~/certs/work-ca.crt /usr/local/share/ca-certificates/"]
posthooks = ["update-ca-certificates"]
```

Profile-level hooks replace (not append to) the base hooks when the profile is active.

## Tags

Tags are a lighter-weight alternative to profiles for toggling individual dots or packages.

```toml
[dot]
name = "linux-tools"
tags = ["linux"]

[dot.packages.wl-clipboard]
tags = ["linux", "wayland"]
```

A dot or package with a `tags` list is only included when all of its tags are in the active tag set. Tags come from the active profile or from `--tags` on the command line:

```bash
bombadil sync --tags linux,wayland
bombadil packages install --tags cli
```

Tags and profiles can be combined:

```bash
bombadil sync --profiles work --tags linux,wayland
```

## Full example

```toml
# zsh/dots.toml

[dot]
name = "zsh"
vars = ["vars/base.toml"]
prehooks  = []
posthooks = ["source ~/.zshrc"]

[dot.files]
"zshrc"    = "~/.zshrc"
"zshenv"   = "~/.zshenv"
"zprofile" = "~/.zprofile"

[dot.profiles.work]
vars      = ["vars/work.toml"]
posthooks = ["source ~/.zshrc", "load-work-secrets"]

[dot.profiles.work.files]
"zshrc.work" = "~/.zshrc"   # work machine gets a different zshrc

[dot.profiles.personal]
vars = ["vars/personal.toml"]
# no file overrides — base mappings are used as-is
```

```bash
bombadil link --profiles work
# → renders vars/base.toml + vars/work.toml
# → links zshrc.work → ~/.zshrc
# → links zshenv, zprofile from base
# → runs work posthooks

bombadil link --profiles personal
# → renders vars/base.toml + vars/personal.toml
# → links zshrc, zshenv, zprofile from base
# → runs base posthooks
```
