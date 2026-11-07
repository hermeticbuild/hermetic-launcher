"""Analysis tests that the prebuilt stub toolchains resolve for each registered platform."""

load("@rules_testing//lib:analysis_test.bzl", "analysis_test")

_ResolvedStubsInfo = provider(
    doc = "The template and finalizer files toolchain resolution picked.",
    fields = ["template", "finalizer"],
)

def _resolved_stubs_impl(ctx):
    return [_ResolvedStubsInfo(
        template = ctx.toolchains["//launcher:template_toolchain_type"].templatetoolchaininfo.template_exe,
        finalizer = ctx.toolchains["//launcher:finalizer_toolchain_type"].finalizer_info.finalizer,
    )]

resolved_stubs = rule(
    implementation = _resolved_stubs_impl,
    toolchains = [
        "//launcher:template_toolchain_type",
        "//launcher:finalizer_toolchain_type",
    ],
)

def _repo_name(file):
    # A canonical repo name such as `+non_module_dependencies+runfiles_stub_aarch64_windows`.
    return file.owner.repo_name.split("+")[-1]

def _prebuilt_stubs_test(name, *, platform, suffix):
    def _impl(env, target):
        resolved = target[_ResolvedStubsInfo]
        env.expect.that_str(_repo_name(resolved.template)).equals("runfiles_stub_" + suffix)
        env.expect.that_str(_repo_name(resolved.finalizer)).equals("finalize_stub_" + suffix)

    analysis_test(
        name = name,
        target = ":resolved_stubs",
        impl = _impl,
        config_settings = {
            "//command_line_option:extra_execution_platforms": [platform],
            "//command_line_option:platforms": [platform],
        },
    )

def prebuilt_toolchains_test_suite(name, platforms):
    """Declares one test per platform that the prebuilt stubs for it resolve.

    Args:
        name: The test_suite name.
        platforms: dict of platform name in `PLATFORMS` to a platform label.
    """
    tests = []
    for suffix, platform in platforms.items():
        test_name = "{}_{}".format(name, suffix)
        _prebuilt_stubs_test(test_name, platform = native.package_relative_label(platform), suffix = suffix)
        tests.append(test_name)
    native.test_suite(name = name, tests = tests)
