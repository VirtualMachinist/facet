#!/usr/bin/env bash
# Version truth gate for Facet.
#
# Facet carries three independent version axes (see Cargo.toml):
#   1. Facet CLI     -- [workspace.package].version; the git tag `vX.Y.Z`.
#   2. facet-lattice -- pinned in crates/lattice/Cargo.toml (crates.io identity).
#   3. probe-*       -- pinned per probe crate (vendored Probe cut).
#
# This script gates axis 1 only, and asserts the three sources agree:
#
#   git tag  ==  Cargo (facet-cli)  ==  `facet --version`
#
# A release that disagrees on any pair is a lie about what a stranger installed,
# so this exits non-zero and CI/release fails. It NEVER edits a version; fixing a
# mismatch is a deliberate human bump, not an automatic rewrite.
#
# Usage:
#   scripts/version-check.sh [--tag <vX.Y.Z>] [--binary <path/to/facet>]
#
#   --tag     Compare against a release tag (leading `v` optional). In GitHub
#             Actions on a tag push, pass "$GITHUB_REF_NAME".
#   --binary  Compare against a built binary's `--version` output.
#
# With neither flag it prints the Cargo version and exits 0, which is how you ask
# "what version does this worktree claim?" without building anything.

set -euo pipefail

die() {
  echo "version-check: $*" >&2
  exit 1
}

tag=""
binary=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag)
      [[ $# -ge 2 ]] || die "--tag needs a value"
      tag="$2"
      shift 2
      ;;
    --binary)
      [[ $# -ge 2 ]] || die "--binary needs a value"
      binary="$2"
      shift 2
      ;;
    -h | --help)
      sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      die "unknown argument: $1"
    ;;
  esac
done

git rev-parse --show-toplevel >/dev/null 2>&1 || die "must run inside the git worktree"
cd "$(git rev-parse --show-toplevel)"

# Axis 1, from Cargo. facet-cli is the crate that owns the `facet` binary, so it
# is the only correct source here -- NOT probe-cli, which is pinned separately.
cargo_version="$(cargo pkgid -p facet-cli | sed 's/.*[@#]//')"
[[ -n "$cargo_version" ]] || die "could not read the facet-cli version from cargo"

echo "cargo (facet-cli):  ${cargo_version}"

failed=0

if [[ -n "$tag" ]]; then
  tag_version="${tag#v}"
  echo "git tag:            ${tag}  (version ${tag_version})"
  if [[ "$tag_version" != "$cargo_version" ]]; then
    echo "MISMATCH: git tag ${tag} says ${tag_version}, Cargo says ${cargo_version}" >&2
    failed=1
  fi
fi

if [[ -n "$binary" ]]; then
  [[ -x "$binary" ]] || die "not an executable binary: ${binary}"

  # Human line is exactly: `facet <facet-version> (probe <probe-version>)`
  version_line="$("$binary" --version)" || die "${binary} --version failed"
  binary_version="$(printf '%s\n' "$version_line" | head -n1 | awk '{print $2}')"
  probe_version="$(printf '%s\n' "$version_line" | head -n1 | sed -n 's/.*(probe \([^)]*\)).*/\1/p')"

  [[ -n "$binary_version" ]] \
    || die "could not parse a version out of: ${version_line}"

  echo "facet --version:    ${version_line}"

  if [[ "$binary_version" != "$cargo_version" ]]; then
    echo "MISMATCH: binary says ${binary_version}, Cargo says ${cargo_version}" >&2
    failed=1
  fi

  # The probe parenthetical is reported, never enforced. Facet and probe are
  # allowed to diverge; both simply have to be true.
  if [[ -n "$probe_version" ]]; then
    echo "probe (reported):   ${probe_version}"
  fi
fi

if [[ "$failed" -ne 0 ]]; then
  echo "" >&2
  echo "version truth is broken. Reconcile the tag and [workspace.package].version" >&2
  echo "before releasing -- do not rename release assets to paper over it." >&2
  exit 1
fi

echo "version truth: OK"
