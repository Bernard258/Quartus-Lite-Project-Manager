#!/bin/sh
# Install a published qlm binary without requiring Rust or sudo.
set -eu

main() {
    if [ "$#" -gt 1 ]; then
        echo 'usage: install.sh [release-tag|latest]' >&2
        return 1
    fi
    repository=${QLM_REPOSITORY:-Bernard258/Quartus-Lite-Project-Manager}
    release=${1:-latest}
    case "$(uname -s):$(uname -m)" in
        Linux:x86_64|Linux:amd64) platform=linux-x86_64 ;;
        Darwin:x86_64) platform=macos-x86_64 ;;
        Darwin:arm64|Darwin:aarch64) platform=macos-aarch64 ;;
        *) echo 'qlm: supported release platforms are Linux x86_64 and macOS Intel/Apple Silicon.' >&2; return 1 ;;
    esac

    install_dir=${QLM_INSTALL_DIR:-"${HOME:?Set HOME or QLM_INSTALL_DIR}/.local/bin"}
    case "$install_dir" in
        /*) ;;
        *) echo 'qlm: QLM_INSTALL_DIR must be an absolute path.' >&2; return 1 ;;
    esac
    if command -v curl >/dev/null 2>&1; then
        downloader=curl
    elif command -v wget >/dev/null 2>&1; then
        downloader=wget
    else
        echo 'qlm: install curl or wget first.' >&2
        return 1
    fi

    if [ "$release" = latest ]; then
        url="https://github.com/$repository/releases/latest/download/qlm-$platform"
    else
        url="https://github.com/$repository/releases/download/$release/qlm-$platform"
    fi
    mkdir -p "$install_dir"
    # Stage beside the destination so replacement is atomic and failed downloads
    # leave any existing installation intact.
    staging=$(mktemp -d "$install_dir/.qlm-install.XXXXXX")
    trap 'rm -rf "$staging"' EXIT
    trap 'exit 1' HUP INT TERM
    echo "Downloading qlm for $platform..."
    if [ "$downloader" = curl ]; then
        curl -fL --retry 3 -o "$staging/qlm" "$url" || {
            echo 'qlm: download failed. Check your connection and that the requested release includes this platform.' >&2
            return 1
        }
    else
        wget -q -O "$staging/qlm" "$url" || {
            echo 'qlm: download failed. Check your connection and that the requested release includes this platform.' >&2
            return 1
        }
    fi
    if [ ! -s "$staging/qlm" ]; then
        echo 'qlm: downloaded binary is empty.' >&2
        return 1
    fi
    chmod 755 "$staging/qlm"
    "$staging/qlm" --version || {
        echo 'qlm: downloaded binary cannot run on this system; existing installation was not replaced.' >&2
        return 1
    }
    if [ -d "$install_dir/qlm" ]; then
        echo "qlm: $install_dir/qlm is a directory; cannot install." >&2
        return 1
    fi
    mv -f "$staging/qlm" "$install_dir/qlm"
    echo "Installed $install_dir/qlm"
    case ":$PATH:" in
        *":$install_dir:"*) ;;
        *) printf 'Add this directory to PATH in your shell configuration: %s\n' "$install_dir" ;;
    esac
}

main "$@"
