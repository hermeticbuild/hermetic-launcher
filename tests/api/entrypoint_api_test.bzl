"""Analysis tests verifying that `launcher.entrypoint()`'s method-chaining API is equivalent to
hand-written calls into the low-level `launcher.*` functions it's built on top of (see PR #67).

For any given sequence of appended embedded args, runfiles, and raw transformed args, both APIs
must produce:
  1. identical `embedded_args` / `transformed_args` lists, and
  2. an identical finalizer action (aside from the declared output path).

These tests don't exercise a "real" rule; they exercise the library functions in
//launcher/private/rules:lib.bzl the way a rule implementation would.
"""

load("@rules_testing//lib:analysis_test.bzl", "analysis_test", "test_suite")
load("//launcher/private/rules:lib.bzl", "launcher")

_FAKE_ENTRYPOINT = "//tests/api:fake_entrypoint"
_FAKE_DATA = [
    "//tests/api:fake_env_config.txt",
    "//tests/api:fake_config.txt",
    "//tests/api:fake_main.txt",
]
_COMMON_OPS = {
    "embedded_args-1": "--config=foo",
    "embedded_args-2": "-n",
    "embedded_args-3": "--verbose",
    "transformed_args-1": "sibling/relative/path",
    "transformed_args-2": "../go/back/path",
    "transformed_args-3": "/absolute/path",
}

_RUNFILE_OPS = {
    "runfile-1": _FAKE_DATA[0],
    "runfile-2": _FAKE_DATA[1],
    "runfile-3": _FAKE_DATA[2],
}

_VALID_OPS = _COMMON_OPS | _RUNFILE_OPS

_EntrypointFixtureInfo = provider(
    fields = [
        "embedded_manual",
        "transformed_manual",
        "embedded_chained",
        "transformed_chained",
    ],
)

def _drop_output_flag(argv):
    """Strips the `-o <output path>` pair out of an argv, so that two actions
    differing only in their declared output file can be compared for
    equality."""

    result = []
    skip_next = False
    for arg in argv:
        if skip_next:
            skip_next = False
            continue
        if arg == "-o":
            skip_next = True
            continue
        result.append(arg)
    return result

def _run_launcher_ops_impl(ctx):
    """
    Given a list of operations, to perform them in both the manual and chained APIs
    """
    entrypoint_file = ctx.executable._entrypoint

    embedded, transformed = launcher.args_from_entrypoint(executable_file = entrypoint_file)
    entrypoint = launcher.entrypoint(entrypoint_file)

    for op in ctx.attr.ops:
        if op.startswith("embedded_args-"):
            entrypoint = entrypoint.embedded_args(ctx.attr._ops[op])
            embedded, transformed = launcher.append_embedded_arg(
                arg = ctx.attr._ops[op],
                embedded_args = embedded,
                transformed_args = transformed,
            )
            continue

        if op.startswith("transformed_args-"):
            entrypoint = entrypoint.raw_transformed_args(ctx.attr._ops[op])
            embedded, transformed = launcher.append_raw_transformed_arg(
                arg = ctx.attr._ops[op],
                embedded_args = embedded,
                transformed_args = transformed,
            )
            continue

        if op.startswith("runfile-"):
            index = int(op.removeprefix("runfile-")) - 1
            entrypoint = entrypoint.runfiles(ctx.files._runfile_ops[index])
            embedded, transformed = launcher.append_runfile(
                file = ctx.files._runfile_ops[index],
                embedded_args = embedded,
                transformed_args = transformed,
            )
            continue

        fail("unknown op: {}".format(op))

    manual_out = ctx.actions.declare_file(ctx.label.name + "_manual")
    launcher.compile_stub(
        ctx = ctx,
        embedded_args = embedded,
        transformed_args = transformed,
        output_file = manual_out,
    )

    chained_out = ctx.actions.declare_file(ctx.label.name + "_chained")
    entrypoint.compile(ctx, output_file = chained_out)

    return [
        DefaultInfo(files = depset([manual_out, chained_out])),
        _EntrypointFixtureInfo(
            embedded_manual = [embedded],
            transformed_manual = [transformed],
            embedded_chained = [entrypoint._embedded_args()],
            transformed_chained = [entrypoint._transformed_args()],
        ),
    ]

_run_launcher_ops = rule(
    doc = """
    Test-only rule that applies the same operations to both launcher APIs for comparison.
    """,
    implementation = _run_launcher_ops_impl,
    attrs = {
        "ops": attr.string_list(mandatory = True, doc = "Operations to perform"),
        "_entrypoint": attr.label(executable = True, cfg = "target", default = _FAKE_ENTRYPOINT),
        "_data": attr.label_list(allow_files = True, default = _FAKE_DATA),
        "_ops": attr.string_dict(default = _COMMON_OPS),
        "_runfile_ops": attr.string_keyed_label_dict(
            allow_files = True,
            default = _RUNFILE_OPS,
        ),
    },
    toolchains = [
        launcher.template_toolchain_type,
        launcher.finalizer_toolchain_type,
    ],
)

def _assert_compiled_actions_are_identical(env, target):
    info = target[_EntrypointFixtureInfo]
    target_subject = env.expect.that_target(target)
    manual_action = target_subject.action_generating("{package}/{name}_manual").actual
    chained_action = target_subject.action_generating("{package}/{name}_chained").actual

    (env.expect
        .that_str(chained_action.mnemonic)
        .equals(manual_action.mnemonic))

    (env.expect
        .that_collection(_drop_output_flag(chained_action.argv))
        .contains_exactly(_drop_output_flag(manual_action.argv))
        .in_order())

    (env.expect
        .that_collection(
        [f.path for f in chained_action.inputs.to_list()],
    )
        .contains_exactly(
        [f.path for f in manual_action.inputs.to_list()],
    )
        .in_order())

    (env.expect
        .that_collection(info.embedded_chained)
        .contains_exactly(info.embedded_manual)
        .in_order())

    (env.expect
        .that_collection(info.transformed_chained)
        .contains_exactly(info.transformed_manual)
        .in_order())

def _next_seed(seed):
    # Linear congruential generator (LCG)
    return (seed * 2458713 + 786245) % 1000

def _generate_ops_from_seed(seed, max_ops):
    "Given a seed, generates a list containing max_ops elements of _VALID_OPS."

    ops_keys = _VALID_OPS.keys()
    result = []
    current_seed = seed
    for i in range(max_ops):
        index = (current_seed + i) % len(ops_keys)
        op_key = ops_keys[index]
        current_seed = _next_seed(current_seed)
        result.append(op_key)
    return result

def _generate_test_cases(name, seed):
    """Generates a list of test cases for the entrypoint API tests."""
    current_seed = seed
    test_cases = []

    for i in range(100):
        subject_target = "{}_subject_{}".format(name, i)
        test_target = "{}_{}".format(name, i)
        _run_launcher_ops(
            name = subject_target,
            tags = ["manual"],
            ops = _generate_ops_from_seed(current_seed, current_seed % 9),
        )
        current_seed = _next_seed(current_seed)
        analysis_test(
            name = test_target,
            target = subject_target,
            impl = _assert_compiled_actions_are_identical,
        )
        test_cases.append(test_target)
    return test_cases

def entrypoint_api_test_suite(name, seed):
    native.test_suite(
        name = name,
        tests = _generate_test_cases(name, seed),
    )
