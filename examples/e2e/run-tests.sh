#!/bin/bash
# E2E Test Runner for bombadil
# Runs the Rust integration tests locally or in a container

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

usage() {
    cat << 'EOF'
Usage: run-tests.sh [OPTIONS] [-- CARGO_ARGS...]

Run bombadil e2e tests.

Options:
  --container       Run tests in a container instead of locally
  --rebuild         Rebuild container image before running
  -h, --help        Show this help

Examples:
  ./run-tests.sh                           # Run all tests locally
  ./run-tests.sh -- --nocapture            # Run with output
  ./run-tests.sh -- test_full_strategy     # Run specific test
  ./run-tests.sh --container               # Run in container
EOF
}

USE_CONTAINER=false
REBUILD=false
CARGO_ARGS=()

while [[ $# -gt 0 ]]; do
    case $1 in
        --container)
            USE_CONTAINER=true
            shift
            ;;
        --rebuild)
            REBUILD=true
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --)
            shift
            CARGO_ARGS=("$@")
            break
            ;;
        *)
            CARGO_ARGS+=("$1")
            shift
            ;;
    esac
done

echo "========================================"
echo "  Bombadil E2E Test Suite"
echo "========================================"
echo ""

if $USE_CONTAINER; then
    # Detect container runtime
    if command -v podman &> /dev/null; then
        CONTAINER_CMD="podman"
    elif command -v docker &> /dev/null; then
        CONTAINER_CMD="docker"
    else
        echo "Error: Neither podman nor docker found"
        exit 1
    fi

    IMAGE_NAME="bombadil-e2e-test"

    # Build image if needed
    if ! $CONTAINER_CMD image inspect "$IMAGE_NAME" &>/dev/null || $REBUILD; then
        echo "Building test container..."
        # Use a Rust image for running tests
        $CONTAINER_CMD build -t "$IMAGE_NAME" -f - "$REPO_ROOT" << 'DOCKERFILE'
FROM rust:1.75-slim

RUN apt-get update && apt-get install -y \
    tree \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace
DOCKERFILE
        echo ""
    fi

    echo "Running tests in container..."
    $CONTAINER_CMD run --rm \
        -v "$REPO_ROOT:/workspace:ro" \
        -v "$REPO_ROOT/target:/workspace/target:rw" \
        -w /workspace \
        "$IMAGE_NAME" \
        cargo test --test e2e "${CARGO_ARGS[@]}"
else
    echo "Running tests locally..."
    cd "$REPO_ROOT"
    cargo test --test e2e "${CARGO_ARGS[@]}"
fi
