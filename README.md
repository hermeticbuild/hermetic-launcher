# Hermetic Launcher

Tiny, cross-platform native launchers that replace shell-script wrappers in Bazel.
A launcher resolves its target's runfiles, forwards arguments, and `exec`s the real
program — in ~16–30 KB, identically on Linux, macOS, and Windows.

## Why

Bazel rules often wrap tools in generated shell scripts to set up runfiles. Shell
scripts aren't portable: bash doesn't run on Windows, `.bat` doesn't run on Unix.
Hermetic Launcher replaces them with a small native binary that does the same job on
every platform. Each launcher is produced by byte-patching a prebuilt template, so a
build on *any* host can emit a launcher for *any* target platform — deterministically
and with byte-identical output.

---

## Use in Bazel

Add the module (toolchains for the prebuilt templates and finalizers register
automatically):

```python
# MODULE.bazel
bazel_dep(name = "hermetic_launcher", version = "<latest>")  # see the Bazel Central Registry
```

Wrap an executable with `launcher_binary`:

```python
load("@hermetic_launcher//launcher:launcher_binary.bzl", "launcher_binary")

launcher_binary(
    name = "hash_file",
    entrypoint = "@openssl",
    embedded_args = [
        "dgst",
        "-sha256",
        "$(rlocationpath :input.txt)",  # auto-resolved through runfiles
    ],
    data = [":input.txt"],
)
```

At runtime this resolves `openssl` and `input.txt` through runfiles and runs
`openssl dgst -sha256 /abs/path/to/input.txt`. Extra arguments are appended:

```bash
bazel run //:hash_file -- --extra-flag
# openssl dgst -sha256 /abs/path/to/input.txt --extra-flag
```

`RUNFILES_DIR`, `RUNFILES_MANIFEST_FILE`, and `JAVA_RUNFILES` are exported to the child
process so it can use Bazel's runfiles libraries.

### Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `entrypoint` | label (required) | Executable to run. Always resolved through runfiles. |
| `embedded_args` | string list | Arguments baked into the binary. Support location expansion; any `$(rlocationpath …)` / `$(rlocationpaths …)` arg is auto-marked for runfiles resolution. |
| `data` | label list | Runtime files; included in the launcher's runfiles tree. |
| `transformed_args` | int list | Indices of args to resolve through runfiles (`0` = entrypoint, `1` = first `embedded_args` entry, …). Default `[]` auto-detects (entrypoint + `$(rlocationpath …)` args). An explicit list **replaces** the default — include `0` to keep resolving the entrypoint. `[-1]` disables all resolution. |

### Low-level rule API

For custom rules that need to build launchers programmatically, use `launcher.entrypoint`
to build up the launcher's arguments fluently, then `.compile()` to emit the binary.
This is the idiomatic way to use the low-level API:

```python
load("@hermetic_launcher//launcher:lib.bzl", "launcher")

def _impl(ctx):
    exe = ctx.actions.declare_file(ctx.label.name)
    (launcher.entrypoint(ctx.executable.tool)
        .runfiles(ctx.file.config)
        .embedded_args("--verbose")
        .compile(ctx, output_file = exe))  # cfg = "target" by default, or cfg = "exec"
    ...
```

| Method | Purpose |
|--------|---------|
| `entrypoint(executable_file)` | Start a builder seeded with the entrypoint at index 0. |
| `.runfiles(*files)` | Append `File`s, each marked for runfiles resolution. |
| `.embedded_args(*args)` | Append literal string arguments. |
| `.raw_transformed_args(*args)` | Append string arguments marked for runfiles resolution (for paths not backed by a `File`). |
| `.compile(ctx, output_file, cfg, template_exec_group, template_file)` | Run the finalizer and emit the launcher. `cfg` is `"target"` (default) or `"exec"`. |

The builder is a thin wrapper over a set of functional helpers, which remain available
for cases that need direct control over the `(embedded_args, transformed_args)` lists:

```python
def _impl(ctx):
    exe = ctx.actions.declare_file(ctx.label.name)
    embedded, transformed = launcher.args_from_entrypoint(ctx.executable.tool)
    embedded, transformed = launcher.append_runfile(
        file = ctx.file.config, embedded_args = embedded, transformed_args = transformed)
    embedded, transformed = launcher.append_embedded_arg(
        arg = "--verbose", embedded_args = embedded, transformed_args = transformed)
    launcher.compile_stub(
        ctx = ctx, embedded_args = embedded, transformed_args = transformed,
        output_file = exe, cfg = "target")  # or cfg = "exec"
    ...
```

| Function | Purpose |
|----------|---------|
| `args_from_entrypoint(executable_file)` | Seed `(embedded_args, transformed_args)` with the entrypoint at index 0. |
| `append_runfile(file, …)` | Append a `File`, marked for runfiles resolution. |
| `append_embedded_arg(arg, …)` | Append a literal string argument. |
| `append_raw_transformed_arg(arg, …)` | Append a string argument marked for resolution. |
| `to_rlocation_path(file)` | Convert a `File` to its rlocation path string. |
| `compile_stub(ctx, embedded_args, transformed_args, output_file, cfg, template_exec_group, template_file)` | Run the finalizer to emit the launcher. `cfg` is `"target"` (default) or `"exec"`. |

Declare the relevant toolchains on your rule:

```python
toolchains = [
    launcher.finalizer_toolchain_type,
    launcher.template_toolchain_type,       # for cfg = "target"
    # launcher.template_exec_toolchain_type # for cfg = "exec"
]
```

---

## Standalone use (without Bazel)

The launcher is two binaries per platform, downloadable from
[GitHub releases](https://github.com/hermeticbuild/hermetic-launcher/releases):

- **`runfiles-stub-<arch>-<os>`** — the _template_: a complete stub with placeholder
  bytes where the arguments go.
- **`finalize-stub-<arch>-<os>`** — the _finalizer_: patches a template's placeholders
  with concrete arguments and writes a ready-to-run launcher. It is pure byte patching,
  so it runs on any host and targets any platform.

```bash
# Bake `_main/echo` into a launcher and mark argument 0 for runfiles resolution.
finalize-stub --template runfiles-stub-x86_64-linux --transform 0 -o my_echo -- _main/echo

# A manifest maps runfiles paths to real paths (a runfiles directory works too).
echo '_main/echo /bin/echo' > manifest.txt

RUNFILES_MANIFEST_FILE=manifest.txt ./my_echo "hello" a b
# runs: /bin/echo hello a b
```

### `finalize-stub` options

```
finalize-stub --template <PATH> [OPTIONS] -- <arg0> [arg1 ...]

-t, --template <PATH>            Template binary to patch (required)
-o, --output <PATH>              Output path (default: stdout; chmod +x on Unix)
    --transform <N>              Mark embedded arg N (0–9) for runfiles resolution.
                                 Repeatable or comma-separated. Default: none.
    --export-runfiles-env <B>    Export RUNFILES_DIR/RUNFILES_MANIFEST_FILE/JAVA_RUNFILES
                                 to the child (default: true)
-v, --verbose                    Verbose output
```

Up to 10 embedded arguments (`arg0`–`arg9`), each ≤ 256 bytes. `arg0` is the program to
execute; the rest are its leading arguments. Runtime arguments are unrestricted.

### Runfiles discovery

At startup the finalized launcher selects one runfiles source from:

1. `$RUNFILES_DIR` and `$RUNFILES_MANIFEST_FILE`
2. `<executable>.runfiles/` and `<executable>.runfiles_manifest`

Valid environment-provided sources take precedence over adjacent sources. When both
sources exist at the same precedence, the runfiles directory wins on platforms that
materialize the runfiles tree (Linux, macOS); on Windows the tree is not materialized
by default and the directory is sparse, so the manifest wins there. Every argument
marked `--transform` resolves only through the selected source (manifest lookup or
directory join; tree-artifact prefixes supported), and only that source is exported to
the child. Absolute paths (leading `/`) pass through unchanged. The launcher then
appends its own runtime arguments and replaces itself with the target.

---

## How it works

```
runfiles-stub (template)           finalize-stub                  launcher
┌────────────────────────┐         patches placeholders:         ┌──────────────────────┐
│ argc / flags / arg0..N │  ──────▶  argc, transform bitmask,  ─▶│ same size, runs the  │
│   = placeholder bytes  │           export flag, arg values     │ embedded program     │
└────────────────────────┘                                       └──────────────────────┘
```

The finalizer scans the template for fixed-size placeholder byte patterns and
overwrites them in place — argument count, a bitmask of which args to resolve, the
export-env flag, and the argument strings. Output size equals input size, and the
result is identical regardless of which host produced it.

| OS      | Arches                 | Entry           | Syscall layer         | Process exec            | Notes                                                                 |
| ------- | ---------------------- | --------------- | --------------------- | ----------------------- | --------------------------------------------------------------------- |
| Linux   | x86_64, aarch64, s390x | custom `_start` | raw syscalls, no libc | `execve`                | fully static (musl), zero deps                                        |
| macOS   | x86_64, aarch64        | `main`          | libSystem             | `execve`                | finalizer re-signs ad-hoc (patching invalidates the Mach-O signature) |
| Windows | x86_64, aarch64        | `main`          | Win32 (UTF-16)        | `CreateProcessW` + wait | converts `/` → `\`                                                    |

The stubs are `no_std` Rust with a static-arena allocator. Patched Mach-O binaries are
re-signed automatically by the finalizer.

### Supported platforms

Templates and finalizers are released for all seven targets above. Bazel toolchains are
auto-registered for **Linux x86_64/aarch64, macOS x86_64/aarch64, and Windows x86_64**;
the **s390x** and **Windows/aarch64** binaries ship in releases for standalone use.

---

## Build & test

Requires [Bazel](https://bazel.build) (see `.bazelversion`; Bazelisk picks it up).
Cross-compilation is handled entirely by Bazel via `rules_rs` and LLVM toolchains.

```bash
# Build every release binary (7 templates + 7 finalizers) into ./artifacts
bash tools/build-release-binaries.sh artifacts

# Tests
bazel test //integration-tests:integration_test   # finalize + run on the host platform
bazel test //e2e:all                               # launcher_binary chain vs. BOTH stub sources
(cd e2e/bzlmod && bazel test //...)                # launcher_binary wrapping cc/go/py/sh
```

`//e2e:all` runs the same launcher chain twice — `e2e_chain_prebuilt` against the
prebuilt stubs downloaded from the release, and `e2e_chain_source_built` against the
stubs freshly built from this checkout (a per-target transition flips
`//launcher/private:stub_source`; add `--config=release` to build them exactly as they
ship). `e2e/bzlmod` is the downstream/BCR test module: it consumes the published
launcher as an external Bazel module, so it only exercises the prebuilt stubs.

`flake.nix` provides a dev shell (Rust, Wine for Windows testing, gdb).

## Updating the prebuilt binaries

1. Check out the latest commit of `main` and create a `binaries-YYYYMMDD` tag, then push it:
   ```bash
   git checkout main && git pull
   git tag binaries-$(date +%Y%m%d)
   git push origin binaries-$(date +%Y%m%d)
   ```
2. The [release workflow](.github/workflows/release.yml) builds all 14 binaries and publishes a GitHub release with a `SHA256SUMS.txt`.
3. Once the release is published, run the updater and commit:
   ```bash
   bazel run //tools:update-binaries
   git add launcher/private/extensions.bzl
   git commit -m "chore: update prebuilt binaries to binaries-YYYYMMDD"
   ```

---

## License

MIT — see [LICENSE](LICENSE).
