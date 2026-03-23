"""Module extension for downloading non-module dependencies."""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_file")

_download_attrs = {
    "finalize-stub-aarch64-linux": {
        "name": "finalize_stub_aarch64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260129/finalize-stub-aarch64-linux",
        "sha256": "d10352da70edf2d604b86f2f8d8ac03873678732d70948315f167491eb16efe7",
    },
    "finalize-stub-aarch64-macos": {
        "name": "finalize_stub_aarch64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260129/finalize-stub-aarch64-macos",
        "sha256": "3b37ffea00198e3f72bb0f0c0fb4d6cc0b4876b8154f082286783aa2a4fcd4ad",
    },
    "finalize-stub-x86_64-linux": {
        "name": "finalize_stub_x86_64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260129/finalize-stub-x86_64-linux",
        "sha256": "797972f9d7fccf28d008041318777974c0d8ba1cb3f5b780c2acc6b1ea695e86",
    },
    "finalize-stub-x86_64-macos": {
        "name": "finalize_stub_x86_64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260129/finalize-stub-x86_64-macos",
        "sha256": "4147b1eb75efa5433b7ff75a49ad26ac5cb5ad148490783e78a9b8ee7c01b759",
    },
    "finalize-stub-x86_64-windows.exe": {
        "name": "finalize_stub_x86_64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260129/finalize-stub-x86_64-windows.exe",
        "sha256": "205af4c7a50f6316c15d9008b273621affe4c62ec3ad7d054bcfa2c15070ec18",
    },
    "runfiles-stub-aarch64-linux": {
        "name": "runfiles_stub_aarch64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260129/runfiles-stub-aarch64-linux",
        "sha256": "1f255c28b084b4659e9054a4cb93bb10df9313c07d184ce14e4fa89c48c0acdb",
    },
    "runfiles-stub-aarch64-macos": {
        "name": "runfiles_stub_aarch64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260129/runfiles-stub-aarch64-macos",
        "sha256": "0a77f05b60da6d57a1970b181e79c614a883a95b8378c96278bd64d19d4cb923",
    },
    "runfiles-stub-x86_64-linux": {
        "name": "runfiles_stub_x86_64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260129/runfiles-stub-x86_64-linux",
        "sha256": "39b67aaf2de42a25ea0e05ec713b1a0cf689d9a81dd64bf4d2aa7d74e6c8ec78",
    },
    "runfiles-stub-x86_64-macos": {
        "name": "runfiles_stub_x86_64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260129/runfiles-stub-x86_64-macos",
        "sha256": "58d708bf32181f05638857cbf8898f0c5c511e1b69ffeac433fcac7b4ca3aa8a",
    },
    "runfiles-stub-x86_64-windows.exe": {
        "name": "runfiles_stub_x86_64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260129/runfiles-stub-x86_64-windows.exe",
        "sha256": "833a856a7012fa1a6ee1c525c83cf67ef6f2652d5ff3e5e38d99603e7652c6a6",
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
