#!/usr/bin/env bash
#
# Anvil deploy — cut a release without committing build output.
#
#   ./deploy.sh [vX.Y.Z] [--yes] [--dry-run]
#
# What it does:
#   1. Requires a clean git tree.
#   2. Syncs the repo version to the tag: bumps `version` in every
#      crates/*/Cargo.toml (+ Cargo.lock) so the TUI footer, `anvil version`
#      and the git tag all agree. Skipped when they already match.
#   3. Runs the test suite as a gate, commits the bump as "release vX.Y.Z".
#   4. Pushes the branch (if a bump commit was made) and the tag.
#      GitHub Actions (see .github/workflows/release.yml) then builds the
#      release binaries on GitHub's machines and attaches them to the
#      GitHub Release — target/ is never committed.
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

# Tag: explicit arg wins, else v<version from the binary crate>.
if [ -z "$TAG" ]; then
  VER="$(sed -nE 's/^version *= *"([^"]+)".*/\1/p' crates/anvil-cli/Cargo.toml | head -1)"
  [ -n "$VER" ] || die "could not read version from crates/anvil-cli/Cargo.toml"
  TAG="v$VER"
fi
echo "$TAG" | grep -qE '^v[0-9]+\.[0-9]+\.[0-9]+$' \
  || die "tag must look like vX.Y.Z, got: $TAG"
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null \
  && die "tag $TAG already exists locally"
log "release tag: $TAG"

# Clean tree before touching anything (tracked files only — target/ etc. are fine).
[ -z "$(git status --porcelain --untracked-files=no)" ] \
  || die "git tree is dirty — commit or stash first"

VER="${TAG#v}"
CUR="$(sed -nE 's/^version *= *"([^"]+)".*/\1/p' crates/anvil-cli/Cargo.toml | head -1)"
BUMPED=0

# Sync repo version to the tag so footer + `anvil version` + tag agree.
if [ "$VER" != "$CUR" ]; then
  log "bumping version $CUR -> $VER"
  if [ "$DRY_RUN" = 1 ]; then
    echo "(dry-run: would set version = \"$VER\" in crates/*/Cargo.toml + Cargo.lock)"
  else
    for f in crates/*/Cargo.toml; do
      sed -i -E "s/^version *= *\".*\"/version = \"$VER\"/" "$f"
    done
    # Re-resolve path crates into the lockfile (no dependency upgrades).
    cargo metadata --format-version=1 >/dev/null
    git diff --stat
    BUMPED=1
  fi
else
  log "repo version already $VER — no bump needed"
fi

# Tests gate the release (against the exact code being tagged).
log "running tests (cargo test --workspace)"
if [ "$DRY_RUN" = 1 ]; then
  echo "(dry-run: skipping tests)"
else
  cargo test --workspace --locked || die "tests failed — release aborted"
fi

if [ "$DRY_RUN" = 1 ]; then
  echo "(dry-run: would commit bump (if any), push branch + tag $TAG, Actions builds binaries)"
  exit 0
fi

if [ "$BUMPED" = 1 ]; then
  git add crates/*/Cargo.toml Cargo.lock
  git commit -q -m "release $TAG"
  log "committed version bump"
fi

if [ "$YES" = 0 ] && [ -t 0 ]; then
  printf 'Create and push tag %s? [y/N] ' "$TAG"
  read -r answer
  [ "$answer" = "y" ] || die "aborted"
fi

if [ "$BUMPED" = 1 ]; then
  git push origin HEAD
fi
git tag -a "$TAG" -m "anvil $TAG"
git push origin "$TAG"

REPO="$(git remote get-url origin | sed -nE 's#.*github\.com[:/]([^/]+/[^/]+?)(\.git)?$#\1#p')"
log "tag pushed — Actions is building now"
log "watch it: https://github.com/$REPO/actions"
log "release lands at: https://github.com/$REPO/releases/tag/$TAG"
log "users install with:"
echo "  curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | bash"
