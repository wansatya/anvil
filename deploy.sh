#!/usr/bin/env bash
#
# Anvil deploy — cut a release without committing build output.
#
#   ./deploy.sh [vX.Y.Z] [--yes] [--dry-run]
#
# What it does:
#   1. Requires a clean git tree and passing tests.
#   2. Tags the release (default: v<version from crates/anvil-cli/Cargo.toml>).
#   3. Pushes the tag. GitHub Actions (see .github/workflows/release.yml)
#      then builds the release binaries on GitHub's machines and attaches
#      them to the GitHub Release — target/ is never committed.
#
# Users install from those binaries:
#   curl -fsSL https://raw.githubusercontent.com/<owner>/<repo>/main/install.sh | bash
#
set -eu

DRY_RUN=0
YES=0
TAG=""

for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --yes) YES=1 ;;
    -h|--help)
      sed -n '2,/^$/p' "$0" | sed 's/^# \?//'
      echo "Usage: $0 [vX.Y.Z] [--yes] [--dry-run]"
      exit 0
      ;;
    v*) TAG="$arg" ;;
    *) echo "error: unknown argument: $arg (see --help)" >&2; exit 1 ;;
  esac
done

log() { printf '%s\n' "==> $*"; }
die() { printf '%s\n' "error: $*" >&2; exit 1; }

command -v git >/dev/null || die "git not found"
command -v cargo >/dev/null || die "cargo not found (needed for pre-release tests)"

# 1. Clean tree (tracked files only — target/ and other ignored files are fine).
[ -z "$(git status --porcelain --untracked-files=no)" ] \
  || die "git tree is dirty — commit or stash first"

# 2. Tests gate the release.
log "running tests (cargo test --workspace)"
if [ "$DRY_RUN" = 1 ]; then
  echo "(dry-run: skipping tests)"
else
  cargo test --workspace --locked || die "tests failed — release aborted"
fi

# 3. Tag. Default comes from the binary crate version.
if [ -z "$TAG" ]; then
  VER="$(sed -nE 's/^version *= *"([^"]+)".*/\1/p' crates/anvil-cli/Cargo.toml | head -1)"
  [ -n "$VER" ] || die "could not read version from crates/anvil-cli/Cargo.toml"
  TAG="v$VER"
fi
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null \
  && die "tag $TAG already exists locally"
log "release tag: $TAG"

if [ "$DRY_RUN" = 1 ]; then
  echo "(dry-run: would create + push tag $TAG, Actions builds binaries)"
  exit 0
fi

if [ "$YES" = 0 ] && [ -t 0 ]; then
  printf 'Create and push tag %s? [y/N] ' "$TAG"
  read -r answer
  [ "$answer" = "y" ] || die "aborted"
fi

git tag -a "$TAG" -m "anvil $TAG"
git push origin "$TAG"

REPO="$(git remote get-url origin | sed -nE 's#.*github\.com[:/]([^/]+/[^/]+?)(\.git)?$#\1#p')"
log "tag pushed — Actions is building now"
log "watch it: https://github.com/$REPO/actions"
log "release lands at: https://github.com/$REPO/releases/tag/$TAG"
log "users install with:"
echo "  curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | bash"
