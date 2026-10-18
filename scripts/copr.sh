#!/usr/bin/env bash
# Submits a published release to Fedora COPR (packaging/copr/jellybridge.spec):
#
#   scripts/copr.sh srpm <version> <dir>     downloads the release sources the
#                                            spec names and builds the SRPM in <dir>
#   scripts/copr.sh submit <version> <dir>   builds the SRPM and sends it to COPR
#
# Needs rpm-build, rpmdevtools (spectool) and, for submit, copr-cli with
# credentials: a ~/.config/copr file, or COPR_LOGIN, COPR_USERNAME and
# COPR_TOKEN (from https://copr.fedorainfracloud.org/api/). COPR_PROJECT
# names the project (default oddbench/jellybridge). Run it on Fedora or in a
# fedora container, as the release workflow does.
set -euo pipefail
PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
CMD="${1:-}"; VERSION="${2:-}"; DIR="${3:-}"
[[ -n "$CMD" && -n "$VERSION" && -n "$DIR" ]] || { sed -n '2,13p' "$0"; exit 2; }
PROJECT="${COPR_PROJECT:-oddbench/jellybridge}"
SPEC="$DIR/SPECS/jellybridge.spec"

srpm() {
    mkdir -p "$DIR/SPECS" "$DIR/SOURCES" "$DIR/SRPMS"
    local spec_version
    spec_version="$(sed -n 's/^Version: *//p' "$PROJECT_DIR/packaging/copr/jellybridge.spec")"
    [[ "$spec_version" == "$VERSION" ]] || { echo "packaging/copr/jellybridge.spec says $spec_version, not $VERSION" >&2; exit 1; }
    cp "$PROJECT_DIR/packaging/copr/jellybridge.spec" "$SPEC"
    spectool -g -C "$DIR/SOURCES" "$SPEC"
    rpmbuild -bs --define "_topdir $DIR" "$SPEC"
    ls "$DIR"/SRPMS/jellybridge-"$VERSION"-*.src.rpm
}

submit() {
    command -v copr-cli >/dev/null || { echo "submit needs copr-cli" >&2; exit 2; }
    if [[ ! -s "$HOME/.config/copr" ]]; then
        [[ -n "${COPR_LOGIN:-}" && -n "${COPR_USERNAME:-}" && -n "${COPR_TOKEN:-}" ]] \
            || { echo "no ~/.config/copr and COPR_LOGIN/COPR_USERNAME/COPR_TOKEN are not set" >&2; exit 2; }
        mkdir -p "$HOME/.config"
        (umask 077; printf '[copr-cli]\nlogin = %s\nusername = %s\ntoken = %s\ncopr_url = https://copr.fedorainfracloud.org\n' \
            "$COPR_LOGIN" "$COPR_USERNAME" "$COPR_TOKEN" > "$HOME/.config/copr")
    fi
    local file
    file="$(srpm | tail -1)"
    # Waits for the build and fails with it, so the workflow shows the result.
    copr-cli build "$PROJECT" "$file"
}

case "$CMD" in
    srpm) srpm ;;
    submit) submit ;;
    *) echo "unknown command: $CMD" >&2; exit 2 ;;
esac
