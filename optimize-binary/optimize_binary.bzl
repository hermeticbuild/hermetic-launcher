"""A post-link binary optimization rule.

`optimize_binary` runs the in-tree `//optimize-binary` tool over an already-linked
binary to apply a target-OS-specific pipeline of size optimizations, producing a
new file. The tool shells out to `llvm-objcopy`, taken from the LLVM module's
public, exec-aware `@llvm//tools:llvm-objcopy` alias (a `select` over the exec
platform), so the optimization runs on the build host while targeting any platform.

The target OS is supplied explicitly (`target_os`) rather than inferred from the
configuration: the binary is produced under a platform transition (the per-triple
`platform_data` wrapper), but this rule consumes that file from the top-level
configuration, so its own constraints would describe the host, not the target.
"""

def _optimize_binary_impl(ctx):
    out = ctx.actions.declare_file(ctx.attr.name)
    objcopy = ctx.executable.objcopy
    src = ctx.executable.src

    ctx.actions.run(
        outputs = [out],
        inputs = [src],
        executable = ctx.executable._optimizer,
        tools = [objcopy],
        arguments = [
            "--input",
            src.path,
            "--output",
            out.path,
            "--target-os",
            ctx.attr.target_os,
            "--objcopy",
            objcopy.path,
        ],
        mnemonic = "OptimizeBinary",
        progress_message = "Optimizing %{label}",
    )

    return [DefaultInfo(
        files = depset([out]),
        executable = out,
    )]

optimize_binary = rule(
    implementation = _optimize_binary_impl,
    executable = True,
    attrs = {
        "src": attr.label(
            doc = "The linked binary to optimize.",
            executable = True,
            cfg = "target",
            mandatory = True,
        ),
        "target_os": attr.string(
            doc = "OS the binary runs on: linux, macos, or windows.",
            mandatory = True,
            values = ["linux", "macos", "windows"],
        ),
        "objcopy": attr.label(
            doc = "llvm-objcopy for the exec platform.",
            default = "@llvm//tools:llvm-objcopy",
            executable = True,
            cfg = "exec",
            allow_single_file = True,
        ),
        "_optimizer": attr.label(
            default = "//optimize-binary",
            executable = True,
            cfg = "exec",
        ),
    },
)
