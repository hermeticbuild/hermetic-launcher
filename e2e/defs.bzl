"""Test helpers for running a launcher_binary against both stub sources.

`launcher_e2e_test` runs a launcher (built with the `launcher_binary` rule) as a
test twice: once against the prebuilt stubs downloaded from the GitHub release, and
once against the stubs built from source in this repo. A per-target transition pins
`//launcher/private:stub_source` for each variant, so a single `bazel test //e2e:all`
exercises both without needing two separate `bazel` invocations or config flags.
"""

_STUB_SOURCE = "//launcher/private:stub_source"

def _stub_source_transition_impl(_settings, attr):
    return {_STUB_SOURCE: attr.stub_source}

_stub_source_transition = transition(
    implementation = _stub_source_transition_impl,
    inputs = [],
    outputs = [_STUB_SOURCE],
)

def _flavored_launcher_test_impl(ctx):
    # An attribute with a transition is accessed as a list.
    launcher = ctx.attr.launcher
    if type(launcher) == "list":
        launcher = launcher[0]
    src = launcher[DefaultInfo].files_to_run.executable
    out = ctx.actions.declare_file(ctx.label.name + (".exe" if src.extension == "exe" else ""))
    ctx.actions.symlink(output = out, target_file = src, is_executable = True)
    runfiles = ctx.runfiles(files = [src]).merge(launcher[DefaultInfo].default_runfiles)
    return [DefaultInfo(executable = out, runfiles = runfiles)]

_flavored_launcher_test = rule(
    doc = "Runs `launcher` as a test with //launcher/private:stub_source pinned to `stub_source`.",
    implementation = _flavored_launcher_test_impl,
    attrs = {
        "launcher": attr.label(
            doc = "The launcher_binary to run.",
            mandatory = True,
            cfg = _stub_source_transition,
        ),
        "stub_source": attr.string(
            doc = "Which stub source to build the launcher against.",
            mandatory = True,
            values = ["prebuilt", "source_built"],
        ),
        "_allowlist_function_transition": attr.label(
            default = "@bazel_tools//tools/allowlists/function_transition_allowlist",
        ),
    },
    test = True,
)

def launcher_e2e_test(name, launcher, **kwargs):
    """Runs `launcher` as a test against both the prebuilt and source-built stubs.

    Emits `<name>_prebuilt` and `<name>_source_built` test targets plus a
    `<name>` test_suite grouping them, so `bazel test //e2e:all` (or
    `bazel test //e2e:<name>`) always runs both.

    Args:
        name: Base name; also the name of the grouping test_suite.
        launcher: A `launcher_binary` target to run as the test.
        **kwargs: Common attributes (tags, visibility, ...) applied to both tests.
    """
    tests = []
    for stub_source in ["prebuilt", "source_built"]:
        test_name = "{}_{}".format(name, stub_source)
        tests.append(test_name)
        _flavored_launcher_test(
            name = test_name,
            launcher = launcher,
            stub_source = stub_source,
            **kwargs
        )
    native.test_suite(name = name, tests = tests)
