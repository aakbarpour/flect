#!/usr/bin/env sh

set -eu

repository='aakbarpour/flect'
version='latest'
bin_dir=''
archive=''
checksum_file=''

usage() {
    cat <<'EOF'
Usage: sh install.sh [OPTIONS]

Install the Flect executable for the current supported platform.

Options:
  --version VERSION       Release version (latest or vX.Y.Z; default: latest)
  --bin-dir PATH          User-local installation directory
  --archive FILE          Use a local release archive (requires --checksum-file)
  --checksum-file FILE    Use a local SHA256SUMS file (requires --archive)
  -h, --help              Show this help
EOF
}

fail() {
    printf 'Flect installer: %s\n' "$1" >&2
    exit 1
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail '--version requires a value'
            version=$2
            shift 2
            ;;
        --bin-dir)
            [ "$#" -ge 2 ] || fail '--bin-dir requires a value'
            bin_dir=$2
            shift 2
            ;;
        --archive)
            [ "$#" -ge 2 ] || fail '--archive requires a value'
            archive=$2
            shift 2
            ;;
        --checksum-file)
            [ "$#" -ge 2 ] || fail '--checksum-file requires a value'
            checksum_file=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            fail "unknown option: $1"
            ;;
    esac
done

case "$version" in
    latest) ;;
    *) printf '%s\n' "$version" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' || fail '--version must be latest or vX.Y.Z' ;;
esac

if [ -n "$archive" ] || [ -n "$checksum_file" ]; then
    [ -n "$archive" ] && [ -n "$checksum_file" ] || fail '--archive and --checksum-file must be supplied together'
fi

if [ -z "$bin_dir" ]; then
    [ -n "${HOME:-}" ] || fail 'HOME is not set; pass --bin-dir explicitly'
    bin_dir="$HOME/.local/bin"
fi

os=$(uname -s 2>/dev/null || printf unknown)
machine=$(uname -m 2>/dev/null || printf unknown)
case "$os:$machine" in
    Linux:x86_64|Linux:amd64)
        target='x86_64-unknown-linux-gnu'
        ;;
    Linux:aarch64|Linux:arm64)
        target='aarch64-unknown-linux-gnu'
        ;;
    Darwin:x86_64|Darwin:amd64)
        target='x86_64-apple-darwin'
        ;;
    Darwin:arm64|Darwin:aarch64)
        target='aarch64-apple-darwin'
        ;;
    *)
        fail "unsupported platform or architecture: $os/$machine"
        ;;
esac

download_https() {
    url=$1
    destination=$2
    case "$url" in
        https://*) ;;
        *) fail "refusing non-HTTPS download URL: $url" ;;
    esac
    if command -v curl >/dev/null 2>&1; then
        curl --fail --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
            --silent --show-error --output "$destination" "$url" || fail "download failed: $url"
    elif command -v wget >/dev/null 2>&1; then
        wget --quiet --output-document "$destination" "$url" || fail "download failed: $url"
    else
        fail 'curl or wget is required for remote installation'
    fi
}

sha256_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        fail 'sha256sum or shasum is required to verify the release'
    fi
}

checksum_for() {
    awk -v expected_file="$1" '
        length($1) == 64 && $1 ~ /^[0-9A-Fa-f]+$/ {
            candidate = $2
            sub(/^\*/, "", candidate)
            if (candidate == expected_file) {
                print tolower($1)
                found = 1
                exit
            }
        }
        END { if (!found) exit 1 }
    ' "$2" || fail "checksum entry for $1 was not found"
}

work_dir=$(mktemp -d "${TMPDIR:-/tmp}/flect-install.XXXXXX") || fail 'could not create a temporary directory'
trap 'rm -rf "$work_dir"' EXIT HUP INT TERM

if [ -n "$archive" ]; then
    [ -f "$archive" ] || fail "archive does not exist: $archive"
    [ ! -L "$archive" ] || fail "archive must not be a symlink: $archive"
    [ -f "$checksum_file" ] || fail "checksum file does not exist: $checksum_file"
    [ ! -L "$checksum_file" ] || fail "checksum file must not be a symlink: $checksum_file"
    archive_path=$archive
    checksum_path=$checksum_file
    archive_name=$(basename "$archive_path")
    case "$archive_name" in
        flect-*-$target.tar.gz) ;;
        *) fail "archive does not match target $target: $archive_name" ;;
    esac
    if [ "$version" != latest ]; then
        [ "$archive_name" = "flect-${version}-${target}.tar.gz" ] || fail "archive does not match requested version $version"
    fi
else
    archive_name="flect-${version}-${target}.tar.gz"
    if [ "$version" = latest ]; then
        release_url="https://github.com/$repository/releases/latest/download"
    else
        release_url="https://github.com/$repository/releases/download/$version"
    fi
    archive_path="$work_dir/$archive_name"
    checksum_path="$work_dir/SHA256SUMS"
    download_https "$release_url/$archive_name" "$archive_path"
    download_https "$release_url/SHA256SUMS" "$checksum_path"
fi

expected=$(checksum_for "$archive_name" "$checksum_path")
actual=$(sha256_file "$archive_path" | tr 'A-F' 'a-f')
[ "$actual" = "$expected" ] || fail "SHA256 checksum mismatch for $archive_name"

archive_root=${archive_name%.tar.gz}
listing=$(tar -tzf "$archive_path") || fail "could not read archive: $archive_name"
while IFS= read -r entry; do
    case "$entry" in
        /*|../*|*/../*|..|*\\*) fail "unsafe archive entry: $entry" ;;
    esac
done <<EOF
$listing
EOF

extract_dir="$work_dir/extracted"
mkdir -p "$extract_dir"
tar -xzf "$archive_path" -C "$extract_dir" || fail "could not extract archive: $archive_name"
binary_path="$extract_dir/$archive_root/flect"
[ -f "$binary_path" ] || fail 'archive does not contain the expected flect executable'
[ ! -L "$binary_path" ] || fail 'archive executable must not be a symlink'
for required_file in LICENSE README.md; do
    required_path="$extract_dir/$archive_root/$required_file"
    [ -f "$required_path" ] || fail "archive does not contain $required_file"
    [ ! -L "$required_path" ] || fail "archive $required_file must not be a symlink"
done

mkdir -p "$bin_dir" || fail "could not create installation directory: $bin_dir"
[ ! -L "$bin_dir" ] || fail "installation directory must not be a symlink: $bin_dir"
bin_dir_abs=$(CDPATH= cd -- "$bin_dir" && pwd -P) || fail "could not resolve installation directory: $bin_dir"
temporary_binary="$work_dir/flect"
cp "$binary_path" "$temporary_binary" || fail 'could not stage the executable'
chmod 0755 "$temporary_binary" || fail 'could not make the executable runnable'
mv -f "$temporary_binary" "$bin_dir_abs/flect" || fail "could not install Flect into $bin_dir_abs"

printf 'Installed Flect %s for %s to %s/flect\n' "$archive_root" "$target" "$bin_dir_abs"
case ":${PATH:-}:" in
    *":$bin_dir_abs:"*) ;;
    *)
        printf '%s\n' "$bin_dir_abs is not currently on PATH. Add this line to your shell profile:"
        printf '  export PATH="%s:$PATH"\n' "$bin_dir_abs"
        ;;
esac
