#!/bin/sh
set -eu

# Fallback only. The installer resolves the newest published release at run
# time; this pin is what it falls back to when that lookup fails. release.yml
# refuses to publish a tag that does not match it, so the fallback cannot go
# stale behind a release.
pinned_version="0.3.0"
heiwa_home="${HEIWA_HOME:-$HOME/.heiwa}"
repo="Heiwa-Limited/heiwa-universe"

fail() {
  echo "heiwa install: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

case "$heiwa_home" in
  ""|"/"|".") fail "refusing unsafe HEIWA_HOME: $heiwa_home" ;;
  /*) ;;
  *) fail "HEIWA_HOME must be an absolute path: $heiwa_home" ;;
esac

need curl
need grep
need awk
need tar
need install
need mv
need mktemp
need cp
need find
need ln
need mkdir
need sed

# GitHub answers /releases/latest with a 302 to /releases/tag/vX.Y.Z, so the
# newest version is readable from one header without shipping a JSON parser.
# Deliberately no --location: the redirect target is the answer.
resolve_latest_version() {
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error --head \
    "https://github.com/${repo}/releases/latest" 2>/dev/null |
    awk 'tolower($1) == "location:" { print $2 }' |
    tr -d '\r' |
    sed -n 's|.*/releases/tag/v\([0-9][0-9.]*\)$|\1|p' |
    tail -n 1
}

version="${HEIWA_VERSION:-}"
version_source="HEIWA_VERSION"
if [ -z "$version" ]; then
  version="$(resolve_latest_version || true)"
  version_source="latest release"
fi
if [ -z "$version" ]; then
  version="$pinned_version"
  version_source="pinned fallback"
fi

printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' ||
  fail "HEIWA_VERSION must be a stable semantic version such as 0.1.0"

os="$(uname -s)"
arch="$(uname -m)"
case "$os:$arch" in
  Darwin:arm64|Darwin:aarch64)
    asset="macos-aarch64"
    ;;
  Linux:x86_64|Linux:amd64)
    asset="linux-x86_64"
    ;;
  *)
    fail "unsupported platform $os/$arch; use the GitHub Release assets directly"
    ;;
esac

archive_name="heiwa-${version}-${asset}.tar.gz"
checksums_name="heiwa-${version}-checksums.txt"
release_base="https://github.com/${repo}/releases/download/v${version}"
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/heiwa-install.XXXXXX")"
staged_path=""
staged_cockpit=""
staged_link=""

cleanup() {
  rm -rf -- "$tmp_dir"
  if [ -n "$staged_path" ]; then
    rm -f -- "$staged_path"
  fi
  if [ -n "$staged_cockpit" ]; then
    rm -rf -- "$staged_cockpit"
  fi
  if [ -n "$staged_link" ]; then
    rm -f -- "$staged_link"
  fi
}
trap cleanup EXIT HUP INT TERM

download() {
  # Retry transient failures. Release assets come from a CDN over whatever
  # network the user happens to have, and without --retry a single 5xx or
  # timeout aborts the whole install. The checksum verification below already
  # makes a truncated body a hard failure, so a retry cannot paper over a bad
  # download. Deliberately only --retry/--retry-delay: --retry-all-errors and
  # --retry-connrefused are newer than the curl on RHEL 8 (7.61) and Ubuntu
  # 20.04 (7.68), where an unknown option would fail the install outright.
  curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error     --retry 3 --retry-delay 2     --output "$2" "$1"
}

echo "heiwa install: downloading v$version for $asset (version from $version_source)"
download "$release_base/$archive_name" "$tmp_dir/$archive_name"
download "$release_base/$checksums_name" "$tmp_dir/$checksums_name"

expected="$(
  awk -v file="$archive_name" '$2 == file || $2 == "*" file { print $1 }'     "$tmp_dir/$checksums_name"
)"
case "$expected" in
  ""|*[!0-9a-fA-F]*) fail "release checksum entry is missing or malformed" ;;
esac
[ "${#expected}" -eq 64 ] || fail "release checksum must be SHA-256"

if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$tmp_dir/$archive_name" | awk '{ print $1 }')"
elif command -v shasum >/dev/null 2>&1; then
  actual="$(shasum -a 256 "$tmp_dir/$archive_name" | awk '{ print $1 }')"
else
  fail "missing SHA-256 tool: install sha256sum or shasum"
fi
[ "$actual" = "$expected" ] || fail "checksum mismatch for $archive_name"

archive_root="heiwa-${version}-${asset}"
if ! tar -tzf "$tmp_dir/$archive_name" | while IFS= read -r path; do
  case "$path" in
    "$archive_root"|"$archive_root/"|"$archive_root/"*) ;;
    *) exit 1 ;;
  esac
  case "/$path/" in
    *"/../"*|*"/./"*) exit 1 ;;
  esac
done; then
  fail "archive contains a path outside $archive_root"
fi
if ! tar -tvzf "$tmp_dir/$archive_name" | awk '
  { type = substr($1, 1, 1) }
  type != "-" && type != "d" { exit 1 }
'; then
  fail "archive contains links or unsupported entry types"
fi

tar -xzf "$tmp_dir/$archive_name" -C "$tmp_dir"
binary="$tmp_dir/$archive_root/heiwa"
[ -f "$binary" ] || fail "release archive does not contain the heiwa binary"
cockpit_source="$tmp_dir/$archive_root/cockpit"
[ -f "$cockpit_source/index.html" ] || fail "release archive does not contain cockpit/index.html"
find "$cockpit_source/assets" -type f -print | grep -q . ||
  fail "release archive does not contain cockpit assets"

bin_dir="$heiwa_home/bin"
app_dir="$heiwa_home/app"
mkdir -p "$bin_dir"
mkdir -p "$app_dir"
staged_path="$bin_dir/.heiwa.new.$$"
install -m 0755 "$binary" "$staged_path"

checksum_prefix="$(printf '%.12s' "$actual")"
cockpit_name="cockpit-$version-$checksum_prefix"
cockpit_target="$app_dir/$cockpit_name"
if [ ! -d "$cockpit_target" ]; then
  staged_cockpit="$app_dir/.cockpit.new.$$"
  cp -R "$cockpit_source" "$staged_cockpit"
  mv "$staged_cockpit" "$cockpit_target"
  staged_cockpit=""
fi
[ -f "$cockpit_target/index.html" ] || fail "installed cockpit target is incomplete"

# The desktop app is a separate, macOS-only release asset. Releases cut before
# it existed have no such entry, so its absence from the checksum manifest is
# a skip rather than a failure -- the CLI install must not regress to serve a
# GUI that older tags never published.
installed_app=""
app_archive_name="heiwa-${version}-${asset}-app.tar.gz"
app_expected="$(
  awk -v file="$app_archive_name" '$2 == file || $2 == "*" file { print $1 }' \
    "$tmp_dir/$checksums_name"
)"

if [ "$os" = "Darwin" ] && [ -n "$app_expected" ]; then
  case "$app_expected" in
    *[!0-9a-fA-F]*) fail "desktop app checksum entry is malformed" ;;
  esac
  [ "${#app_expected}" -eq 64 ] || fail "desktop app checksum must be SHA-256"

  download "$release_base/$app_archive_name" "$tmp_dir/$app_archive_name"

  if command -v sha256sum >/dev/null 2>&1; then
    app_actual="$(sha256sum "$tmp_dir/$app_archive_name" | awk '{ print $1 }')"
  else
    app_actual="$(shasum -a 256 "$tmp_dir/$app_archive_name" | awk '{ print $1 }')"
  fi
  [ "$app_actual" = "$app_expected" ] ||
    fail "checksum mismatch for $app_archive_name"

  # Same containment and entry-type rules the runtime archive gets. A bundle
  # is a directory tree the user will execute, so it earns no relaxation.
  if ! tar -tzf "$tmp_dir/$app_archive_name" | while IFS= read -r path; do
    case "$path" in
      "Heiwa.app"|"Heiwa.app/"|"Heiwa.app/"*) ;;
      *) exit 1 ;;
    esac
    case "/$path/" in
      *"/../"*|*"/./"*) exit 1 ;;
    esac
  done; then
    fail "desktop archive contains a path outside Heiwa.app"
  fi
  if ! tar -tvzf "$tmp_dir/$app_archive_name" | awk '
    { type = substr($1, 1, 1) }
    type != "-" && type != "d" { exit 1 }
  '; then
    fail "desktop archive contains links or unsupported entry types"
  fi

  tar -xzf "$tmp_dir/$app_archive_name" -C "$tmp_dir"
  [ -x "$tmp_dir/Heiwa.app/Contents/MacOS/Heiwa" ] ||
    fail "desktop archive does not contain an executable app bundle"

  # Deliberately not $app_dir/Heiwa.app: `heiwa install` runs later in this
  # script and writes its own launcher shim to that path. Two different things
  # cannot own one path, and the runtime's bootstrap is not this installer's
  # to redefine. /Applications is also where a user's app belongs -- it is the
  # only location that reaches Spotlight and Launchpad.
  if [ -w /Applications ]; then
    applications_dir="/Applications"
  else
    applications_dir="$HOME/Applications"
    mkdir -p "$applications_dir"
  fi

  staged_bundle="$applications_dir/.Heiwa.app.new.$$"
  rm -rf -- "$staged_bundle"
  if cp -R "$tmp_dir/Heiwa.app" "$staged_bundle" 2>/dev/null; then
    rm -rf -- "$applications_dir/Heiwa.app"
    mv "$staged_bundle" "$applications_dir/Heiwa.app"
    installed_app="$applications_dir/Heiwa.app"
  else
    rm -rf -- "$staged_bundle"
    echo "heiwa install: could not write $applications_dir; skipped the app" >&2
  fi
fi

cockpit_link="$app_dir/cockpit-current"
if [ -e "$cockpit_link" ] && [ ! -L "$cockpit_link" ]; then
  fail "$cockpit_link exists and is not a managed symlink"
fi
staged_link="$app_dir/.cockpit-current.$$"
ln -s "$cockpit_name" "$staged_link"
case "$os" in
  Darwin) mv -fh "$staged_link" "$cockpit_link" ;;
  Linux) mv -Tf "$staged_link" "$cockpit_link" ;;
esac
staged_link=""

mv -f "$staged_path" "$bin_dir/heiwa"
staged_path=""

echo "heiwa install: bootstrapping local runtime state"
"$bin_dir/heiwa" install

app_line=""
if [ -n "$installed_app" ]; then
  app_line="
  app: $installed_app"
fi

cat <<EOF
heiwa install: complete
  version: v$version
  binary: $bin_dir/heiwa
  cockpit: $cockpit_link$app_line
  archive: $archive_name
  sha256: $actual

Next:
  export PATH="$bin_dir:\$PATH"
  heiwa doctor
  heiwa app start --no-open
EOF
