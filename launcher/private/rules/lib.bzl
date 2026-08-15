_FINALIZER_TOOLCHAIN_TYPE = Label("//launcher:finalizer_toolchain_type")
_TEMPLATE_TOOLCHAIN_TYPE = Label("//launcher:template_toolchain_type")
_TEMPLATE_EXEC_TOOLCHAIN_TYPE = Label("//launcher:template_exec_toolchain_type")

def _get_finalizer(ctx):
    toolchain = ctx.toolchains[_FINALIZER_TOOLCHAIN_TYPE]
    return toolchain.finalizer_info.finalizer

def _get_template(ctx, *, cfg = "target", template_exec_group = None, template_file = None):
    if template_file != None:
        return template_file
    toolchain_dict = ctx.toolchains if template_exec_group == None else ctx.exec_groups[template_exec_group].toolchains
    if cfg == "target":
        toolchain = toolchain_dict[_TEMPLATE_TOOLCHAIN_TYPE]
    elif cfg == "exec":
        toolchain = toolchain_dict[_TEMPLATE_EXEC_TOOLCHAIN_TYPE]
    else:
        fail("Invalid cfg '%s': must be 'target' or 'exec'" % cfg)
    return toolchain.templatetoolchaininfo.template_exe

def _to_rlocation_path(f):
    if f.short_path.startswith("../"):
        return f.short_path[3:]
    return "_main/" + f.short_path

def _args_from_entrypoint(executable_file):
    embedded_args = [_to_rlocation_path(executable_file)]
    transformed_args = [0]
    return embedded_args, transformed_args

def _append_runfile(*, file, embedded_args, transformed_args):
    new_arg = _to_rlocation_path(file)
    transformed_args.append(str(len(embedded_args)))
    embedded_args.append(new_arg)
    return embedded_args, transformed_args

def _append_embedded_arg(*, arg, embedded_args, transformed_args):
    embedded_args.append(arg)
    return embedded_args, transformed_args

def _append_raw_transformed_arg(*, arg, embedded_args, transformed_args):
    transformed_args.append(str(len(embedded_args)))
    embedded_args.append(arg)
    return embedded_args, transformed_args

def _compile_stub(*, ctx, embedded_args, transformed_args, output_file, cfg = "target", template_exec_group = None, template_file = None):
    template = _get_template(ctx, cfg = cfg, template_exec_group = template_exec_group, template_file = template_file)
    args = ctx.actions.args()
    args.add("--template", template)
    args.add("-o", output_file)
    args.add_joined("--transform", transformed_args, join_with = ",")
    args.add("--")
    args.add_all(embedded_args)
    ctx.actions.run(
        outputs = [output_file],
        executable = _get_finalizer(ctx),
        arguments = [args],
        inputs = [template],
        toolchain = _FINALIZER_TOOLCHAIN_TYPE,
    )
    return output_file

def _runfiles(*, self, files):
    for file in files:
        _append_runfile(
            file = file,
            embedded_args = self._embedded_args(),
            transformed_args = self._transformed_args(),
        )
    return self

def _embedded_args(*, self, args):
    for arg in args:
        _append_embedded_arg(
            arg = arg,
            embedded_args = self._embedded_args(),
            transformed_args = self._transformed_args(),
        )
    return self

def _raw_transformed_args(*, self, args):
    for arg in args:
        _append_raw_transformed_arg(
            arg = arg,
            embedded_args = self._embedded_args(),
            transformed_args = self._transformed_args(),
        )
    return self

# buildifier: disable=uninitialized
def _entrypoint(executable_file, *, transformed_args = None):
    mutable_embedded_args = [_to_rlocation_path(executable_file)]
    mutable_transformed_args = [0]
    self = struct(
        _embedded_args = lambda: mutable_embedded_args,
        _transformed_args = lambda: mutable_transformed_args,
        embedded_args = lambda *args: _embedded_args(
            self = self,
            args = args,
        ),
        raw_transformed_args = lambda *args: _raw_transformed_args(
            self = self,
            args = args,
        ),
        runfiles = lambda *files: _runfiles(
            self = self,
            files = files,
        ),
        compile = lambda ctx, **kwargs: _compile_stub(
            ctx = ctx,
            embedded_args = mutable_embedded_args,
            transformed_args = mutable_transformed_args,
            **kwargs
        ),
    )
    return self

launcher = struct(
    to_rlocation_path = _to_rlocation_path,
    entrypoint = _entrypoint,
    args_from_entrypoint = _args_from_entrypoint,
    append_runfile = _append_runfile,
    append_embedded_arg = _append_embedded_arg,
    append_raw_transformed_arg = _append_raw_transformed_arg,
    compile_stub = _compile_stub,
    finalizer_toolchain_type = _FINALIZER_TOOLCHAIN_TYPE,
    template_toolchain_type = _TEMPLATE_TOOLCHAIN_TYPE,
    template_exec_toolchain_type = _TEMPLATE_EXEC_TOOLCHAIN_TYPE,
)
