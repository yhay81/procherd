#!/bin/sh
set -eu

expected_signing_fingerprint="0C153FFE2B0274365ACB1BF1AEFA86FA828C52C5"

fail() {
  printf 'continuity drill failed: %s\n' "$*" >&2
  exit 1
}

if [ "$#" -ne 2 ]; then
  fail "usage: $0 OWNER/REPOSITORY BINARY"
fi

repository=$1
binary=$2

case "$repository" in
  yhay81/[A-Za-z0-9_.-]*) ;;
  *) fail "repository must be under yhay81" ;;
esac
case "$binary" in
  "" | *[!A-Za-z0-9_-]*) fail "binary must be a portable file name" ;;
esac

for command_name in curl gh git gpg gpgconf sha256sum tar; do
  command -v "$command_name" >/dev/null 2>&1 ||
    fail "required command is unavailable: $command_name"
done

case "$(uname -s):$(uname -m)" in
  Linux:x86_64) native_target="linux-x86_64" ;;
  Darwin:arm64) native_target="macos-aarch64" ;;
  Darwin:x86_64) native_target="macos-x86_64" ;;
  *) fail "no native release archive is declared for $(uname -s) $(uname -m)" ;;
esac

drill_root=$(mktemp -d "${TMPDIR:-/tmp}/oss-drill.XXXXXX")
case "$drill_root" in
  "${TMPDIR:-/tmp}/"*) ;;
  *) fail "mktemp returned an unexpected path" ;;
esac
mirror="$drill_root/repository.git"
release_dir="$drill_root/release"
extract_dir="$drill_root/extracted"
gnupg_dir="$drill_root/gnupg"
key_file="$drill_root/yhay81-public-key.gpg"

cleanup() {
  gpgconf --homedir "$gnupg_dir" --kill all >/dev/null 2>&1 || true
  rm -rf "$drill_root"
}
trap cleanup EXIT HUP INT TERM

git clone --quiet --mirror "https://github.com/${repository}.git" "$mirror"
git --git-dir="$mirror" fsck --full

curl --fail --silent --show-error --location \
  "https://github.com/yhay81.gpg" \
  --output "$key_file"
mkdir -m 700 "$gnupg_dir"
GNUPGHOME="$gnupg_dir" gpg --batch --quiet --import "$key_file"
GNUPGHOME="$gnupg_dir" gpg --batch --with-colons --fingerprint |
  awk -F: '$1 == "fpr" { print $10 }' |
  grep -Fx "$expected_signing_fingerprint" >/dev/null ||
  fail "the published key does not contain the pinned signing fingerprint"

release_tags=$(git --git-dir="$mirror" tag --list "v*" --sort=version:refname)
[ -n "$release_tags" ] || fail "no release tags were found"
for tag in $release_tags; do
  GNUPGHOME="$gnupg_dir" git --git-dir="$mirror" verify-tag "$tag"
done

release_tag=$(gh release view --repo "$repository" --json tagName --jq ".tagName")
case "$release_tag" in
  v[0-9]*.[0-9]*.[0-9]*) ;;
  *) fail "latest release tag is not semantic: $release_tag" ;;
esac

mkdir "$release_dir" "$extract_dir"
gh release download "$release_tag" --repo "$repository" --dir "$release_dir"
(
  cd "$release_dir"
  sha256sum --check SHA256SUMS
)

archive="$release_dir/${binary}-${release_tag}-${native_target}.tar.gz"
[ -f "$archive" ] || fail "native recovery archive is missing: $archive"
gh attestation verify "$archive" \
  --repo "$repository" \
  --signer-workflow "${repository}/.github/workflows/release.yml" \
  --source-ref "refs/tags/${release_tag}"
gh attestation verify "$archive" \
  --repo "$repository" \
  --predicate-type "https://cyclonedx.org/bom" \
  --signer-workflow "${repository}/.github/workflows/release.yml" \
  --source-ref "refs/tags/${release_tag}"

tar -xzf "$archive" -C "$extract_dir"
binary_path=$(find "$extract_dir" -type f -name "$binary" -perm -u+x -print -quit)
[ -n "$binary_path" ] || fail "released binary was not found after extraction"
"$binary_path" --version

printf '{"repository":"%s","release":"%s","native_target":"%s","signing_fingerprint":"%s","status":"passed"}\n' \
  "$repository" "$release_tag" "$native_target" "$expected_signing_fingerprint"
