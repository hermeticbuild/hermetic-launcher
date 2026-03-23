"""Module extension for downloading non-module dependencies."""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_file")

_download_attrs = {
    "finalize-stub-aarch64-linux": {
        "name": "finalize_stub_aarch64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260323/finalize-stub-aarch64-linux",
        "sha256": "26f825c68c7fe1d893fc1cbda9d5f8f1525045841fbc13ff353564015ef75b80",
    },
    "finalize-stub-aarch64-macos": {
        "name": "finalize_stub_aarch64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260323/finalize-stub-aarch64-macos",
        "sha256": "435d35ffaf0b096604102d8c098ae8a54aa60acabb3f02a704158355fee2b8ec",
    },
    "finalize-stub-x86_64-linux": {
        "name": "finalize_stub_x86_64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260323/finalize-stub-x86_64-linux",
        "sha256": "651aa2b709d8bd2b1ff974c189606a7585173d2e06080455a92007459d4c5964",
    },
    "finalize-stub-x86_64-macos": {
        "name": "finalize_stub_x86_64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260323/finalize-stub-x86_64-macos",
        "sha256": "c2123321c05f9a16448acbd859375cbe18d4ff4bda4d6b2bfe73bedff2a8afc8",
    },
    "finalize-stub-x86_64-windows.exe": {
        "name": "finalize_stub_x86_64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260323/finalize-stub-x86_64-windows.exe",
        "sha256": "19ae47ed7d993054c20b657e3938f8104926b775f46d8d979f97c58ddd287f2c",
    },
    "runfiles-stub-aarch64-linux": {
        "name": "runfiles_stub_aarch64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260323/runfiles-stub-aarch64-linux",
        "sha256": "cd07b371ef5cd26fbd80df7ac24d3c328bb6e8af0392567aa0570da4626d6c73",
    },
    "runfiles-stub-aarch64-macos": {
        "name": "runfiles_stub_aarch64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260323/runfiles-stub-aarch64-macos",
        "sha256": "752f8c0580a9624527b8878bc82abd785d2600de4ab7aa7b8452c3a901c58b1d",
    },
    "runfiles-stub-x86_64-linux": {
        "name": "runfiles_stub_x86_64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260323/runfiles-stub-x86_64-linux",
        "sha256": "d0dc56be9be9c20abe3f3dd01147f0bc686db99221114f3fa424a194f6bc8c70",
    },
    "runfiles-stub-x86_64-macos": {
        "name": "runfiles_stub_x86_64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260323/runfiles-stub-x86_64-macos",
        "sha256": "bf9edf841c4011cf60979bd3355104cc831b0398c35c0ccf814ba011551f731c",
    },
    "runfiles-stub-x86_64-windows.exe": {
        "name": "runfiles_stub_x86_64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260323/runfiles-stub-x86_64-windows.exe",
        "sha256": "74f6e72e0b04b83ed735721d883364ce1090003bf6d54ec2be1a5760b0136a7c",
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
