# Expectations Directory

This directory contains the expected state after running `bombadil link` with different profiles.
The e2e tests compare the actual results against these expectations.

## Structure

```
expectations/
├── default/              # Expected state with no profile
├── profile-laptop/       # Expected state with --profile laptop
└── profile-work/         # Expected state with --profile work
```

## How Expectations Work

1. Each profile directory mirrors the structure of `~/.config` and `~` after linking
2. Files contain the expected rendered content (with variables substituted)
3. Symlinks are represented as regular files with the expected content
4. The e2e tests diff the actual result against these expectations

## Updating Expectations

When bombadil behavior changes, update expectations by:
1. Running `bombadil link` in the test container
2. Copying the relevant files to the expectations directory
3. Verifying the changes are intentional
