#!/bin/bash
# Post-link hook - runs after dotfiles are linked
# Demonstrates hook functionality

echo "=== Post-link hook ==="
echo "Dotfiles linked successfully!"

# Example: Reload configurations
# sway reload 2>/dev/null || true
# tmux source-file ~/.tmux.conf 2>/dev/null || true

echo "Post-link hook complete"
