+++
title = "Drift Detection"
description = "Find packages installed on the system that are not declared in any dots.toml."
date = 2026-03-22
updated = 2026-03-22
draft = false
weight = 30
template = "docs/page.html"

[extra]
lead = "bombadil drift shows packages present on the system that have no matching entry in any dots.toml. Use it to catch manual installs before they get lost."
toc = true
top = false
+++

## Running drift detection

```bash
bombadil drift
```

Example output:

```
Drift detected: 3 packages installed but not configured
  + htop (dnf)
  + ncdu (dnf)
  + bat (cargo)
```

Each line shows the package name and the manager that owns it. Packages listed in any `dots.toml` under the active dotfiles directory are excluded from the output — only genuinely undeclared packages appear.

## Options

```bash
# Show extra installed packages (default behavior, explicit flag)
bombadil drift --extra

# Limit drift check to specific profiles
bombadil drift --profiles work,linux
```

`--extra` is implied when no flag is given; it reports packages installed on the system that are not in any dots.toml. When `--profiles` is passed, only the packages declared under those profiles are treated as "known" — anything else still counts as drift.

## Workflow

1. Run `bombadil drift` after a new machine setup or a manual install session.
2. For each package in the drift list, either add it to the appropriate `dots.toml` so it is tracked going forward, or remove it from the system if it was installed by mistake.
3. Run `bombadil drift` again to confirm the list is empty.

Drift detection does not modify any files or packages. It is a read-only audit command.
