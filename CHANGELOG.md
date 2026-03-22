# Changelog

All notable changes to this project will be documented in this file.
See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

- - -

## [4.0.0-alpha.1] - 2026-03-22

### Breaking Changes

Complete rewrite of the config format and architecture (v4). The previous v3
`bombadil.toml` format is no longer the primary interface. Use
`bombadil migrate` to convert existing v3 configs to v4 `dots.toml` format.

### Added

- `dots.toml` per-directory config format with `[dot.files]`, `[dot.packages]`, `[dot.profiles]`
- `bombadil sync` — unified command combining dotfile linking and package installation
- `bombadil migrate` — converts v3 `bombadil.toml` + imports to v4 `dots.toml` files
- `bombadil drift` — detects packages installed on the system but not in config
- `bombadil log` / `bombadil inspect` / `bombadil revert` — full audit trail for all file operations
- Extended package manager support: dnf, apt, brew, pacman, cargo, flatpak with repo setup
- `go install` and binary package types
- `hard_copy_target` for dotfiles that require a real copy (e.g. SDDM/Wayland)
- `bombadil schema` command to generate JSON Schema for editor autocomplete
- Container-based e2e test suite covering all major workflows

### Changed

- CLI subcommand renamed: `bombadil-sync` → `bombadil sync`
- Package manager config extended from plain strings to structured `Extended` variant
  supporting `repo`, `repo_url`, `gpg_key` fields

- - -

## v3.x and earlier

See git history for v3.x changelog. v3 was the original upstream release series
from [oknozor/toml-bombadil](https://github.com/oknozor/toml-bombadil).
