"""Defines providers for a stub template for a specific target platform."""

DOC = """\
Information about a template binary toolchain that can be finalized.
"""

FIELDS = dict(
    template_exes = "All template executables, from which the finalizer selects (list of File).",
    template_exe = "The template executable (File).",
)

TemplateToolchainInfo = provider(
    doc = DOC,
    fields = FIELDS,
)
