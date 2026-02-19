#!/bin/sh
# OpenCrust installer — https://github.com/opencrust-org/opencrust
# Usage: curl -fsSL https://raw.githubusercontent.com/opencrust-org/opencrust/main/install.sh | sh
set -eu

REPO="opencrust-org/opencrust"
BINARY="opencrust"

main() {
    detect_platform
    get_latest_version
    download_and_verify
    install_binary
    echo ""
    echo "OpenCrust $VERSION installed to $INSTALL_PATH"
    echo ""
    echo "Next steps:"
    echo "  opencrust init    # interactive setup wizard"
    echo "  opencrust start   # start the gateway"
}

detect_platform() {
    OS="$(uname -s)"
    ARCH="$(uname -m)"

    case "$OS" in
        Linux)  OS_NAME="linux" ;;
        Darwin) OS_NAME="macos" ;;
        *)      error "Unsupported OS: $OS. Only Linux and macOS are supported." ;;
    esac

    case "$ARCH" in
        x86_64|amd64)   ARCH_NAME="x86_64" ;;
        aarch64|arm64)   ARCH_NAME="aarch64" ;;
        *)               error "Unsupported architecture: $ARCH" ;;
    esac

    ARTIFACT="${BINARY}-${OS_NAME}-${ARCH_NAME}"
    echo "Detected platform: ${OS_NAME}-${ARCH_NAME}"
}

get_latest_version() {
    VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' \
        | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')

    if [ -z "$VERSION" ]; then
        error "Failed to fetch latest release version"
    fi

    echo "Latest version: $VERSION"
}

download_and_verify() {
    BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"
    TMPDIR=$(mktemp -d)
    trap 'rm -rf "$TMPDIR"' EXIT

    echo "Downloading ${ARTIFACT}..."
    curl -fsSL "${BASE_URL}/${ARTIFACT}" -o "${TMPDIR}/${ARTIFACT}"
    curl -fsSL "${BASE_URL}/${ARTIFACT}.sha256" -o "${TMPDIR}/${ARTIFACT}.sha256"

    echo "Verifying checksum..."
    cd "$TMPDIR"
    if ! shasum -a 256 -c "${ARTIFACT}.sha256" >/dev/null 2>&1; then
        error "Checksum verification failed"
    fi
    cd - >/dev/null
    echo "Checksum verified."
}

install_binary() {
    # Priority: $OPENCRUST_INSTALL_DIR > ~/.local/bin (if in PATH) > /usr/local/bin
    if [ -n "${OPENCRUST_INSTALL_DIR:-}" ]; then
        INSTALL_DIR="$OPENCRUST_INSTALL_DIR"
    elif echo "$PATH" | tr ':' '\n' | grep -qx "$HOME/.local/bin"; then
        INSTALL_DIR="$HOME/.local/bin"
    else
        INSTALL_DIR="/usr/local/bin"
    fi

    mkdir -p "$INSTALL_DIR"
    INSTALL_PATH="${INSTALL_DIR}/${BINARY}"

    if [ "$INSTALL_DIR" = "/usr/local/bin" ] && [ "$(id -u)" -ne 0 ]; then
        echo "Installing to ${INSTALL_DIR} (requires sudo)..."
        sudo cp "${TMPDIR}/${ARTIFACT}" "$INSTALL_PATH"
        sudo chmod +x "$INSTALL_PATH"
    else
        cp "${TMPDIR}/${ARTIFACT}" "$INSTALL_PATH"
        chmod +x "$INSTALL_PATH"
    fi
}

error() {
    echo "error: $1" >&2
    exit 1
}

main
