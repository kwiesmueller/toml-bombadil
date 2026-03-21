### DOTFILES MANAGED START ###
# Prepended by bombadil inject strategy
# This content appears BEFORE original bashrc content

# Environment setup
export DOTFILES_MANAGED="true"
export PATH="$HOME/.local/bin:$PATH"
### DOTFILES MANAGED END ###
# User's existing bashrc - inject strategy will add to this
# without destroying existing content

# Original user customizations
export EDITOR="nano"
alias ll="ls -l"

# End of original content
### DOTFILES MANAGED START ###
# Appended by bombadil inject strategy
# This content appears AFTER original bashrc content

# Custom aliases
alias dotfiles="cd ~/dotfiles"
alias reload="source ~/.bashrc"

# Editor setup from variables
export EDITOR="nvim"
export VISUAL="nvim"

# Theme colors available as env vars
export THEME_BG="#1a1b26"
export THEME_FG="#c0caf5"
### DOTFILES MANAGED END ###
