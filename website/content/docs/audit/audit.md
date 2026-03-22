+++
title = "Audit Trail"
description = "How to browse, inspect, and revert Bombadil actions using bombadil log, inspect, and revert."
date = 2026-03-22
updated = 2026-03-22
draft = false
weight = 10
template = "docs/page.html"

[extra]
lead = "Every symlink, file write, and hook execution is recorded. Use bombadil log to browse history, bombadil inspect to examine a specific action, and bombadil revert to undo it."
toc = true
top = false
+++

## bombadil log

`bombadil log` shows the session history — each sync or link run is a session, and each action within it is listed with its short ID.

```bash
bombadil log
```

Example output:

```
Session abc12 — link — 2026-03-22 14:30:01
  ✓ [ab12] Create  ~/.zshrc → .dots/zsh/zshrc
  ✓ [cd34] Create  ~/.config/kitty/kitty.conf → .dots/kitty/kitty.conf
  ✓ [ef56] HookExecuted  posthook: source ~/.zshrc (exit 0)

Session 9f3a1 — sync — 2026-03-21 09:12:44
  ✓ [a1b2] Create  ~/.gitconfig → .dots/git/gitconfig
  ✓ [c3d4] PackageInstalled  ripgrep (dnf)
  ! [e5f6] HookExecuted  posthook: rehash (exit 1)
```

Each action line shows:
- The short action ID (used with `bombadil inspect` and `bombadil revert`)
- The action type (`Create`, `HookExecuted`, `PackageInstalled`, etc.)
- A brief description of what was done

### Options

```bash
# Show only the last 5 sessions
bombadil log --limit 5

# Filter to actions that touched a specific file
bombadil log --file ~/.zshrc

# Filter to actions from a specific dot (as named in dots.toml)
bombadil log --dot zsh

# Show full details for every action (no truncation)
bombadil log --verbose
```

## bombadil inspect

`bombadil inspect <id>` shows the full record for one action.

```bash
# Show details (default)
bombadil inspect ab12

# Show a before/after diff of the file content
bombadil inspect ab12 --show diff

# Show the file content captured at the time of the action
bombadil inspect ab12 --show content

# Show captured stdout/stderr from a hook
bombadil inspect ab12 --logs
```

Example — inspect a symlink creation:

```bash
bombadil inspect ab12
```

```
ID:      ab12
Type:    Create
Time:    2026-03-22 14:30:01
Dot:     zsh
Target:  ~/.zshrc
Source:  .dots/zsh/zshrc
Status:  Success
```

Example — inspect a failed hook:

```bash
bombadil inspect e5f6 --logs
```

```
ID:      e5f6
Type:    HookExecuted
Time:    2026-03-21 09:12:44
Hook:    posthook: rehash
Exit:    1

--- stdout ---
(empty)

--- stderr ---
rehash: command not found
```

## bombadil revert

`bombadil revert <id>` undoes a recorded action. For a symlink creation, Bombadil removes the symlink and restores any backup that was made before the action ran.

```bash
# Revert a specific action
bombadil revert ab12

# Preview what would be changed without touching the filesystem
bombadil revert ab12 --dry-run

# Revert all recorded actions that touched a given file
bombadil revert --file ~/.zshrc

# Revert even if the file has been modified since the action was recorded
bombadil revert ab12 --force
```

### What revert does

| Action type | Revert behaviour |
|---|---|
| `Create` (symlink) | Removes the symlink; restores the previous file if a backup was taken |
| `Overwrite` | Restores the backed-up content |
| `HookExecuted` | Not revertable; Bombadil reports an error |
| `PackageInstalled` | Not revertable via revert; use `bombadil packages remove <name>` |

### The `--force` flag

By default, Bombadil refuses to revert an action if the target file has been modified after the action was recorded (the modification time or content has changed). Pass `--force` to override this check and revert anyway, discarding the current content.
