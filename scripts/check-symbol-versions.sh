#!/bin/bash
# Fails when any given ELF file needs a newer glibc or libstdc++ than
# Raspberry Pi OS Bookworm has (GLIBC_2.36, GLIBCXX_3.4.30).
# Usage: scripts/check-symbol-versions.sh FILE...
set -euo pipefail

LIMITS="GLIBC:2.36 GLIBCXX:3.4.30"

highest_version() { # highest_version <prefix> <file>
    readelf --dyn-syms --wide "$2" \
        | { grep -oE "@${1}_[0-9]+(\.[0-9]+)*" || true; } \
        | sed "s/^@${1}_//" | sort -V | tail -n 1
}

is_newer() { # is_newer <version> <limit>
    [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n 1)" = "$1" ]
}

if [ "$#" -eq 0 ]; then
    echo "usage: $0 FILE..." >&2
    exit 2
fi
for file in "$@"; do
    readelf -h "$file" >/dev/null 2>&1 || {
        echo "error: $file is not a readable ELF file" >&2
        exit 1
    }
done
for file in "$@"; do
    for limit in $LIMITS; do
        prefix="${limit%%:*}"
        highest="$(highest_version "$prefix" "$file")"
        if [ -n "$highest" ] && is_newer "$highest" "${limit#*:}"; then
            echo "error: $file needs ${prefix}_$highest; the Pi has ${prefix}_${limit#*:}" >&2
            exit 1
        fi
    done
done
echo "Symbol versions ok: $*"
