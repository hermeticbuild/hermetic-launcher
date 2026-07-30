"""Module extension for downloading non-module dependencies."""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_file")

_download_attrs = {
    "finalize-stub-aarch64-linux": {
        "name": "finalize_stub_aarch64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260730/finalize-stub-aarch64-linux",
        "sha256": "042bf32e5b511a8b2a346f34e79876e0ee252fea03a6c73bd16d23e5785b298c",
    },
    "finalize-stub-aarch64-macos": {
        "name": "finalize_stub_aarch64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260730/finalize-stub-aarch64-macos",
        "sha256": "e4578d8b8056c857a64dc18144de7272243582f82da76346cd71dfb9ee0eb163",
    },
    "finalize-stub-aarch64-windows.exe": {
        "name": "finalize_stub_aarch64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260730/finalize-stub-aarch64-windows.exe",
        "sha256": "c5fe2c4c929b6b86d427c3ed4074be863446a6661fcacea0c82823ff7a0dea2a",
    },
    "finalize-stub-s390x-linux": {
        "name": "finalize_stub_s390x_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260730/finalize-stub-s390x-linux",
        "sha256": "631eea557ff24e8c6d2a6765df6f9478eef30530e4f41d53212f74133a0d0fdf",
    },
    "finalize-stub-x86_64-linux": {
        "name": "finalize_stub_x86_64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260730/finalize-stub-x86_64-linux",
        "sha256": "c7a09c660f998150305b32bdc619820ec1c7b4a8be153901cb1e32c3ba0e44e9",
    },
    "finalize-stub-x86_64-macos": {
        "name": "finalize_stub_x86_64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260730/finalize-stub-x86_64-macos",
        "sha256": "555b7aadaa2a0061ea37ccae8b6e4095a6310178f86d310672473d40eb7f4269",
    },
    "finalize-stub-x86_64-windows.exe": {
        "name": "finalize_stub_x86_64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260730/finalize-stub-x86_64-windows.exe",
        "sha256": "65cca0a78135e6b75d0e02079e1292a1c6b511e3cc0d64754825cfe219a95a55",
    },
    "runfiles-stub-aarch64-linux": {
        "name": "runfiles_stub_aarch64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260730/runfiles-stub-aarch64-linux",
        "sha256": "dc22404c103daa00cd95c87a15d429e7c9a1e6c8ffd88e1e49384639bcb16bd0",
    },
    "runfiles-stub-aarch64-macos": {
        "name": "runfiles_stub_aarch64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260730/runfiles-stub-aarch64-macos",
        "sha256": "61b114fe88c54be843a7367e03519d3d6d5a877836a2047d0576bf7f16579ab7",
    },
    "runfiles-stub-aarch64-windows.exe": {
        "name": "runfiles_stub_aarch64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260730/runfiles-stub-aarch64-windows.exe",
        "sha256": "51d3da85031739cb6b60c7d20a0ef781904f2809ca3a267a92a7b5c34d142190",
    },
    "runfiles-stub-s390x-linux": {
        "name": "runfiles_stub_s390x_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260730/runfiles-stub-s390x-linux",
        "sha256": "ba7af6958c79290dcbfd78bda4cb391db88394b2ddcce80cef4f1066096e7349",
    },
    "runfiles-stub-x86_64-linux": {
        "name": "runfiles_stub_x86_64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260730/runfiles-stub-x86_64-linux",
        "sha256": "b1341a51350871f1bdf4573870694e353b3fe3cadaeb40d931844644dee2e907",
    },
    "runfiles-stub-x86_64-macos": {
        "name": "runfiles_stub_x86_64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260730/runfiles-stub-x86_64-macos",
        "sha256": "b6003008318587ab325b9d39355178d0531c3124d86417e92deda3f4b8912b32",
    },
    "runfiles-stub-x86_64-windows.exe": {
        "name": "runfiles_stub_x86_64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260730/runfiles-stub-x86_64-windows.exe",
        "sha256": "458e5db02750e312451c2e2b2a0a4ebdf63491e10085f3281acadf2379025c7c",
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
