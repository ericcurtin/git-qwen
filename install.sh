#!/bin/bash
set -euo pipefail

REPO="ericcurtin/git-qwen"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
TMP_DIR=""

cleanup() {
    if [ -n "$TMP_DIR" ] && [ -d "$TMP_DIR" ]; then
        rm -rf "$TMP_DIR"
    fi
}
trap cleanup EXIT

# Detect OS and architecture
detect_platform() {
    local os arch

    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  os="linux" ;;
        Darwin) os="macos" ;;
        *)
            echo "Error: Unsupported operating system: $os" >&2
            exit 1
            ;;
    esac

    case "$arch" in
        x86_64|amd64)   arch="x86_64" ;;
        arm64|aarch64)  arch="aarch64" ;;
        *)
            echo "Error: Unsupported architecture: $arch" >&2
            exit 1
            ;;
    esac

    echo "${os}-${arch}"
}

# Get the latest release tag (including prereleases if no stable release exists)
get_latest_release() {
    local tag
    # Try to get the latest stable release first
    tag=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null | \
        grep '"tag_name":' | \
        sed -E 's/.*"([^"]+)".*/\1/')

    # If no stable release, fall back to the most recent release (including prereleases)
    if [ -z "$tag" ]; then
        tag=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases" | \
            grep '"tag_name":' | \
            head -1 | \
            sed -E 's/.*"([^"]+)".*/\1/')
    fi

    echo "$tag"
}

main() {
    echo "Installing git-qwen..."

    local platform tag asset_name download_url

    platform="$(detect_platform)"
    echo "Detected platform: $platform"

    tag="$(get_latest_release)"
    if [ -z "$tag" ]; then
        echo "Error: Could not determine latest release" >&2
        exit 1
    fi
    echo "Latest release: $tag"

    asset_name="git-qwen-${platform}.tar.gz"
    download_url="https://github.com/${REPO}/releases/download/${tag}/${asset_name}"

    echo "Downloading ${download_url}..."

    TMP_DIR="$(mktemp -d)"

    curl -fsSL "$download_url" -o "${TMP_DIR}/git-qwen.tar.gz"

    echo "Extracting..."
    tar -xzf "${TMP_DIR}/git-qwen.tar.gz" -C "$TMP_DIR"

    echo "Installing to ${INSTALL_DIR}..."
    mkdir -p "$INSTALL_DIR"
    mv "${TMP_DIR}/git-qwen" "${INSTALL_DIR}/git-qwen"
    chmod +x "${INSTALL_DIR}/git-qwen"

    echo ""
    echo "git-qwen installed successfully to ${INSTALL_DIR}/git-qwen"

    # Check if INSTALL_DIR is in PATH
    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
        echo ""
        echo "NOTE: ${INSTALL_DIR} is not in your PATH."
        echo "Add it by running:"
        echo ""
        echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
        echo ""
        echo "Or add that line to your ~/.bashrc or ~/.zshrc"
    fi
}

main
