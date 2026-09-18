#!/bin/sh
# Remove only the qlm binary installed in the selected directory.
set -eu

main() {
    install_dir=${QLM_INSTALL_DIR:-"${HOME:?Set HOME or QLM_INSTALL_DIR}/.local/bin"}
    case "$install_dir" in
        /*) ;;
        *) echo 'qlm: QLM_INSTALL_DIR must be an absolute path.' >&2; return 1 ;;
    esac
    binary="$install_dir/qlm"
    if [ -L "$binary" ] || [ -f "$binary" ]; then
        rm "$binary"
        printf 'Removed %s\n' "$binary"
    elif [ -e "$binary" ]; then
        printf 'qlm: refusing to remove non-file %s\n' "$binary" >&2
        return 1
    else
        printf 'qlm is not installed at %s\n' "$binary"
    fi
}

main "$@"
