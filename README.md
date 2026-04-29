# Hermetic Launcher

A minimal, cross-platform Bazel runfiles stub runner that replaces shell scripts with tiny native binaries.

## Why This Exists

**Problem**: Many Bazel rules create shell script wrappers to invoke tools with their runfiles dependencies. Shell scripts aren't cross-platform—bash scripts don't work on Windows, batch files don't work on Unix.

**Solution**: This project provides tiny native binaries (10-68KB) that:
- Work on **Linux, macOS, and Windows**
- Resolve Bazel runfiles paths
- Forward arguments to the actual tool
- Can be **"cross-compiled"** (finalized) from any build platform for any target platform

## Primary Use Case: Bazel Rules

Instead of generating platform-specific shell scripts like this:

```bash
#!/bin/bash
# Generated wrapper script - Linux/macOS only because of shebang!
exec $RUNFILES_DIR/my_workspace/bin/tool "$@"
```

Create a launcher binary for the target platform:

```bash
./finalize-stub \
  --template runfiles-stub-x86_64-linux \
  --transform 0 \
  --output my_tool \
  -- \
  my_workspace/bin/tool

# Runtime: works with runfiles
RUNFILES_DIR=/path/to/runfiles ./my_tool --flag arg1 arg2
```

This enables Bazel rules to create tiny, platform-agnostic entrypoints that work identically on Linux, macOS, and Windows.

## Features

- **Cross-platform**: Linux (x86_64, aarch64, s390x), macOS (x86_64, aarch64), Windows (x86_64)
- **True cross-compilation**: Finalize launcher for **any target platform** from **any build platform**
  - Build on Linux → create Windows/macOS launcher
  - Build on macOS → create Linux/Windows launcher
  - Build on Windows → create Linux/macOS launcher
- **Deterministic**: Same inputs always produce identical output, regardless of build platform
- **Tiny binaries**: 10-68KB depending on platform
- **Runtime arguments**: Forward `$@` to the wrapped tool
- **No dependencies**: Fully static on Linux, minimal dependencies on macOS/Windows

## Quick Start

### Download Pre-built Binaries

```bash
# Download from GitHub releases
VERSION=<version> # could be "binaries-20251216"
wget https://github.com/hermeticbuild/hermetic-launcher/releases/download/${VERSION}/runfiles-stub-x86_64-linux
wget https://github.com/hermeticbuild/hermetic-launcher/releases/download/${VERSION}/finalize-stub-x86_64-linux
chmod +x runfiles-stub-x86_64-linux
chmod +x finalize-stub-x86_64-linux
```

### Create a Stub

```bash
# Finalize a stub that wraps /bin/echo
./finalize-stub-x86_64-linux \
  --template runfiles-stub-x86_64-linux \
  --transform 0 \
  --output my_echo \
  -- \
  _main/echo

# Create a manifest (This maps the runfile _main/echo to /bin/echo)
# Note: you can either use a runfiles manifest or a runfiles directory.
cat > manifest.txt << 'EOF'
_main/echo /bin/echo
EOF

# Run it - embedded args + runtime args
RUNFILES_MANIFEST_FILE=manifest.txt ./my_echo "Hello from embedded!" arg1 arg2
# Output: Hello from embedded! arg1 arg2
```

The stub:
1. Resolves `_main/echo` to `/bin/echo` through runfiles (because `--transform=0`)
2. Appends runtime arguments (`arg1 arg2`)
3. Executes: `/bin/echo "Hello from embedded!" arg1 arg2`

## How It Works

### Two-Step Process

```
┌──────────────────┐
│  Template Binary │  Generic stub for a platform (10-68KB)
│ (runfiles-stub)  │  Contains placeholder sections
└────────┬─────────┘
         │
         │ finalize-stub patches placeholders with:
         │  - Number of arguments
         │  - Which args to transform (bitmask)
         │  - Actual argument values
         │
         ▼
┌──────────────────┐
│ Finalized Binary │  Ready-to-use stub (same size)
│   (my_tool)      │  Embedded args + accepts runtime args
└────────┬─────────┘
         │
         │ At runtime:
         │  1. Checks for RUNFILES_DIR, RUNFILES_MANIFEST_FILE, <executable>.runfiles_manifest, or <executable>.runfiles
         │  2. Resolves selected embedded args through runfiles
         │  3. Appends runtime $@ arguments
         │  4. Executes target with all args
         │
         ▼
    Target Program
```

### Cross-Platform Finalization

The finalizer works on any platform to create launchers for any platform:

```bash
# On Linux, create launcher for the target platform of choice
./finalize-stub-x86_64-linux --template runfiles-stub-x86_64-linux --output stub-linux -- /bin/tool
./finalize-stub-x86_64-linux --template runfiles-stub-x86_64-macos --output stub-macos -- /bin/tool
./finalize-stub-x86_64-linux --template runfiles-stub-x86_64-windows.exe --output stub.exe -- 'C:\Windows\System32\cmd.exe'

# The finalizer just patches bytes - no platform-specific logic needed!
```

This is crucial for Bazel: your **exec platform** (where the build runs) can create stubs for any **target platform** (where the output runs).

## Supported Platforms

| Platform | Architectures | Template Size | Notes |
|----------|--------------|---------------|-------|
| **Linux** | x86_64, aarch64, s390x | 10-68KB | Fully static, no dependencies |
| **macOS** | x86_64, aarch64 | 13-49KB | Links with libSystem |
| **Windows** | x86_64 | 22KB | Links with kernel32.dll, shell32.dll |

**Finalizers** (the tool that patches templates):
- Linux: x86_64, aarch64 (static musl binaries), s390x (gnu)
- macOS: x86_64, aarch64
- Windows: x86_64

## Usage

### Basic Usage

```bash
# Syntax
finalize-stub --template <template> [OPTIONS] -- <arg0> [arg1 ...]

# No transforms (default - all arguments are literal)
finalize-stub --template template --output my_tool -- my_workspace/bin/tool data/input.txt

# Transform only specific arguments (repeated flags)
finalize-stub --template template --transform 0 --transform 2 --output my_tool -- /bin/tool --flag data/file

# Transform only specific arguments (comma-separated)
finalize-stub --template template --transform 0,2 --output my_tool -- /bin/tool --flag data/file
#                                             ^^^                       ^^^        ^^^^    ^^^
#                                          arg0,arg2                 transform   literal transform
```

### Options

```
--template <PATH>           Path to template runfiles-stub binary (required)

--transform <N>             Mark argument N for runfiles resolution (0-9)
                            Can be repeated for multiple arguments (--transform 0 --transform 2)
                            or comma-separated (--transform 0,2)
                            Default: no arguments are transformed

--export-runfiles-env       Export runfiles environment variables to child process
                            Values: true (default) or false
                            When true: RUNFILES_DIR, RUNFILES_MANIFEST_FILE, and JAVA_RUNFILES
                            are set in the child process based on discovered runfiles
                            When false: child process inherits environment unchanged

--output <PATH>             Output file path (default: stdout)

--                          Separates flags from positional arguments (recommended)
```

### Runtime Arguments

Finalized stubs forward runtime arguments to the target:

```bash
# Create stub with embedded args
finalize-stub --template template --transform 0 --output stub -- /bin/grep "pattern"

# Run with additional args - they're forwarded as argv
./stub file1.txt file2.txt
# Executes: /bin/grep "pattern" file1.txt file2.txt
```

This is like bash `$@` - embedded args come first, runtime args are appended.

### Runfiles Environment

Stubs discover runfiles through environment variables or automatic detection:

```bash
# Manifest-based (file maps runfiles paths to absolute paths)
RUNFILES_MANIFEST_FILE=/path/to/manifest.txt ./stub

# Directory-based (simple directory layout)
RUNFILES_DIR=/path/to/runfiles ./stub

# Automatic fallback (discovers <stub>.runfiles/ directory)
./stub  # Looks for ./stub.runfiles/ automatically
```

#### Environment Variable Export

By default (`--export-runfiles-env=true`), stubs export runfiles environment variables to the child process:

```bash
# Create stub with export enabled (default)
finalize-stub --template template --transform 0 --output stub -- tool

# Child process receives:
#   RUNFILES_MANIFEST_FILE (if manifest-based)
#   RUNFILES_DIR (if directory-based or fallback)
#   JAVA_RUNFILES (same as RUNFILES_DIR)
```

This allows child processes to use Bazel's runfiles libraries without manual environment setup.

To disable export and keep the environment unchanged:

```bash
# Create stub with export disabled
finalize-stub --template template --export-runfiles-env=false --output stub -- tool

# Child process inherits parent environment unchanged
```

## Building from Source

### Prerequisites

- Rust toolchain (stable)
- For cross-compilation: platform toolchains (mingw-w64 for Windows, etc.)

### Build All Binaries

```bash
# Linux templates
cd runfiles-stub
cargo build --release --target x86_64-unknown-linux-gnu
cargo build --release --target aarch64-unknown-linux-gnu

# macOS templates
cargo build --release --target x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin

# Windows template
cargo build --release --target x86_64-pc-windows-gnu

# Finalizers
cd ../finalize-stub
cargo build --release --target x86_64-unknown-linux-musl
cargo build --release --target aarch64-unknown-linux-musl
cargo build --release --target x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-pc-windows-gnu
```

See `.github/workflows/release.yml` for the complete build matrix.

### Running Integration Tests

The `integration-tests/` directory contains a comprehensive test suite:

```bash
# Build the test binaries
cd integration-tests
cargo build --release

# Run tests (requires template and finalizer binaries)
./target/release/test-runner \
  --template ../runfiles-stub/target/release/runfiles-stub \
  --finalizer ../finalize-stub/target/release/finalize-stub \
  --test-binaries ./target/release
```

## Architecture Details

### Platform Implementations

| Platform | Entry Point | API | Process Execution | Path Separator |
|----------|-------------|-----|-------------------|----------------|
| **Linux** | Custom `_start` | Raw syscalls (no libc) | `execve` syscall | `/` |
| **macOS** | Standard `main` | libSystem functions | `execve` function | `/` |
| **Windows** | Standard `main` | Win32 API (UTF-16) | `CreateProcessW` | `\` |

### Runfiles Path Handling

**Input** (embedded arguments): Always Unix-style forward slashes
```
my_workspace/bin/tool
data/input.txt
```

**Output** (after runfiles resolution): Platform-native
```
Linux/macOS:  /absolute/path/to/tool
Windows:      C:\absolute\path\to\tool
```

The Windows implementation automatically converts `/` to `\`.

### Binary Size Breakdown

Sizes vary by platform due to different linking requirements:

- **x86_64 Linux**: ~10KB (fully static, no libc)
- **aarch64 Linux**: ~67KB (static, larger due to alignment and number of instructions)
- **x86_64 macOS**: ~13KB (links libSystem)
- **aarch64 macOS**: ~49KB (links libSystem, ARM64)
- **x86_64 Windows**: ~22KB (links kernel32.dll)

## Use Cases for Bazel Rules

### 1. Tool Wrappers

Create consistent wrappers for tools that need runfiles.

```python
load("@hermetic_launcher//launcher:launcher_binary.bzl", "launcher_binary")

# Simple wrapper that invokes a tool with arguments
launcher_binary(
    name = "buildozer_version_command",
    entrypoint = "@buildozer",
    embedded_args = ["--version"],
)

# Wrapper with data dependencies and path resolution
launcher_binary(
    name = "hash_file",
    entrypoint = "@openssl",
    embedded_args = [
        "dgst",
        "-sha256",
        "$(rlocationpath :file_to_hash.txt)",  # Auto-detected for transformation
    ],
    data = [":file_to_hash.txt"],
)
```

**Key features:**
- **Automatic path transformation**: Arguments matching `$(rlocationpath ...)` are automatically resolved through runfiles
- **Runtime argument forwarding**: Additional arguments passed at runtime are appended
  ```bash
  bazel run //:hash_file -- --some-extra-flag
  # Executes: openssl dgst -sha256 /resolved/path/to/file.txt --some-extra-flag
  ```
- **Cross-platform**: The same BUILD file works on Linux, macOS, and Windows

### 2. Test Runners

Wrap test executables with their data dependencies:

```python
launcher_binary(
    name = "integration_test",
    entrypoint = "//test:runner",
    embedded_args = [
        "$(rlocationpath //test/data:fixtures.json)",
        "--verbose",
    ],
    data = ["//test/data:fixtures.json"],
)
```

### 3. Interpreted Language Binaries

Create native binaries for interpreted languages like Python, Ruby, or Node.js.
In those cases, you tend to have an interpreter binary that is not aware of runfiles and is generic for every binary of that language and a script that is executed on startup.
Neither are good to be used as the "executable" file of a Bazel "*_binary" rule, so hermetic-launcher can be used to invoke the interpreter with a path to the script entrypoint.

```python
# Python script wrapped as a native binary
launcher_binary(
    name = "my_python_tool",
    entrypoint = "@python_3_11//:python",
    embedded_args = [
        "$(rlocationpath //tools:script.py)",
    ],
    data = ["//tools:script.py"],
    visibility = ["//visibility:public"],
)
```

This creates a native executable that:
1. Resolves the Python interpreter through runfiles
2. Resolves the script path through runfiles
3. Executes: `python /resolved/path/to/script.py`
4. Works identically on Linux, macOS, and Windows

The launcher is tiny (10-68KB) regardless of the script size.

## Advanced Usage for Rule Authors

For rule authors who need more control than `launcher_binary` provides, the `launcher` struct in `@hermetic_launcher//launcher:lib.bzl` offers a lower-level API to construct custom launchers within your own rule implementations.

### When to Use the Launcher Struct

Use the `launcher` struct when you need to:
- Dynamically determine arguments based on rule context
- Integrate launcher generation into complex custom rules
- Build launchers with `cfg = "exec"` for tools that run during the build
- Have fine-grained control over which arguments are transformed

Use `launcher_binary` when you can express your launcher declaratively with static attributes.

### API Reference

```python
load("@hermetic_launcher//launcher:lib.bzl", "launcher")
```

The `launcher` struct provides these functions:

#### `launcher.args_from_entrypoint(executable_file)`

Initialize embedded args from an entrypoint executable file. Returns `(embedded_args, transformed_args)` with the entrypoint as the first argument.

```python
embedded_args, transformed_args = launcher.args_from_entrypoint(
    executable_file = ctx.executable.tool,
)
# Returns: (["_main/path/to/tool"], ["0"])
```

#### `launcher.append_runfile(file, embedded_args, transformed_args)`

Add a file that needs runfiles path resolution at runtime. Automatically marks it for transformation.

```python
embedded_args, transformed_args = launcher.append_runfile(
    file = ctx.file.config,
    embedded_args = embedded_args,
    transformed_args = transformed_args,
)
```

#### `launcher.append_embedded_arg(arg, embedded_args, transformed_args)`

Add a literal argument that will NOT be transformed through runfiles.

```python
embedded_args, transformed_args = launcher.append_embedded_arg(
    arg = "--verbose",
    embedded_args = embedded_args,
    transformed_args = transformed_args,
)
```

#### `launcher.append_raw_transformed_arg(arg, embedded_args, transformed_args)`

Add an argument that WILL be transformed through runfiles, but without converting a File object first.

```python
embedded_args, transformed_args = launcher.append_raw_transformed_arg(
    arg = "my_workspace/data/file.txt",
    embedded_args = embedded_args,
    transformed_args = transformed_args,
)
```

#### `launcher.compile_stub(ctx, embedded_args, transformed_args, output_file, cfg, template_exec_group)`

Compile the final launcher stub binary.

```python
launcher.compile_stub(
    ctx = ctx,
    embedded_args = embedded_args,
    transformed_args = transformed_args,
    output_file = exe,
    cfg = "target",  # or "exec" for build-time tools
    template_exec_group = None,  # or exec group name for exec cfg
)
```

**Parameters:**
- `cfg`: `"target"` (default) for binaries that run on the target platform, or `"exec"` for tools that run during the build
- `template_exec_group`: Name of the exec group when using `cfg = "exec"` (e.g., `"host"`)

#### `launcher.to_rlocation_path(file)`

Convert a File object to its rlocation path string.

```python
path = launcher.to_rlocation_path(ctx.file.data)
# Returns: "_main/path/to/data" or "external_repo/path/to/data"
```

### Required Toolchains

When using the `launcher` struct in your rules, declare the appropriate toolchains:

```python
# For target platform launchers (cfg = "target")
toolchains = [
    launcher.finalizer_toolchain_type,
    launcher.template_toolchain_type,
]

# For exec platform launchers (cfg = "exec")
toolchains = [
    launcher.finalizer_toolchain_type,
    launcher.template_exec_toolchain_type,
]
```

## FAQ

**Q: Why not just use shell scripts?**
A: Shell scripts aren't cross-platform. Bash doesn't work on Windows, batch files don't work on Unix. Native launchers work everywhere.

**Q: How is this different from other Bazel runfiles libraries?**
A: This creates standalone binaries, not library code. The launcher is your program's entry point.

**Q: Can I use this outside Bazel?**
A: Yes! As long as you set `RUNFILES_DIR`, `RUNFILES_MANIFEST_FILE`, or you create `<executable>.runfiles`, launchers work anywhere.

**Q: Why are the binaries different sizes?**
A: Platform differences. Linux can be fully static (smaller), macOS requires libSystem, Windows needs DLLs, ARM architectures need more instructions than x86 and have stricter alignment requirements.

**Q: Is the finalizer deterministic?**
A: Yes! The same inputs produce byte-identical outputs regardless of which platform you run the finalizer on. This is tested in CI.

## License

MIT License - See [LICENSE](LICENSE) file for details.

## Contributing

Contributions welcome! This project demonstrates:
- Cross-platform `no_std` Rust development
- Platform-specific system call interfaces
- Binary template patching techniques
- Bazel runfiles protocol implementation
