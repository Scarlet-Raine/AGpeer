#!/bin/sh
# agpeer container entrypoint.
#
# Linuxserver-style PUID/PGID support: the container starts as root, ensures
# a user/group with the requested IDs exists, aligns ownership of the
# writable volumes, then drops privileges before exec'ing agpeer. The
# process that runs the binary (and holds the Soulseek credentials) is
# always unprivileged.
#
# Defaults match the image's historical `USER nobody`: 65534:65534.
# Unraid users typically set PUID=99 PGID=100.
set -e

if [ "$(id -u)" = "0" ]; then
    PUID="${PUID:-65534}"
    PGID="${PGID:-65534}"

    if ! getent group "$PGID" >/dev/null 2>&1; then
        addgroup --gid "$PGID" agpeer
    fi
    GROUP_NAME="$(getent group "$PGID" | cut -d: -f1)"
    if ! getent passwd "$PUID" >/dev/null 2>&1; then
        adduser --quiet --uid "$PUID" --ingroup "$GROUP_NAME" --system \
            --no-create-home --home /nonexistent agpeer
    fi

    # Align ownership of writable volumes with the runtime user. Only chown
    # when the top-level owner actually differs, so a large library is not
    # walked on every start.
    for dir in /data /opt/agpeer; do
        if [ -d "$dir" ] && [ "$(stat -c %u "$dir")" != "$PUID" ]; then
            chown -R "$PUID:$PGID" "$dir"
        fi
    done

    exec gosu "$PUID:$PGID" agpeer "$@"
fi

# Not root (an explicit `user:` in compose): honor it as-is.
exec agpeer "$@"
