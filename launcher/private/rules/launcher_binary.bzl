load(":lib.bzl", "launcher")

def _launcher_binary_impl(ctx):
    output_basename = ctx.label.name
    if ctx.target_platform_has_constraint(ctx.attr._windows_constraint[platform_common.ConstraintValueInfo]):
        output_basename += ".exe"
    exe = ctx.actions.declare_file(output_basename)

    # The entrypoint always needs runfiles expansion at runtime.
    embedded_args, transformed_args = launcher.args_from_entrypoint(
        executable_file = ctx.executable.entrypoint,
    )

    should_infer_transformed_args = len(ctx.attr.transformed_args) == 0
    if should_infer_transformed_args:
        # Auto-detect transformed args. Matches both:
        #   $(rlocationpath //the:target)
        #   $(rlocationpath //the:target)/some/suffix
        for (i, arg) in enumerate(ctx.attr.embedded_args):
            if "$(rlocationpath " in arg or "$(rlocationpaths " in arg:
                transformed_args.append(i+1)
    else:
        if ctx.attr.transformed_args == [-1]:
            # If we get the special value [-1], disable transformation entirely.
            transformed_args = []
        else:
            # User-specified transformed args.
            transformed_args = [str(arg) for arg in ctx.attr.transformed_args]

    for arg in ctx.attr.embedded_args:
        embedded_args, transformed_args = launcher.append_embedded_arg(
            arg = ctx.expand_location(arg, targets = [ctx.attr.entrypoint] + ctx.attr.data),
            embedded_args = embedded_args,
            transformed_args = transformed_args,
        )

    launcher.compile_stub(
        ctx = ctx,
        embedded_args = embedded_args,
        transformed_args = transformed_args,
        output_file = exe,
    )

    runfiles = ctx.runfiles(
        files = [ctx.executable.entrypoint] + ctx.files.data,
    )
    transitive_runfiles = []
    for runfiles_attr in (
        [ctx.attr.entrypoint],
        ctx.attr.data,
    ):
        for target in runfiles_attr:
            if hasattr(target, "default_runfiles"):
                transitive_runfiles.append(target[DefaultInfo].default_runfiles)
    runfiles = runfiles.merge_all(transitive_runfiles)
    return [DefaultInfo(
        files = depset([exe]),
        executable = exe,
        runfiles = runfiles,
    )]


launcher_binary = rule(
    doc = """Creates a tiny, binary launcher that wraps another executable with its runfiles.

This rule generates a small native binary (10-68KB depending on platform) that:
- Resolves Bazel runfiles paths at runtime
- Executes the target program with embedded arguments
- Forwards additional runtime arguments
- Exports runfiles environment variables to child processes
- Works identically on Linux, macOS, and Windows

The launcher automatically detects runfiles using RUNFILES_DIR, RUNFILES_MANIFEST_FILE,
or by looking for a <binary>.runfiles/ directory adjacent to the executable.

**Example - Wrapping openssl to hash a file:**

```python
load("@hermetic_launcher//launcher:launcher_binary.bzl", "launcher_binary")

launcher_binary(
    name = "hash_file",
    entrypoint = "@openssl",
    embedded_args = [
        "dgst",
        "-sha256",
        "$(rlocationpath :file_to_hash.txt)",
    ],
    data = [":file_to_hash.txt"],
)
```

The launcher will:
1. Resolve the openssl binary location through runfiles
2. Resolve file_to_hash.txt location through runfiles (auto-detected from $(rlocationpath))
3. Execute: `openssl dgst -sha256 /resolved/path/to/file_to_hash.txt`
4. Export RUNFILES_DIR, RUNFILES_MANIFEST_FILE, and JAVA_RUNFILES to the child process

**Runtime argument forwarding:**

Additional arguments passed to the binary are forwarded to the entrypoint:

```bash
bazel run //:hash_file -- --some-extra-flag
# Executes: openssl dgst -sha256 /resolved/path/to/file.txt --some-extra-flag
```

**Automatic path transformation:**

By default, the entrypoint (index 0) and any argument matching `$(rlocationpath ...)`
are automatically transformed through runfiles. You can customize this with `transformed_args`.

**Cross-platform:**

The same BUILD file works on Linux, macOS, and Windows. The launcher handles platform-specific
path separators and runfiles resolution automatically.
""",
    implementation = _launcher_binary_impl,
    attrs = {
        "entrypoint": attr.label(
            doc = """The target executable to wrap. This is the actual program that will be executed.

The entrypoint's runfiles path will be automatically resolved at runtime through the Bazel
runfiles mechanism. This must be an executable target (e.g., a binary, a script, or another
launcher_binary).

Example: `entrypoint = "@python_3_11//:python"` or `entrypoint = "//tools:my_tool"`
""",
            executable = True,
            cfg = "target",
            mandatory = True,
        ),
        "embedded_args": attr.string_list(
            doc = """Arguments to embed in the binary that will be passed to the entrypoint.

These arguments are baked into the binary at build time. They support Bazel location
expansion (e.g., `$(rlocationpath)`, `$(location)`, `$(execpath)`).

Arguments matching the pattern `$(rlocationpath ...)` are automatically detected and marked
for runtime path transformation unless you explicitly set `transformed_args`.

Example:
```python
embedded_args = [
    "--config",
    "$(rlocationpath :config.yaml)",  # Auto-detected for transformation
    "--mode=production",               # Literal string, not transformed
]
```
""",
        ),
        "transformed_args": attr.int_list(
            doc = """Explicit list of argument indices that need runtime runfiles path transformation.

Index 0 is the entrypoint, index 1 is the first embedded arg, index 2 is the second, etc.

**Default behavior (empty list):**
- Entrypoint (index 0) is always transformed
- Any argument matching `$(rlocationpath ...)` is transformed
- Other arguments are passed as literal strings

**Custom behavior:**
Set explicit indices to transform:
```python
transformed_args = [0, 2, 5]  # Transform entrypoint, 2nd arg, and 5th arg
```

**Disable all transformation:**
```python
transformed_args = [-1]  # Special value: no transformation at all
```

Use this when you want fine-grained control over which paths are resolved through runfiles.
""",
            default = [],
        ),
        "data": attr.label_list(
            doc = """Runtime dependencies (data files, scripts, etc.) needed by the entrypoint.

These files and their transitive runfiles will be included in the binary's runfiles tree,
making them available at runtime. Use `$(rlocationpath)` to reference these files in
`embedded_args`.

Example:
```python
data = [
    ":config.yaml",
    "//data:test_fixtures",
    "@some_external//:data_files",
]
```
""",
            allow_files = True,
        ),
        "_windows_constraint": attr.label(default = "@platforms//os:windows"),
    },
    executable = True,
    toolchains = [
        "@hermetic_launcher//launcher:template_toolchain_type",
        "@hermetic_launcher//launcher:finalizer_toolchain_type",
    ],
)
