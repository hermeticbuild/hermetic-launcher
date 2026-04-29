load("@rules_platform//platform_data:defs.bzl", "platform_data")
load("@rules_rust//rust:defs.bzl", "rust_binary")

TRIPLES = [
    "aarch64-unknown-linux-musl",
    "aarch64-apple-darwin",
    "aarch64-pc-windows-gnullvm",
    "s390x-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-gnullvm",
]

def rust_release_binary(name, triples = TRIPLES, visibility = None, **kwargs):
    rust_binary(
        name = name,
        visibility = visibility,
        **kwargs
    )

    for triple in triples:
        platform_data(
            name = name + "_" + triple,
            platform = "@rules_rs//rs/platforms:" + triple,
            target = name,
        )
