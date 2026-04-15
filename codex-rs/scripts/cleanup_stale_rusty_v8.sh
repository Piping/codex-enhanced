#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_root="${repo_root}/target"
found_stale=0

if [[ ! -d "${target_root}" ]]; then
  exit 0
fi

find "${target_root}" -type d -path '*/gn_out/obj' | while read -r obj_dir; do
  stale=0

  for archive_name in librusty_v8.a rusty_v8.lib; do
    archive_path="${obj_dir}/${archive_name}"
    sum_path="${obj_dir}/${archive_name%.*}.sum"
    tmp_path="${obj_dir}/${archive_name%.*}.tmp"

    if [[ ! -e "${sum_path}" && ! -e "${tmp_path}" ]]; then
      continue
    fi

    if [[ ! -s "${archive_path}" || -e "${tmp_path}" ]]; then
      stale=1
      found_stale=1
      rm -f "${archive_path}" "${sum_path}" "${tmp_path}"
    fi
  done

  if [[ "${stale}" -eq 1 ]]; then
    echo "cleaned stale rusty_v8 artifacts in ${obj_dir}"
  fi
done

if [[ "${found_stale}" -eq 1 ]]; then
  (
    cd "${repo_root}"
    cargo clean -p v8
  )

  find "${target_root}" -type d -path '*/build/v8-*' -prune -exec rm -rf {} +
  find "${target_root}" -type d -path '*/gn_out' -prune -exec rm -rf {} +
fi
