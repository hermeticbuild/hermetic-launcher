"""Module extension for downloading non-module dependencies."""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_file")

_download_attrs = {
    "finalize-stub-aarch64-linux": {
        "name": "finalize_stub_aarch64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260718/finalize-stub-aarch64-linux",
        "sha256": "042bf32e5b511a8b2a346f34e79876e0ee252fea03a6c73bd16d23e5785b298c",
    },
    "finalize-stub-aarch64-macos": {
        "name": "finalize_stub_aarch64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260718/finalize-stub-aarch64-macos",
        "sha256": "e4578d8b8056c857a64dc18144de7272243582f82da76346cd71dfb9ee0eb163",
    },
    "finalize-stub-aarch64-windows.exe": {
        "name": "finalize_stub_aarch64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260718/finalize-stub-aarch64-windows.exe",
        "sha256": "2624078e60241de96fc9384470a8769aabd003ba43afd9ba4b88b5773a936003",
    },
    "finalize-stub-s390x-linux": {
        "name": "finalize_stub_s390x_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260718/finalize-stub-s390x-linux",
        "sha256": "631eea557ff24e8c6d2a6765df6f9478eef30530e4f41d53212f74133a0d0fdf",
    },
    "finalize-stub-x86_64-linux": {
        "name": "finalize_stub_x86_64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260718/finalize-stub-x86_64-linux",
        "sha256": "c7a09c660f998150305b32bdc619820ec1c7b4a8be153901cb1e32c3ba0e44e9",
    },
    "finalize-stub-x86_64-macos": {
        "name": "finalize_stub_x86_64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260718/finalize-stub-x86_64-macos",
        "sha256": "555b7aadaa2a0061ea37ccae8b6e4095a6310178f86d310672473d40eb7f4269",
    },
    "finalize-stub-x86_64-windows.exe": {
        "name": "finalize_stub_x86_64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260718/finalize-stub-x86_64-windows.exe",
        "sha256": "43afdfa224dc24a9b05caa4375f112030595f6e4f9c151b969810c48ff56ba5a",
    },
    "runfiles-stub-aarch64-linux": {
        "name": "runfiles_stub_aarch64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260718/runfiles-stub-aarch64-linux",
        "sha256": "399a094032e597972a13fcb052e42e4df2a2f88ad374093ac5344fc7ecff6822",
    },
    "runfiles-stub-aarch64-macos": {
        "name": "runfiles_stub_aarch64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260718/runfiles-stub-aarch64-macos",
        "sha256": "348759fd7710b7bf430cac5e085f2f3d4eef2d58d690b3639d8660e002393516",
    },
    "runfiles-stub-aarch64-windows.exe": {
        "name": "runfiles_stub_aarch64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260718/runfiles-stub-aarch64-windows.exe",
        "sha256": "8281f4a22eafe89649235127340e1307d1df31ae20dbdccf757bd401443c62a4",
    },
    "runfiles-stub-s390x-linux": {
        "name": "runfiles_stub_s390x_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260718/runfiles-stub-s390x-linux",
        "sha256": "ffd67db72c4c88890a183682691605d8f796e63cd6a42a892438d4d1a7d68be8",
    },
    "runfiles-stub-x86_64-linux": {
        "name": "runfiles_stub_x86_64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260718/runfiles-stub-x86_64-linux",
        "sha256": "f9acffffaa4943bb9725484de60cc909d05f754b6aeea8939536ddd1241c7bf4",
    },
    "runfiles-stub-x86_64-macos": {
        "name": "runfiles_stub_x86_64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260718/runfiles-stub-x86_64-macos",
        "sha256": "fad351f9a871d84b9c87ad9b79ac66daf8369ba23a0bd5da14942227ed8e3a1f",
    },
    "runfiles-stub-x86_64-windows.exe": {
        "name": "runfiles_stub_x86_64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260718/runfiles-stub-x86_64-windows.exe",
        "sha256": "6767e320b3f37e8a46913b86b375e194e1f06777a307129c82cb614d3f15c6b1",
    },
}

def _non_module_dependencies_impl(ctx):
    for filename, attrs in _download_attrs.items():
        http_file(
            name = attrs["name"],
            url = attrs["url"],
            sha256 = attrs["sha256"],
            downloaded_file_path = filename,
            executable = True,
        )
    return ctx.extension_metadata(
        root_module_direct_deps = "all",
        root_module_direct_dev_deps = [],
        reproducible = True,
    )


non_module_dependencies = module_extension(
    implementation = _non_module_dependencies_impl,
)
