+++
title = "Declaring Packages"
description = "How to declare system packages in dots.toml."
date = 2026-03-22
updated = 2026-03-22
draft = false
weight = 10
template = "docs/page.html"

[extra]
lead = "Declare packages in dots.toml alongside your dotfiles. Bombadil reads the active package manager and installs the right package for the current machine."
toc = true
top = false
+++

## Basic declaration

```toml
[dot.packages.ripgrep]

[dot.packages.ripgrep.install]
dnf    = "ripgrep"
apt    = "ripgrep"
brew   = "ripgrep"
pacman = "ripgrep"
cargo  = "ripgrep"
```

`[dot.packages.<name>]` declares the package. The name (`ripgrep` here) is the canonical cross-platform identifier — it is what Bombadil uses in logs, drift reports, and the `bombadil packages list` output.

`[dot.packages.<name>.install]` maps each package manager to the package name to install. Bombadil detects which manager is available on the current machine and uses the matching entry.

## Omitting the install name

If the package name is the same across all managers, you can omit the `install` block entirely. Bombadil falls back to the canonical name as the package name for the active manager:

```toml
# Both of these are equivalent when the package name matches everywhere:

[dot.packages.htop]

# is the same as:

[dot.packages.htop]
[dot.packages.htop.install]
dnf    = "htop"
apt    = "htop"
brew   = "htop"
pacman = "htop"
```

Only list manager-specific names when they differ from the canonical name.

## Extended install: repo files (dnf)

Some packages require a repository to be added before installation. Pass a table instead of a string to include a repo file:

```toml
[dot.packages.kubectl.install]
dnf = { package = "kubectl", repo = "k8s/kubectl.repo" }
```

`repo` is a path relative to your dotfiles root. Bombadil copies it to `/etc/yum.repos.d/<filename>` before running `dnf install`.

## Extended install: repo URL and GPG key (apt)

```toml
[dot.packages.docker.install]
apt = { package = "docker-ce", repo_url = "https://download.docker.com/linux/ubuntu", gpg_key = "https://download.docker.com/linux/ubuntu/gpg" }
```

Bombadil adds the GPG key and the repository URL via `apt-key` and `/etc/apt/sources.list.d/` before installing.

## Tags: conditional installation

```toml
[dot.packages.ripgrep]
tags = ["cli"]

[dot.packages.ripgrep.install]
dnf   = "ripgrep"
cargo = "ripgrep"
```

A package tagged `cli` is only installed when `cli` is in the active tag set. Activate tags via `--tags cli,tools` or through a profile. Packages without a `tags` field are always included.

## Prehooks and posthooks

Run commands before or after a package is installed:

```toml
[dot.packages.nvm]
prehooks  = ["curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.7/install.sh | bash"]
posthooks = ["nvm install --lts"]

[dot.packages.nvm.install]
# nvm is installed via prehook; no package manager entry needed
```

Hooks run in the order listed. A non-zero exit from a prehook aborts the install. A non-zero exit from a posthook is recorded in the audit log but does not roll back the installation.

## Full example

```toml
[dot.packages.ripgrep]
tags      = ["cli"]
prehooks  = []
posthooks = []

[dot.packages.ripgrep.install]
dnf    = "ripgrep"
apt    = "ripgrep"
brew   = "ripgrep"
pacman = "ripgrep"
cargo  = "ripgrep"

[dot.packages.kubectl]
tags = ["k8s"]

[dot.packages.kubectl.install]
dnf = { package = "kubectl", repo = "k8s/kubectl.repo" }
apt = { package = "kubectl", repo_url = "https://apt.kubernetes.io/", gpg_key = "https://packages.cloud.google.com/apt/doc/apt-key.gpg" }
brew = "kubectl"

[dot.packages.docker]
tags = ["containers"]

[dot.packages.docker.install]
dnf = "docker-ce"
apt = { package = "docker-ce", repo_url = "https://download.docker.com/linux/ubuntu", gpg_key = "https://download.docker.com/linux/ubuntu/gpg" }
brew = "docker"
```
