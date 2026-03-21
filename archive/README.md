# Archived Legacy Code

This directory contains legacy code from the v3 architecture that has been superseded by the v4 action-based auditing system.

## Contents

### dots_legacy.rs
The original dotfile installation logic that was in `src/dots.rs`. This was replaced by:
- `src/dots/strategy/` - New trait-based installation strategies (full, patch, inject, semantic)
- `src/audit/` - Action-based auditing system that tracks all changes

### state.rs
The original `BombadilState` that tracked symlinks via `previous_state.toml`. This was replaced by:
- `src/audit/storage.rs` - Session and action persistence with content snapshots
- `src/audit/session.rs` - Session management with detailed action tracking

## Migration Notes

The v4 architecture provides:
1. **Action tracking** - Every change is recorded with a unique ID
2. **Revert capability** - Individual actions can be reverted
3. **Diff inspection** - Before/after content can be compared
4. **Multiple strategies** - Full replacement, patching, injection, semantic patching

## Date Archived
2024-02-04
