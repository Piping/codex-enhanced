#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo_home="${CARGO_HOME:-${HOME}/.cargo}"
host_triple="$(rustc -vV | sed -n 's/^host: //p')"

if [[ -z "${host_triple}" ]]; then
  echo "failed to determine rust host triple" >&2
  exit 1
fi

v8_version="$(awk '
  $1 == "name" && $3 == "\"v8\"" { in_v8 = 1; next }
  in_v8 && $1 == "version" {
    gsub(/"/, "", $3)
    print $3
    exit
  }
' "${repo_root}/Cargo.lock")"

if [[ -z "${v8_version}" ]]; then
  echo "failed to determine v8 version from Cargo.lock" >&2
  exit 1
fi

if [[ "${host_triple}" == *windows* ]]; then
  archive_filename="rusty_v8_release_${host_triple}.lib"
  archive_basename="rusty_v8"
else
  archive_filename="librusty_v8_release_${host_triple}.a"
  archive_basename="librusty_v8"
fi

obj_dir="${repo_root}/target/debug/gn_out/obj"
archive_path="${obj_dir}/${archive_basename}.${archive_filename##*.}"
sum_path="${obj_dir}/${archive_basename}.sum"
download_tmp="${obj_dir}/${archive_basename}.tmp"
url="https://github.com/denoland/rusty_v8/releases/download/v${v8_version}/${archive_filename}.gz"
cache_key="$(printf '%s' "${url}" | sed 's/[^[:alnum:]]/_/g')"
cache_path="${cargo_home}/.rusty_v8/${cache_key}"

if [[ -s "${archive_path}" ]]; then
  exit 0
fi

mkdir -p "${obj_dir}" "${cargo_home}/.rusty_v8"

if [[ ! -f "${cache_path}" ]]; then
  curl -L -f -s -o "${download_tmp}" "${url}"
  cp "${download_tmp}" "${cache_path}"
  rm -f "${download_tmp}"
fi

gzip -dc "${cache_path}" > "${archive_path}"
printf '%s' "${url}" > "${sum_path}"
