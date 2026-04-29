#!/usr/bin/env bash
set -euo pipefail

out_dir="${1:-artifacts}"
bazel_cmd="${BAZEL:-bazel}"

targets=(
  //runfiles-stub:runfiles-stub_aarch64-unknown-linux-musl
  //runfiles-stub:runfiles-stub_s390x-unknown-linux-gnu
  //runfiles-stub:runfiles-stub_x86_64-unknown-linux-musl
  //runfiles-stub:runfiles-stub_aarch64-apple-darwin
  //runfiles-stub:runfiles-stub_x86_64-apple-darwin
  //runfiles-stub:runfiles-stub_aarch64-pc-windows-gnullvm
  //runfiles-stub:runfiles-stub_x86_64-pc-windows-gnullvm
  //finalize-stub:finalize-stub_aarch64-unknown-linux-musl
  //finalize-stub:finalize-stub_s390x-unknown-linux-gnu
  //finalize-stub:finalize-stub_x86_64-unknown-linux-musl
  //finalize-stub:finalize-stub_aarch64-apple-darwin
  //finalize-stub:finalize-stub_x86_64-apple-darwin
  //finalize-stub:finalize-stub_aarch64-pc-windows-gnullvm
  //finalize-stub:finalize-stub_x86_64-pc-windows-gnullvm
)

"${bazel_cmd}" build --config=release "${targets[@]}"

rm -rf "${out_dir}"
mkdir -p "${out_dir}"

copy() {
  local src="$1"
  local dst="$2"

  cp "bazel-bin/${src}" "${out_dir}/${dst}"
  chmod +x "${out_dir}/${dst}"
}

copy "runfiles-stub/runfiles-stub_aarch64-unknown-linux-musl" "runfiles-stub-aarch64-linux"
copy "runfiles-stub/runfiles-stub_s390x-unknown-linux-gnu" "runfiles-stub-s390x-linux"
copy "runfiles-stub/runfiles-stub_x86_64-unknown-linux-musl" "runfiles-stub-x86_64-linux"
copy "runfiles-stub/runfiles-stub_aarch64-apple-darwin" "runfiles-stub-aarch64-macos"
copy "runfiles-stub/runfiles-stub_x86_64-apple-darwin" "runfiles-stub-x86_64-macos"
copy "runfiles-stub/runfiles-stub_aarch64-pc-windows-gnullvm" "runfiles-stub-aarch64-windows.exe"
copy "runfiles-stub/runfiles-stub_x86_64-pc-windows-gnullvm" "runfiles-stub-x86_64-windows.exe"
copy "finalize-stub/finalize-stub_aarch64-unknown-linux-musl" "finalize-stub-aarch64-linux"
copy "finalize-stub/finalize-stub_s390x-unknown-linux-gnu" "finalize-stub-s390x-linux"
copy "finalize-stub/finalize-stub_x86_64-unknown-linux-musl" "finalize-stub-x86_64-linux"
copy "finalize-stub/finalize-stub_aarch64-apple-darwin" "finalize-stub-aarch64-macos"
copy "finalize-stub/finalize-stub_x86_64-apple-darwin" "finalize-stub-x86_64-macos"
copy "finalize-stub/finalize-stub_aarch64-pc-windows-gnullvm" "finalize-stub-aarch64-windows.exe"
copy "finalize-stub/finalize-stub_x86_64-pc-windows-gnullvm" "finalize-stub-x86_64-windows.exe"

(
  cd "${out_dir}"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum * > SHA256SUMS.txt
  else
    shasum -a 256 * > SHA256SUMS.txt
  fi
)

ls -lh "${out_dir}"
