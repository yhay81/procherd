# Platform support

## Release targets

| Platform | Release archive | Process-tree control | Graceful stop |
| --- | --- | --- | --- |
| Linux x86-64 | `linux-x86_64` | process group | `SIGTERM`, then `SIGKILL` |
| macOS x86-64 | `macos-x86_64` | process group | `SIGTERM`, then `SIGKILL` |
| macOS Apple silicon | `macos-aarch64` | process group | `SIGTERM`, then `SIGKILL` |
| Windows x86-64 | `windows-x86_64` | Job Object | forced Job termination |

CI executes the complete test suite and release build on Linux, macOS, and
Windows. Rust 1.85 is the minimum supported source-build toolchain.

## Guarantees and exceptions

Unix cleanup addresses the child's process group. A child that creates a new
session/process group, double-forks, or otherwise deliberately daemonizes can
escape. ProcHerd does not scan the global process table to guess ancestry.

Windows children are assigned to a Job Object configured to terminate members
when ownership closes. Processes with sufficient privileges and explicit
breakaway behavior may escape operating-system containment.

No platform receives CPU, memory, filesystem, network, or process-count
isolation. ProcHerd is a lifecycle owner, not a sandbox.

## State permissions

On Unix, ProcHerd creates new state directories as `0700` and files as `0600`.
Existing parent permissions and privileged readers remain relevant.

On Windows, ProcHerd relies on the ACL inherited from `%LOCALAPPDATA%` or the
explicit state directory. Users selecting a shared directory must set a
restrictive ACL themselves.

## Filesystem and text

Version 0.1 accepts UTF-8 program names and arguments. Child output may contain
arbitrary bytes and is stored as base64 records. File readiness requires a
regular file; state documents and run directories reject symbolic links on
read.

## Crash behavior

Normal CLI disconnects do not affect the detached supervisor. Unexpected
supervisor failure triggers best-effort cleanup while its ownership guards are
alive. A machine crash is different: durable state can be stale, and version
0.1 does not reattach to surviving processes after reboot or supervisor loss.
