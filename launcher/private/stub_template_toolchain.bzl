"""Implementation of the stub_template_toolchain rule."""

load("//launcher/private/providers:stub_template_toolchain_info.bzl", "TemplateToolchainInfo")

DOC = """\
Defines an stub template toolchain.

The template can be finalized to create a stub that runs some other tool.

See https://bazel.build/extending/toolchains#defining-toolchains.
"""

ATTRS = dict(
    template_exe = attr.label(
        doc = "A template binary that can be finalized.",
        allow_single_file = True,
    ),
)

TOOLCHAIN_TYPE = str(Label("//launcher:template_toolchain_type"))

def _stub_template_toolchain_impl(ctx):
    stub_template_toolchain_info = TemplateToolchainInfo(
        template_exe = ctx.file.template_exe,
    )
    toolchain_info = platform_common.ToolchainInfo(
        templatetoolchaininfo = stub_template_toolchain_info,
        # FIXME: This typoed name was used by accident.
        # rulesets should use the correct spelling.
        # We will remove this field in a few releases.
        tempaltetoolchaininfo = stub_template_toolchain_info,
    )

    return [toolchain_info]

stub_template_toolchain = rule(
    implementation = _stub_template_toolchain_impl,
    attrs = ATTRS,
    doc = DOC,
)
