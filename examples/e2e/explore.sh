#!/bin/bash
# Interactive exploration mode for bombadil testing
# Launches a container where you can safely experiment with dotfiles

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
EXAMPLES_DIR="$REPO_ROOT/examples"

# Check for podman or docker
if command -v podman &> /dev/null; then
    CONTAINER_CMD="podman"
elif command -v docker &> /dev/null; then
    CONTAINER_CMD="docker"
else
    echo "Error: Neither podman nor docker found"
    exit 1
fi

IMAGE_NAME="bombadil-e2e-test"
RUNTIME_IMAGE_NAME="bombadil-e2e-runtime"

echo "========================================"
echo "  Bombadil Interactive Exploration"
echo "========================================"
echo ""
echo "Container runtime: $CONTAINER_CMD"
echo ""

# Build the full image (with builder stage)
build_image() {
    echo "Building container image (compiling inside container)..."
    $CONTAINER_CMD build -t "$IMAGE_NAME" -f "$SCRIPT_DIR/Containerfile" "$REPO_ROOT"
    echo ""
}

# Build only the runtime image (uses host binary)
build_runtime_image() {
    echo "Building runtime image (fast, uses host binary)..."
    $CONTAINER_CMD build -t "$RUNTIME_IMAGE_NAME" --target runtime -f "$SCRIPT_DIR/Containerfile" "$REPO_ROOT"
    echo ""
}

# Check if image exists
image_exists() {
    $CONTAINER_CMD image inspect "$IMAGE_NAME" &>/dev/null
}

runtime_image_exists() {
    $CONTAINER_CMD image inspect "$RUNTIME_IMAGE_NAME" &>/dev/null
}

# Find host binary
find_host_binary() {
    if [[ -x "$REPO_ROOT/target/release/bombadil" ]]; then
        echo "$REPO_ROOT/target/release/bombadil"
    elif [[ -x "$REPO_ROOT/target/debug/bombadil" ]]; then
        echo "$REPO_ROOT/target/debug/bombadil"
    else
        echo ""
    fi
}

# Parse arguments
REBUILD=false
USE_HOST_BINARY=auto  # auto, yes, no
PROFILE=""
EDIT_MODE=false
SCRIPT=""
SCRIPT_FILE=""
while [[ $# -gt 0 ]]; do
    case $1 in
        --rebuild)
            REBUILD=true
            shift
            ;;
        --profile)
            PROFILE="$2"
            shift 2
            ;;
        --edit)
            EDIT_MODE=true
            shift
            ;;
        --script)
            SCRIPT="$2"
            shift 2
            ;;
        --script-file)
            SCRIPT_FILE="$2"
            shift 2
            ;;
        --host-binary)
            USE_HOST_BINARY=yes
            shift
            ;;
        --container-build)
            USE_HOST_BINARY=no
            shift
            ;;
        -h|--help)
            cat << 'EOF'
Usage: explore.sh [OPTIONS]

Launch an interactive container for experimenting with bombadil dotfiles.

Options:
  --rebuild           Force rebuild of container image
  --profile NAME      Pre-apply dotfiles with specified profile
  --edit              Edit mode: mount examples read-write (changes persist to host)
  --script "CMD"      Run command(s) in container and exit (non-interactive)
  --script-file FILE  Run script file in container and exit (non-interactive)
  --host-binary       Use pre-built binary from host (fast, default if available)
  --container-build   Force building inside container (slow but portable)
  -h, --help          Show this help

Default behavior:
  - Uses host binary if available (target/release/bombadil or target/debug/bombadil)
  - Falls back to container build if no host binary found
  - Examples are copied into the container (read-only source)
  - Changes in the container don't affect host files
  - Home directory is isolated

Edit mode (--edit):
  - Examples are mounted read-write
  - Changes in the container persist to the host
  - Useful for developing new examples

Script mode (--script or --script-file):
  - Runs non-interactively
  - Outputs command results
  - Useful for CI/testing
EOF
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

# Determine if we should use host binary
HOST_BINARY=$(find_host_binary)
if [[ "$USE_HOST_BINARY" == "auto" ]]; then
    if [[ -n "$HOST_BINARY" ]]; then
        USE_HOST_BINARY=yes
        echo "Found host binary: $HOST_BINARY"
    else
        USE_HOST_BINARY=no
        echo "No host binary found, will build in container"
    fi
fi

# Build appropriate image
if [[ "$USE_HOST_BINARY" == "yes" ]]; then
    if [[ -z "$HOST_BINARY" ]]; then
        echo "Error: --host-binary specified but no binary found"
        echo "Run 'cargo build --release' first"
        exit 1
    fi
    if ! runtime_image_exists || $REBUILD; then
        build_runtime_image
    fi
    ACTIVE_IMAGE="$RUNTIME_IMAGE_NAME"
    BINARY_MOUNT="-v $HOST_BINARY:/usr/local/bin/bombadil:ro"
else
    if ! image_exists || $REBUILD; then
        build_image
    fi
    ACTIVE_IMAGE="$IMAGE_NAME"
    BINARY_MOUNT=""
fi
echo ""

# Setup message
cat << 'EOF'
Starting interactive container...

Inside the container:
  - Your home directory is isolated (changes won't affect your real home)
  - ~/dotfiles     - Bombadil configuration
  - ~/.config      - Where dotfiles get linked

Commands to try:
  bombadil link --force                    # Apply dotfiles
  bombadil link --force --profiles laptop  # Apply with profile
  tree ~/.config                           # See linked files
  cat ~/.config/full-simple/config.conf    # Check a linked file

EOF

# Script mode (non-interactive)
if [[ -n "$SCRIPT" || -n "$SCRIPT_FILE" ]]; then
    # Determine what to run
    if [[ -n "$SCRIPT_FILE" ]]; then
        if [[ ! -f "$SCRIPT_FILE" ]]; then
            echo "Error: Script file not found: $SCRIPT_FILE"
            exit 1
        fi
        USER_SCRIPT=$(cat "$SCRIPT_FILE")
    else
        USER_SCRIPT="$SCRIPT"
    fi

    # Build profile command if specified
    PROFILE_CMD=""
    if [[ -n "$PROFILE" ]]; then
        PROFILE_CMD="bombadil link --force --profiles $PROFILE &&"
    fi

    $CONTAINER_CMD run --rm \
        --hostname "bombadil-explore" \
        --security-opt label=disable \
        $BINARY_MOUNT \
        -v "$EXAMPLES_DIR:/examples:ro" \
        -w /home/testuser \
        "$ACTIVE_IMAGE" \
        bash -c "
            # Copy base files to home (simulates existing user files)
            cp -r /examples/base/. ~/ 2>/dev/null || true
            # Copy dotfiles
            cp -r /examples/dotfiles ~/dotfiles
            # Copy expectations for reference
            cp -r /examples/expectations ~/expectations 2>/dev/null || true
            mkdir -p ~/dotfiles/.dots ~/.config ~/.local/bin
            bombadil install ~/dotfiles 2>/dev/null
            $PROFILE_CMD
            # Run user script
            $USER_SCRIPT
        "
    exit $?
fi

if $EDIT_MODE; then
    echo "Edit mode: Changes will persist to host filesystem"
    echo ""

    $CONTAINER_CMD run -it --rm \
        --hostname "bombadil-explore" \
        --security-opt label=disable \
        $BINARY_MOUNT \
        -v "$EXAMPLES_DIR/base:/mnt/base:ro" \
        -v "$EXAMPLES_DIR/dotfiles:/home/testuser/dotfiles:rw" \
        -v "$EXAMPLES_DIR/expectations:/home/testuser/expectations:ro" \
        -w /home/testuser \
        "$ACTIVE_IMAGE" \
        bash -c '
            # Copy base files to home (simulates existing user files)
            cp -r /mnt/base/. ~/ 2>/dev/null || true
            mkdir -p ~/dotfiles/.dots ~/.config ~/.local/bin
            bombadil install ~/dotfiles
            echo "Ready! Run: bombadil link --force"
            exec bash
        '
else
    echo "Read-only mode: Changes won't affect host filesystem"
    echo ""

    # Build profile command if specified
    PROFILE_CMD=""
    if [[ -n "$PROFILE" ]]; then
        PROFILE_CMD="bombadil link --force --profiles $PROFILE && echo 'Profile $PROFILE applied!' &&"
    fi

    $CONTAINER_CMD run -it --rm \
        --hostname "bombadil-explore" \
        --security-opt label=disable \
        $BINARY_MOUNT \
        -v "$EXAMPLES_DIR:/examples:ro" \
        -w /home/testuser \
        "$ACTIVE_IMAGE" \
        bash -c "
            # Copy base files to home (simulates existing user files)
            cp -r /examples/base/. ~/ 2>/dev/null || true
            # Copy dotfiles
            cp -r /examples/dotfiles ~/dotfiles
            # Copy expectations for reference
            cp -r /examples/expectations ~/expectations 2>/dev/null || true
            mkdir -p ~/dotfiles/.dots ~/.config ~/.local/bin
            bombadil install ~/dotfiles
            $PROFILE_CMD
            echo 'Ready! Run: bombadil link --force'
            exec bash
        "
fi

echo ""
echo "Container session ended."
