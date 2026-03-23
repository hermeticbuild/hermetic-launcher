def _copy_to_directory_impl(ctx):
    out = ctx.actions.declare_directory(ctx.label.name)
    cmds = ["set -euo pipefail", "rm -rf \"$OUT\" && mkdir -p \"$OUT\""]
    for f in ctx.files.srcs:
        cmds.append("cp {} \"$OUT\"/{}".format(f.path, f.basename))
        cmds.append("chmod +x \"$OUT\"/{}".format(f.basename))
    ctx.actions.run_shell(
        inputs = ctx.files.srcs,
        outputs = [out],
        command = "\n".join(cmds),
        mnemonic = "CopyToDirectory",
        progress_message = "Copying binaries into {}".format(ctx.label.name),
        env = {"OUT": out.path},
    )
    return [DefaultInfo(files = depset([out]))]

copy_to_directory = rule(
    implementation = _copy_to_directory_impl,
    attrs = {
        "srcs": attr.label_list(allow_files = True),
    },
)
