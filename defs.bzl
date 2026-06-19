load("@rules_platform//platform_data:defs.bzl", "platform_data")
load("@rules_rust//rust:defs.bzl", "rust_binary")
load("//optimize-binary:optimize_binary.bzl", "optimize_binary")

TRIPLES = [
    "aarch64-unknown-linux-musl",
    "aarch64-apple-darwin",
    "aarch64-pc-windows-gnullvm",
    "s390x-unknown-linux-musl",
    "x86_64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-gnullvm",
]

# Map a target triple to the OS passed to the optimizer pipeline.
def _triple_os(triple):
    if "darwin" in triple:
        return "macos"
    if "windows" in triple:
        return "windows"
    return "linux"

def rust_release_binary(name, triples = TRIPLES, visibility = None, optimize = True, **kwargs):
    """A rust_binary plus per-triple, platform-transitioned, optimized outputs.

    For each triple, `name_triple` is the release artifact: the binary built for that
    target platform and run through the post-link optimization pipeline (see
    //optimize-binary). The optimizer only removes provably-dead data.

    Args:
        name: Base rust_binary name.
        triples: Target triples to produce release artifacts for.
        visibility: Visibility for the base target and the per-triple artifacts.
        optimize: Run the optimization pipeline. Disable for binaries that are NOT
            `panic = "abort"` (e.g. the finalizer), since stripping unwind tables
            would turn a panic into an abort.
        **kwargs: Forwarded to the base rust_binary.
    """
    rust_binary(
        name = name,
        visibility = visibility,
        **kwargs
    )

    for triple in triples:
        if optimize:
            # Build the stub for the target platform, then run the post-link
            # optimization pipeline over it. The optimizer runs on the exec
            # platform and targets `triple`'s OS. The final `name_triple` label is
            # the optimized binary; the platform_data step is an internal input.
            platform_data(
                name = name + "_" + triple + ".unoptimized",
                platform = "@rules_rs//rs/platforms:" + triple,
                target = name,
                visibility = ["//visibility:private"],
            )
            optimize_binary(
                name = name + "_" + triple,
                src = name + "_" + triple + ".unoptimized",
                target_os = _triple_os(triple),
                visibility = visibility,
            )
        else:
            platform_data(
                name = name + "_" + triple,
                platform = "@rules_rs//rs/platforms:" + triple,
                target = name,
                visibility = visibility,
            )
