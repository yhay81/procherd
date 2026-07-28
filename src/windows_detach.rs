//! The narrow Win32 FFI boundary used to detach a supervisor without
//! inheriting the foreground caller's standard-I/O pipe handles.

use std::{ffi::OsStr, io, mem::size_of, os::windows::ffi::OsStrExt, path::Path, ptr::null};

use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::Threading::{
        CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CreateProcessW, DETACHED_PROCESS,
        PROCESS_INFORMATION, STARTUPINFOW,
    },
};

pub(crate) fn spawn_supervisor(executable: &Path, run_dir: &Path) -> io::Result<()> {
    let mut application_name = nul_terminated(executable.as_os_str())?;
    let mut command_line = build_command_line(executable.as_os_str(), run_dir.as_os_str())?;
    let startup_info = STARTUPINFOW {
        cb: u32::try_from(size_of::<STARTUPINFOW>()).expect("STARTUPINFOW size always fits in u32"),
        ..STARTUPINFOW::default()
    };
    let mut process_info = PROCESS_INFORMATION::default();

    // SAFETY:
    // - both UTF-16 buffers are NUL-terminated and remain alive for the call;
    // - CreateProcessW may mutate only `command_line`, which is uniquely owned;
    // - optional security, environment, and current-directory pointers are null;
    // - both output handles are closed exactly once after a successful call.
    //
    // `bInheritHandles` is deliberately FALSE. Rust's normal Windows Command
    // spawning enables inheritance for stdio setup, which can keep an invoking
    // process's capture pipes open for the lifetime of this detached supervisor.
    let created = unsafe {
        CreateProcessW(
            application_name.as_mut_ptr(),
            command_line.as_mut_ptr(),
            null(),
            null(),
            0,
            CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW | DETACHED_PROCESS,
            null(),
            null(),
            &startup_info,
            &mut process_info,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: CreateProcessW returned success, so these are owned, valid handles.
    // Closing them does not stop the detached process or its primary thread.
    unsafe {
        let _ = CloseHandle(process_info.hThread);
        let _ = CloseHandle(process_info.hProcess);
    }
    Ok(())
}

fn build_command_line(executable: &OsStr, run_dir: &OsStr) -> io::Result<Vec<u16>> {
    let mut command_line = Vec::new();
    append_quoted(&mut command_line, &wide(executable)?);
    command_line.push(u16::from(b' '));
    command_line.extend("__supervise".encode_utf16());
    command_line.push(u16::from(b' '));
    command_line.extend("--run-dir".encode_utf16());
    command_line.push(u16::from(b' '));
    append_quoted(&mut command_line, &wide(run_dir)?);
    command_line.push(0);
    Ok(command_line)
}

fn nul_terminated(value: &OsStr) -> io::Result<Vec<u16>> {
    let mut units = wide(value)?;
    units.push(0);
    Ok(units)
}

fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
    let units: Vec<_> = value.encode_wide().collect();
    if units.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows process argument contains a NUL code unit",
        ));
    }
    Ok(units)
}

/// Apply the quoting rules used by `CommandLineToArgvW`.
fn append_quoted(output: &mut Vec<u16>, argument: &[u16]) {
    const BACKSLASH: u16 = b'\\' as u16;
    const QUOTE: u16 = b'"' as u16;
    const SPACE: u16 = b' ' as u16;
    const TAB: u16 = b'\t' as u16;

    let needs_quotes = argument.is_empty()
        || argument
            .iter()
            .any(|unit| matches!(*unit, SPACE | TAB | QUOTE));
    if !needs_quotes {
        output.extend(argument);
        return;
    }

    output.push(QUOTE);
    let mut backslashes = 0;
    for &unit in argument {
        if unit == BACKSLASH {
            backslashes += 1;
        } else if unit == QUOTE {
            output.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2 + 1));
            output.push(QUOTE);
            backslashes = 0;
        } else {
            output.extend(std::iter::repeat_n(BACKSLASH, backslashes));
            output.push(unit);
            backslashes = 0;
        }
    }
    output.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2));
    output.push(QUOTE);
}

#[cfg(test)]
mod tests {
    use super::append_quoted;

    fn quote(value: &str) -> String {
        let mut output = Vec::new();
        let argument: Vec<_> = value.encode_utf16().collect();
        append_quoted(&mut output, &argument);
        String::from_utf16(&output).unwrap()
    }

    #[test]
    fn quotes_windows_arguments_without_changing_their_value() {
        assert_eq!(quote("plain"), "plain");
        assert_eq!(quote(""), r#""""#);
        assert_eq!(quote("two words"), r#""two words""#);
        assert_eq!(quote(r#"a"b"#), r#""a\"b""#);
        assert_eq!(
            quote(r#"C:\directory with space\"#),
            r#""C:\directory with space\\"#
        );
    }
}
