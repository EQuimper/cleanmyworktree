#!/usr/bin/env bash
# Cut a cmw release: bump the version, verify, commit, tag, push.
# The tag push starts .github/workflows/release.yml (dist), which builds the macOS binaries,
# creates the GitHub release and publishes the formula to EQuimper/homebrew-tap.
#
#   scripts/release.sh <patch|minor|major|X.Y.Z> [--dry-run]
#
# --dry-run runs every check and prints the plan without changing or pushing anything.
set -euo pipefail

REPO="EQuimper/cleanmyworktree"
BRANCH="main"

die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
step() { printf '\033[36m==>\033[0m %s\n' "$*"; }

[ $# -ge 1 ] || die "usage: scripts/release.sh <patch|minor|major|X.Y.Z> [--dry-run]"
bump="$1"
dry_run=false
[ "${2:-}" = "--dry-run" ] && dry_run=true

cd "$(git rev-parse --show-toplevel)"

current="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"
[[ "$current" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]] || die "can't read the version in Cargo.toml (got '$current')"
major="${BASH_REMATCH[1]}" minor="${BASH_REMATCH[2]}" patch="${BASH_REMATCH[3]}"

case "$bump" in
  patch) next="$major.$minor.$((patch + 1))" ;;
  minor) next="$major.$((minor + 1)).0" ;;
  major) next="$((major + 1)).0.0" ;;
  *)
    [[ "$bump" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must be patch, minor, major or X.Y.Z (got '$bump')"
    next="$bump"
    ;;
esac
tag="v$next"

# Strictly newer than Cargo.toml and every existing tag.
newer() { [ "$1" != "$2" ] && [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -1)" = "$1" ]; }
newer "$next" "$current" || die "$next is not newer than the current version $current"

step "Release $current → $next ($tag)$($dry_run && echo ' [dry run]')"

step "Checking the repository"
[ "$(git branch --show-current)" = "$BRANCH" ] || die "releases are cut from $BRANCH (you're on $(git branch --show-current))"
[ -z "$(git status --porcelain)" ] || die "working tree has changes; commit or stash them first"
git fetch --quiet --tags origin
[ "$(git rev-parse HEAD)" = "$(git rev-parse "origin/$BRANCH")" ] \
  || die "$BRANCH is not in sync with origin/$BRANCH; pull or push first"
latest_tag="$(git tag --list 'v[0-9]*' | sed 's/^v//' | sort -V | tail -1)"
if [ -n "$latest_tag" ]; then
  newer "$next" "$latest_tag" || die "$next is not newer than the latest tag v$latest_tag"
fi
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && die "tag $tag already exists"
command -v dist >/dev/null || die "dist is not installed: cargo install cargo-dist --locked"
gh auth status >/dev/null 2>&1 || die "gh is not logged in: gh auth login"

step "Formatting, lints, tests"
cargo fmt --check
cargo clippy --all-targets --quiet -- -D warnings
cargo test --quiet

step "Bumping Cargo.toml to $next"
# Until the release commit exists, any exit puts Cargo.toml/Cargo.lock back.
committed=false
trap '$committed || git checkout --quiet -- Cargo.toml Cargo.lock' EXIT
perl -0pi -e "s/^version = \"\Q$current\E\"\$/version = \"$next\"/m" Cargo.toml # first match only: [package]
grep -q "^version = \"$next\"\$" Cargo.toml || die "could not bump the version in Cargo.toml"
cargo build --quiet # refreshes Cargo.lock with the new version
[ "$(cargo run --quiet -- --version)" = "cmw $next" ] || die "built binary doesn't report $next"

step "Checking the dist plan for $tag"
dist plan --tag="$tag" >/dev/null

if $dry_run; then
  step "Dry run OK. Run without --dry-run to commit, tag and push $tag."
  exit 0
fi

step "Committing and tagging"
git commit --quiet -m "chore(release): $tag" -- Cargo.toml Cargo.lock
committed=true
git tag -a "$tag" -m "cmw $next"

step "Pushing $BRANCH and $tag"
git push --atomic origin "$BRANCH" "$tag"

step "Released $tag. The Release workflow is building it:"
echo "    https://github.com/$REPO/actions/workflows/release.yml"
echo "    gh run watch -R $REPO \$(gh run list -R $REPO --workflow release.yml --limit 1 --json databaseId -q '.[0].databaseId')"
