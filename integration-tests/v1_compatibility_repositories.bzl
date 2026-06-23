"""Immutable V1 release artifacts used only by integration tests."""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_file")

_ARTIFACTS = {
    "finalize-stub-aarch64-linux": {
        "name": "v1_compat_20260619_finalize_stub_aarch64_linux",
        "sha256": "042bf32e5b511a8b2a346f34e79876e0ee252fea03a6c73bd16d23e5785b298c",
    },
    "finalize-stub-aarch64-macos": {
        "name": "v1_compat_20260619_finalize_stub_aarch64_macos",
        "sha256": "e4578d8b8056c857a64dc18144de7272243582f82da76346cd71dfb9ee0eb163",
    },
    "finalize-stub-x86_64-linux": {
        "name": "v1_compat_20260619_finalize_stub_x86_64_linux",
        "sha256": "c7a09c660f998150305b32bdc619820ec1c7b4a8be153901cb1e32c3ba0e44e9",
    },
    "finalize-stub-x86_64-macos": {
        "name": "v1_compat_20260619_finalize_stub_x86_64_macos",
        "sha256": "555b7aadaa2a0061ea37ccae8b6e4095a6310178f86d310672473d40eb7f4269",
    },
    "finalize-stub-x86_64-windows.exe": {
        "name": "v1_compat_20260619_finalize_stub_x86_64_windows",
        "sha256": "e9c6610f3a08e0a4989a7a2dc5923403d376b5732425e42a6e394238089a8389",
    },
    "runfiles-stub-aarch64-linux": {
        "name": "v1_compat_20260619_runfiles_stub_aarch64_linux",
        "sha256": "1bc3c4308410685958e4b1b7a10f0a61e5a817b0298a5493073ebafd62fc3e0f",
    },
    "runfiles-stub-aarch64-macos": {
        "name": "v1_compat_20260619_runfiles_stub_aarch64_macos",
        "sha256": "d998a9751cdf5d114a47f2bb6ccc2d24405c6e9550488ce0cdbff1bb7e3ab4e9",
    },
    "runfiles-stub-x86_64-linux": {
        "name": "v1_compat_20260619_runfiles_stub_x86_64_linux",
        "sha256": "1cfdfe80a0d06eecf8b063f4604102de001f6d75d9a81eeb294964aab6c6a888",
    },
    "runfiles-stub-x86_64-macos": {
        "name": "v1_compat_20260619_runfiles_stub_x86_64_macos",
        "sha256": "4af8603ac91c2183133fa3c81fd98bb56310fbbb35408346a09f65586f2abd0d",
    },
    "runfiles-stub-x86_64-windows.exe": {
        "name": "v1_compat_20260619_runfiles_stub_x86_64_windows",
        "sha256": "978090fc2914e2aa3b4c83ca5f3899d2651ff367e43e1d21688923ea5eb1594e",
    },
}

def _v1_compatibility_repositories_impl(ctx):
    for filename, attrs in _ARTIFACTS.items():
        http_file(
            name = attrs["name"],
            downloaded_file_path = filename,
            executable = True,
            sha256 = attrs["sha256"],
            url = "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260619/{}".format(filename),
        )
    return ctx.extension_metadata(
        reproducible = True,
        root_module_direct_deps = [],
        root_module_direct_dev_deps = "all",
    )

v1_compatibility_repositories = module_extension(
    implementation = _v1_compatibility_repositories_impl,
)
