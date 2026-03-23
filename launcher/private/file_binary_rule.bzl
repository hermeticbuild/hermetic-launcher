"""Implementation of the file_binary rule."""

DOC = """\
Defines an executable from a single file.
"""

ATTRS = dict(
    exe = attr.label(
        doc = "An executable file.",
        allow_single_file = True,
    ),
)

def _file_binary_impl(ctx):
    ctx.actions.symlink(output = ctx.outputs.executable, target_file = ctx.file.exe, is_executable = True)
    return [DefaultInfo(
        files = depset([ctx.outputs.executable]),
        executable = ctx.outputs.executable,
    )]

def _host_transition_impl(settings, _attr):
    host_platform = settings["//command_line_option:host_platform"]
    return {
        "//command_line_option:platforms": str(host_platform),
    }

_host_transition = transition(
    implementation = _host_transition_impl,
    inputs = ["//command_line_option:host_platform"],
    outputs = ["//command_line_option:platforms"],
)

file_binary = rule(
    implementation = _file_binary_impl,
    attrs = ATTRS,
    doc = DOC,
    executable = True,
)

host_file_binary = rule(
    implementation = _file_binary_impl,
    attrs = ATTRS,
    doc = DOC,
    executable = True,
    cfg = _host_transition,
)
