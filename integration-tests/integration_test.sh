#!/usr/bin/env bash
set -euo pipefail

runfiles_root="${RUNFILES_DIR:-${TEST_SRCDIR:-}}"
if [[ -z "${runfiles_root}" ]]; then
  echo "RUNFILES_DIR/TEST_SRCDIR not set" >&2
  exit 1
fi

template_bin="${runfiles_root}/${TEMPLATE_BIN_RL}"
finalizer_bin="${runfiles_root}/${FINALIZER_BIN_RL}"
runner_bin="${runfiles_root}/${TEST_RUNNER_RL}"
test_bin_dir="${runfiles_root}/${TEST_BIN_DIR_RL}"

work_root="${TEST_TMPDIR:-$(mktemp -d)}"
work_dir="${work_root}/work"
mkdir -p "${work_dir}"

"${runner_bin}" \
  --template "${template_bin}" \
  --finalizer "${finalizer_bin}" \
  --test-binaries "${test_bin_dir}" \
  --work-dir "${work_dir}"
