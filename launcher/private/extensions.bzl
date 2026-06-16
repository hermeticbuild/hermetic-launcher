"""Module extension for downloading non-module dependencies."""

load("@bazel_tools//tools/build_defs/repo:http.bzl", "http_file")

_download_attrs = {
    "finalize-stub-aarch64-linux": {
        "name": "finalize_stub_aarch64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260616/finalize-stub-aarch64-linux",
        "sha256": "042bf32e5b511a8b2a346f34e79876e0ee252fea03a6c73bd16d23e5785b298c",
    },
    "finalize-stub-aarch64-macos": {
        "name": "finalize_stub_aarch64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260616/finalize-stub-aarch64-macos",
        "sha256": "e4578d8b8056c857a64dc18144de7272243582f82da76346cd71dfb9ee0eb163",
    },
    "finalize-stub-aarch64-windows.exe": {
        "name": "finalize_stub_aarch64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260616/finalize-stub-aarch64-windows.exe",
        "sha256": "cbbc30fd15779bf9da53fcb247df2736eef330c37760d17308477165ebb783b9",
    },
    "finalize-stub-s390x-linux": {
        "name": "finalize_stub_s390x_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260616/finalize-stub-s390x-linux",
        "sha256": "631eea557ff24e8c6d2a6765df6f9478eef30530e4f41d53212f74133a0d0fdf",
    },
    "finalize-stub-x86_64-linux": {
        "name": "finalize_stub_x86_64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260616/finalize-stub-x86_64-linux",
        "sha256": "c7a09c660f998150305b32bdc619820ec1c7b4a8be153901cb1e32c3ba0e44e9",
    },
    "finalize-stub-x86_64-macos": {
        "name": "finalize_stub_x86_64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260616/finalize-stub-x86_64-macos",
        "sha256": "555b7aadaa2a0061ea37ccae8b6e4095a6310178f86d310672473d40eb7f4269",
    },
    "finalize-stub-x86_64-windows.exe": {
        "name": "finalize_stub_x86_64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260616/finalize-stub-x86_64-windows.exe",
        "sha256": "73602239c8183e9e24a8e057c44c0b4ca57a860e21a5ae4b00dd0b33a62ceebd",
    },
    "runfiles-stub-aarch64-linux": {
        "name": "runfiles_stub_aarch64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260616/runfiles-stub-aarch64-linux",
        "sha256": "f4f312007033f9c92e1c914acd5195d88ad91526159469cf7fdb97e5ec0eb48c",
    },
    "runfiles-stub-aarch64-macos": {
        "name": "runfiles_stub_aarch64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260616/runfiles-stub-aarch64-macos",
        "sha256": "bb05bb35a7042a0046e546c7ec109619d0ffe35dd45a8ebaa4f52aa192cad8eb",
    },
    "runfiles-stub-aarch64-windows.exe": {
        "name": "runfiles_stub_aarch64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260616/runfiles-stub-aarch64-windows.exe",
        "sha256": "30db110e5507e0b789010b43136a5a7f95de2a37b47f0e37932cc2dc598d385f",
    },
    "runfiles-stub-s390x-linux": {
        "name": "runfiles_stub_s390x_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260616/runfiles-stub-s390x-linux",
        "sha256": "e6e589a4ea33f8b89b9b67dcca342190ed46fc415e7ef7c90a59f62fb5c07621",
    },
    "runfiles-stub-x86_64-linux": {
        "name": "runfiles_stub_x86_64_linux",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260616/runfiles-stub-x86_64-linux",
        "sha256": "c6dd51abe0b8b5d848475fc3613e17be0584163f172fa41d3aaaff7b25a47bb5",
    },
    "runfiles-stub-x86_64-macos": {
        "name": "runfiles_stub_x86_64_macos",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260616/runfiles-stub-x86_64-macos",
        "sha256": "d42e86b3defbc030e5f8762e5921972bf5514f12c92f3dc6ed47470723fab243",
    },
    "runfiles-stub-x86_64-windows.exe": {
        "name": "runfiles_stub_x86_64_windows",
        "url": "https://github.com/hermeticbuild/hermetic-launcher/releases/download/binaries-20260616/runfiles-stub-x86_64-windows.exe",
        "sha256": "fb74105cc0384dce9631625909be83fa3fafd0b28eba05a971458c452e2b2d46",
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
