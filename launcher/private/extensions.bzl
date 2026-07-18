"""Module extension for downloading non-module dependencies."""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_file")

_download_attrs = {
    "finalize-stub-aarch64-linux": {
        "name": "finalize_stub_aarch64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/finalize-stub-aarch64-linux",
        "sha256": "042bf32e5b511a8b2a346f34e79876e0ee252fea03a6c73bd16d23e5785b298c",
    },
    "finalize-stub-aarch64-macos": {
        "name": "finalize_stub_aarch64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/finalize-stub-aarch64-macos",
        "sha256": "e4578d8b8056c857a64dc18144de7272243582f82da76346cd71dfb9ee0eb163",
    },
    "finalize-stub-aarch64-windows.exe": {
        "name": "finalize_stub_aarch64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/finalize-stub-aarch64-windows.exe",
        "sha256": "9c6e9a83d3b039255f23ba60864efab5c9569e2f91c618a09a5eea518317f906",
    },
    "finalize-stub-s390x-linux": {
        "name": "finalize_stub_s390x_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/finalize-stub-s390x-linux",
        "sha256": "631eea557ff24e8c6d2a6765df6f9478eef30530e4f41d53212f74133a0d0fdf",
    },
    "finalize-stub-x86_64-linux": {
        "name": "finalize_stub_x86_64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/finalize-stub-x86_64-linux",
        "sha256": "c7a09c660f998150305b32bdc619820ec1c7b4a8be153901cb1e32c3ba0e44e9",
    },
    "finalize-stub-x86_64-macos": {
        "name": "finalize_stub_x86_64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/finalize-stub-x86_64-macos",
        "sha256": "555b7aadaa2a0061ea37ccae8b6e4095a6310178f86d310672473d40eb7f4269",
    },
    "finalize-stub-x86_64-windows.exe": {
        "name": "finalize_stub_x86_64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/finalize-stub-x86_64-windows.exe",
        "sha256": "e9c6610f3a08e0a4989a7a2dc5923403d376b5732425e42a6e394238089a8389",
    },
    "runfiles-stub-aarch64-linux": {
        "name": "runfiles_stub_aarch64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/runfiles-stub-aarch64-linux",
        "sha256": "1bc3c4308410685958e4b1b7a10f0a61e5a817b0298a5493073ebafd62fc3e0f",
    },
    "runfiles-stub-aarch64-macos": {
        "name": "runfiles_stub_aarch64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/runfiles-stub-aarch64-macos",
        "sha256": "d998a9751cdf5d114a47f2bb6ccc2d24405c6e9550488ce0cdbff1bb7e3ab4e9",
    },
    "runfiles-stub-aarch64-windows.exe": {
        "name": "runfiles_stub_aarch64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/runfiles-stub-aarch64-windows.exe",
        "sha256": "25627dd0c43a5fe500909ad5d2c517359c298cde6c0aaa8222e44bb93577f145",
    },
    "runfiles-stub-s390x-linux": {
        "name": "runfiles_stub_s390x_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/runfiles-stub-s390x-linux",
        "sha256": "5bed6fb43446b67d66313e1f48a3c54da676c6d1bdbdc0fb74478d98b93a2fa5",
    },
    "runfiles-stub-x86_64-linux": {
        "name": "runfiles_stub_x86_64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/runfiles-stub-x86_64-linux",
        "sha256": "1cfdfe80a0d06eecf8b063f4604102de001f6d75d9a81eeb294964aab6c6a888",
    },
    "runfiles-stub-x86_64-macos": {
        "name": "runfiles_stub_x86_64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/runfiles-stub-x86_64-macos",
        "sha256": "4af8603ac91c2183133fa3c81fd98bb56310fbbb35408346a09f65586f2abd0d",
    },
    "runfiles-stub-x86_64-windows.exe": {
        "name": "runfiles_stub_x86_64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/runfiles-stub-x86_64-windows.exe",
        "sha256": "978090fc2914e2aa3b4c83ca5f3899d2651ff367e43e1d21688923ea5eb1594e",
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
