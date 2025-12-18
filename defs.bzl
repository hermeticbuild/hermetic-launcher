load("@rules_platform//platform_data:defs.bzl", "platform_data")
load("@rules_rust//rust:defs.bzl", "rust_binary")

TRIPLES = [
    "aarch64-unknown-linux-musl",
    "aarch64-apple-darwin",
    "aarch64-pc-windows-gnullvm",
    "x86_64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "x86_64-pc-windows-gnullvm",
]

def _strip_binary_impl(ctx):
    # Preserve the input basename (including extensions like .exe) for the stripped copy.
    src_base = ctx.file.src.basename
    out_name = src_base
    if out_name.endswith("_unstripped.exe"):
        out_name = out_name[:-15] + ".exe"  # drop "_unstripped" (11) + ".exe" (4)
    elif out_name.endswith("_unstripped"):
        out_name = out_name[:-11]  # drop "_unstripped"

    output = ctx.actions.declare_file(out_name)

    args = ctx.actions.args()
    args.add("--strip-all")
    args.add(ctx.file.src)
    args.add(output)

    ctx.actions.run(
        inputs = [ctx.file.src],
        outputs = [output],
        executable = ctx.file._objcopy,
        arguments = [args],
        mnemonic = "StripBinary",
    )

    return [DefaultInfo(
        files = depset([output]),
        executable = output,
        runfiles = ctx.runfiles(files = [output]),
    )]

strip_binary = rule(
    implementation = _strip_binary_impl,
    executable = True,
    attrs = {
        "src": attr.label(
            allow_single_file = True,
            mandatory = True,
        ),
        "_objcopy": attr.label(
            default = "@llvm//tools:llvm-objcopy",
            cfg = "exec",
            allow_single_file = True,
        ),
    },
)

def rust_release_binary(name, triples = TRIPLES, visibility = None, **kwargs):
    rust_binary(
        name = name + "_unstripped",
        tags = ["manual"],
        **kwargs
    )

    strip_binary(
        name = name,
        src = name + "_unstripped",
        visibility = visibility,
    )

    for triple in triples:
        platform_data(
            name = name + "_" + triple,
            platform = "@rules_rs//rs/platforms:" + triple,
            target = name,
        )
