"""Registers the launcher template + finalizer toolchains for one stub source.

Both the prebuilt (downloaded) and the source-built stub toolchains are declared
through this macro so the two sets stay in lockstep: same platforms, same
constraints, differing only in where the template/finalizer binaries come from and
in the `//launcher/private:stub_source` value that selects them.
"""

load(":stub_finalizer_toolchain.bzl", "stub_finalizer_toolchain")
load(":stub_template_toolchain.bzl", "stub_template_toolchain")

def _executable_file_impl(ctx):
    # Re-expose only the executable as the single default output. The prebuilt
    # finalizer is a downloaded file (single output already), but the source-built
    # finalizer is a platform_data target whose default outputs include more than
    # one file, which the toolchain's `allow_single_file` finalizer attr rejects.
    return [DefaultInfo(files = depset([ctx.attr.src[DefaultInfo].files_to_run.executable]))]

_executable_file = rule(
    doc = "Exposes a target's executable as a single-file output.",
    implementation = _executable_file_impl,
    attrs = {"src": attr.label(mandatory = True)},
)

# The platforms toolchains are registered for. This is intentionally a subset of
# the released triples: s390x and Windows/aarch64 ship for standalone use but have
# no auto-registered Bazel toolchain (see README "Supported platforms").
#
# (name, cpu, os)
PLATFORMS = [
    ("aarch64_linux", "aarch64", "linux"),
    ("aarch64_macos", "aarch64", "macos"),
    ("x86_64_linux", "x86_64", "linux"),
    ("x86_64_macos", "x86_64", "macos"),
    ("x86_64_windows", "x86_64", "windows"),
]

def launcher_stub_toolchains(name, *, templates, finalizers):
    """Declares and exposes the template + finalizer toolchains for one stub source.

    Args:
        name: The stub source, "prebuilt" or "source_built". Used both as the
            target-name namespace and to select the matching
            `//launcher/private:stub_source_<name>` config_setting via
            `target_settings`, so exactly one source's toolchains resolve for a
            given value of the `//launcher/private:stub_source` flag.
        templates: dict mapping each platform name in `PLATFORMS` to the template
            (runfiles-stub) label for that platform.
        finalizers: dict mapping each platform name in `PLATFORMS` to the finalizer
            (finalize-stub) label for that platform.
    """
    if name not in ("prebuilt", "source_built"):
        fail("launcher_stub_toolchains name must be \"prebuilt\" or \"source_built\", got %r" % name)

    target_settings = ["//launcher/private:stub_source_" + name]

    for (platform, cpu, os) in PLATFORMS:
        constraints = [
            "@platforms//cpu:" + cpu,
            "@platforms//os:" + os,
        ]

        stub_template_toolchain(
            name = "template_%s_%s" % (platform, name),
            template_exe = templates[platform],
        )
        native.toolchain(
            name = "template_%s_%s_toolchain" % (platform, name),
            target_compatible_with = constraints,
            target_settings = target_settings,
            toolchain = ":template_%s_%s" % (platform, name),
            toolchain_type = "//launcher:template_toolchain_type",
        )
        native.toolchain(
            name = "template_%s_%s_exec_toolchain" % (platform, name),
            exec_compatible_with = constraints,
            target_settings = target_settings,
            toolchain = ":template_%s_%s" % (platform, name),
            toolchain_type = "//launcher:template_exec_toolchain_type",
        )

        _executable_file(
            name = "finalizer_file_%s_%s" % (platform, name),
            src = finalizers[platform],
        )
        stub_finalizer_toolchain(
            name = "finalizer_%s_%s" % (platform, name),
            finalizer = ":finalizer_file_%s_%s" % (platform, name),
        )
        native.toolchain(
            name = "finalizer_%s_%s_toolchain" % (platform, name),
            exec_compatible_with = constraints,
            target_settings = target_settings,
            toolchain = ":finalizer_%s_%s" % (platform, name),
            toolchain_type = "//launcher:finalizer_toolchain_type",
        )
