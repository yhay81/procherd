# Support

## Where to ask

- Use [GitHub Discussions](https://github.com/yhay81/procherd/discussions) for
  installation, lifecycle, readiness, and integration questions.
- Use a structured [GitHub issue](https://github.com/yhay81/procherd/issues/new/choose)
  for reproducible bugs or scoped feature requests.
- Follow [SECURITY.md](SECURITY.md) for vulnerabilities.

ProcHerd is maintained by volunteers. Reports with a minimal synthetic child,
exact version, operating system, command argument vector, and redacted
structured output are the easiest to investigate.

Never post owner tokens, environment values, raw logs, private absolute paths,
or a complete state directory.

## Supported environment

The latest tagged pre-1.0 release supports:

- Linux x86-64;
- macOS x86-64 and Apple silicon;
- Windows x86-64;
- Rust 1.85 or newer when building from source;
- one-machine, same-user local processes.

Collect:

```bash
procherd --version
procherd --format json schema --document brief
```

Also report whether the child spawns descendants, daemonizes, handles
termination, binds a leased port, and emits high-volume or non-UTF-8 output.

## Scope

Support does not cover safely executing untrusted code, remote or distributed
jobs, container isolation, authenticated readiness, programs that deliberately
escape OS process ownership, guaranteed port transfer, log secrecy, or
automatic recovery after a machine or supervisor crash.
