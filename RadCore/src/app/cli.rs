//! Command-line configuration selection shared by RadByte executables.
//!
//! Replacement authored in this chat on 2026-11-15 from the required interface
//! and standard-library API documentation. The prior file was accessible and
//! is archived under `NU/RadCore/src/app/cli.rs`; no clean-room status is claimed.
//! Sources: https://doc.rust-lang.org/std/env/fn.args_os.html and
//! https://doc.rust-lang.org/std/path/struct.PathBuf.html .

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

/// Prints the 6-line ANSI Shadow `RADBYTE` logo followed by the product and protocol versions.
pub fn print_startup_banner(product_name: &str, crate_version: &str, _ascii_art: &str) {
    enable_ansi_colors();
    if std::env::var_os("RADBYTE_ANTIGRAVITY_CLI").is_some_and(|v| v == "1")
        || crate::console_ui::is_interactive_tty()
    {
        return;
    }
    let mut out = String::with_capacity(1024);
    crate::console_ui::append_radbyte_logo(&mut out);
    if product_name == "RadProxy" {
        out.push_str(&format!(
            "  \x1b[1;37m{product_name}\x1b[0m \x1b[38;2;232;125;13mv{crate_version}\x1b[0m \x1b[90m|\x1b[0m Minecraft 1.7.2+ \x1b[90m(multi-version)\x1b[0m\n"
        ));
    } else {
        out.push_str(&format!(
            "  \x1b[1;37m{product_name}\x1b[0m \x1b[38;2;232;125;13mv{crate_version}\x1b[0m \x1b[90m|\x1b[0m Minecraft {} \x1b[90m(protocol {})\x1b[0m\n",
            crate::MINECRAFT_VERSION,
            crate::PROTOCOL_VERSION
        ));
    }
    println!("{out}");
}

/// Enables ANSI escape sequence rendering on Windows consoles.
pub fn enable_ansi_colors() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetStdHandle(std_handle: u32) -> isize;
            fn GetConsoleMode(console_handle: isize, mode: *mut u32) -> i32;
            fn SetConsoleMode(console_handle: isize, mode: u32) -> i32;
        }
        const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
        const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
        unsafe {
            let handle = GetStdHandle(STD_OUTPUT_HANDLE);
            if handle != 0 && handle != -1 {
                let mut mode = 0u32;
                if GetConsoleMode(handle, &raw mut mode) != 0 {
                    let _ = SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
                }
            }
        }
    }
}

/// Exits the process, pausing for Enter if running in a standalone double-clicked Windows console.
pub fn exit_with_pause(code: i32) -> ! {
    #[cfg(windows)]
    if code != 0 {
        use std::io::{IsTerminal, Read, Write};
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetConsoleProcessList(process_list: *mut u32, process_count: u32) -> u32;
        }
        let mut pids = [0u32; 2];
        let count = unsafe { GetConsoleProcessList(pids.as_mut_ptr(), 2) };
        if count == 1 && std::io::stdin().is_terminal() {
            print!("\nPress Enter to exit...");
            let _ = std::io::stdout().flush();
            let _ = std::io::stdin().read(&mut [0u8]);
        }
    }
    std::process::exit(code);
}

/// Selects the first configuration option from process arguments.
/// Separate option values retain the operating system's native path encoding.
#[must_use]
pub fn resolve_config_path(default_file: &str) -> PathBuf {
    choose_configuration(std::env::args_os().skip(1), default_file)
}

/// Selects `--config PATH`, `-c PATH`, or `--config=PATH` from supplied arguments.
/// Stops option processing at `--`; absent or empty values use `default_file`.
#[must_use]
pub fn resolve_config_path_from_args<I, S>(args: I, default_file: &str) -> PathBuf
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let native_arguments = args
        .into_iter()
        .map(|argument| OsString::from(argument.as_ref()));
    choose_configuration(native_arguments, default_file)
}

fn choose_configuration(mut arguments: impl Iterator<Item = OsString>, fallback: &str) -> PathBuf {
    while let Some(argument) = arguments.next() {
        if argument == OsStr::new("--") {
            break;
        }
        if argument == OsStr::new("--config") || argument == OsStr::new("-c") {
            return match arguments.next() {
                Some(path) if !path.is_empty() => PathBuf::from(path),
                _ => PathBuf::from(fallback),
            };
        }
        if let Some(path) = argument
            .to_str()
            .and_then(|value| value.strip_prefix("--config="))
        {
            return PathBuf::from(if path.is_empty() { fallback } else { path });
        }
    }
    PathBuf::from(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_each_supported_option_without_changing_paths() {
        for arguments in [
            vec!["--config", "server config café.toml"],
            vec!["-c", "server config café.toml"],
            vec!["--config=server config café.toml"],
        ] {
            assert_eq!(
                resolve_config_path_from_args(arguments, "default.toml"),
                PathBuf::from("server config café.toml")
            );
        }
    }

    #[test]
    fn falls_back_for_missing_and_empty_values() {
        for arguments in [
            vec![],
            vec!["--other"],
            vec!["--config"],
            vec!["-c", ""],
            vec!["--config="],
        ] {
            assert_eq!(
                resolve_config_path_from_args(arguments, "default.toml"),
                PathBuf::from("default.toml")
            );
        }
    }

    #[test]
    fn first_option_wins_and_terminator_stops_scanning() {
        assert_eq!(
            resolve_config_path_from_args(
                ["--unrelated", "-c", "first.toml", "--config=second.toml"],
                "default.toml"
            ),
            PathBuf::from("first.toml")
        );
        assert_eq!(
            resolve_config_path_from_args(["--", "--config", "ignored.toml"], "default.toml"),
            PathBuf::from("default.toml")
        );
    }

    #[cfg(unix)]
    #[test]
    fn separate_path_preserves_non_utf8_bytes() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(vec![b'c', 0xFF, b'g']);
        let arguments = vec![OsString::from("-c"), path.clone()];
        assert_eq!(
            choose_configuration(arguments.into_iter(), "default.toml"),
            PathBuf::from(path)
        );
    }
}
