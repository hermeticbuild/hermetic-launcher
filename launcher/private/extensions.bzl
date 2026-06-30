"""Module extension for downloading non-module dependencies."""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_file")

_download_attrs = {
    "finalize-stub-aarch64-linux": {
        "name": "finalize_stub_aarch64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260630/finalize-stub-aarch64-linux",
        "sha256": "042bf32e5b511a8b2a346f34e79876e0ee252fea03a6c73bd16d23e5785b298c",
    },
    "finalize-stub-aarch64-macos": {
        "name": "finalize_stub_aarch64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260630/finalize-stub-aarch64-macos",
        "sha256": "e4578d8b8056c857a64dc18144de7272243582f82da76346cd71dfb9ee0eb163",
    },
    "finalize-stub-aarch64-windows.exe": {
        "name": "finalize_stub_aarch64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260630/finalize-stub-aarch64-windows.exe",
        "sha256": "537e61614ac5be0f0f7471f347fa147341da54de65572ec98cbabf42d9dbce8b",
    },
    "finalize-stub-s390x-linux": {
        "name": "finalize_stub_s390x_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260630/finalize-stub-s390x-linux",
        "sha256": "631eea557ff24e8c6d2a6765df6f9478eef30530e4f41d53212f74133a0d0fdf",
    },
    "finalize-stub-x86_64-linux": {
        "name": "finalize_stub_x86_64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260630/finalize-stub-x86_64-linux",
        "sha256": "c7a09c660f998150305b32bdc619820ec1c7b4a8be153901cb1e32c3ba0e44e9",
    },
    "finalize-stub-x86_64-macos": {
        "name": "finalize_stub_x86_64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260630/finalize-stub-x86_64-macos",
        "sha256": "555b7aadaa2a0061ea37ccae8b6e4095a6310178f86d310672473d40eb7f4269",
    },
    "finalize-stub-x86_64-windows.exe": {
        "name": "finalize_stub_x86_64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260630/finalize-stub-x86_64-windows.exe",
        "sha256": "7de797f6163d0a1f55b9c41af4fc1074543a67a69975f6a3eca946611457ae17",
    },
    "runfiles-stub-aarch64-linux": {
        "name": "runfiles_stub_aarch64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260630/runfiles-stub-aarch64-linux",
        "sha256": "2bd0d8395319105253095d38f687e129bb656a9a2cf85c8d25b099bb1366ac9c",
    },
    "runfiles-stub-aarch64-macos": {
        "name": "runfiles_stub_aarch64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260630/runfiles-stub-aarch64-macos",
        "sha256": "edfe1fe39d14caa668569191b9abd6fd915da380f828ccdc1089f265f815db0f",
    },
    "runfiles-stub-aarch64-windows.exe": {
        "name": "runfiles_stub_aarch64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260630/runfiles-stub-aarch64-windows.exe",
        "sha256": "00a4f196d33c437867209ffdbe986a75cb1bbdcbc78d9839f43cd7fd2bc1e8c5",
    },
    "runfiles-stub-s390x-linux": {
        "name": "runfiles_stub_s390x_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260630/runfiles-stub-s390x-linux",
        "sha256": "f92295b052bfcccd50b1ecb8113a76eb04d2b7b4a424309714e5de4cbee073d0",
    },
    "runfiles-stub-x86_64-linux": {
        "name": "runfiles_stub_x86_64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260630/runfiles-stub-x86_64-linux",
        "sha256": "daf886b3b3a5e6f8065073e40c4e97529b5fe520a83194bb23479fb5cb18b828",
    },
    "runfiles-stub-x86_64-macos": {
        "name": "runfiles_stub_x86_64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260630/runfiles-stub-x86_64-macos",
        "sha256": "e9313c752723426419a56c59e14548bcb0ba16fa2edfb7d0e9172dc769f0247c",
    },
    "runfiles-stub-x86_64-windows.exe": {
        "name": "runfiles_stub_x86_64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260630/runfiles-stub-x86_64-windows.exe",
        "sha256": "d7dbe8ba7c697ced84ad574c162e4dee102ed9f4282801b53ed90055a7806155",
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
