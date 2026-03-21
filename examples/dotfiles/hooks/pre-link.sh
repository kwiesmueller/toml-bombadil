#!/bin/bash
# Pre-link hook - runs before dotfiles are linked
# Demonstrates hook functionality

echo "=== Pre-link hook ==="
echo "Preparing to link dotfiles..."
echo "Dotfiles path: $BOMBADIL_DOTFILES_PATH"

# Example: Create required directories
mkdir -p ~/.config
mkdir -p ~/.local/bin

echo "Pre-link hook complete"
