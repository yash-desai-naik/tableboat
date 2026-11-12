//! Terminal bits: colours (off when piped, `--no-color` or NO_COLOR), questions and hidden
//! password input. Questions read the console itself (CONIN$), so they work even when the
//! query comes in on a pipe.

use std::io::{BufRead, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

static COLOR: AtomicBool = AtomicBool::new(false);

pub fn init(no_color: bool) {
    let want = !no_color && std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal();
    if want {
        enable_vt();
    }
    COLOR.store(want, Ordering::Relaxed);
}

/// Windows consoles need "virtual terminal processing" switched on for colour codes.
fn enable_vt() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Console::{
            ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetConsoleMode,
        };
        for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            if let Ok(h) = GetStdHandle(which) {
                let mut mode = Default::default();
                if GetConsoleMode(h, &mut mode).is_ok() {
                    let _ = SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
                }
            }
        }
    }
}

fn paint(code: &str, s: &str) -> String {
    if COLOR.load(Ordering::Relaxed) { format!("\x1b[{code}m{s}\x1b[0m") } else { s.to_string() }
}

pub fn dim(s: &str) -> String {
    paint("2", s)
}
pub fn red(s: &str) -> String {
    paint("31", s)
}
pub fn green(s: &str) -> String {
    paint("32", s)
}
pub fn clay(s: &str) -> String {
    paint("38;5;173", s)
}

/// Colour codes are only used for stderr text; stdout stays plain when it's a pipe.
pub fn stdout_is_terminal() -> bool {
    std::io::stdout().is_terminal()
}

/// The console for questions, even when stdin is a pipe. None when there is no console.
fn console() -> Option<std::fs::File> {
    std::fs::OpenOptions::new().read(true).write(true).open("CONIN$").ok()
}

/// A person can answer: there's a console and stderr (where questions go) is a terminal.
/// In a script, CI or a tool with a hidden console this is false, and `tb` refuses instead of
/// waiting forever for an answer. (stdin may be a pipe: answers are read from the console.)
pub fn has_console() -> bool {
    std::io::stderr().is_terminal() && console().is_some()
}

/// Ask a question on stderr; the answer is read from the console. None when nobody can answer.
pub fn ask(question: &str) -> Option<String> {
    if !has_console() {
        return None;
    }
    let con = console()?;
    eprint!("{question}");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::BufReader::new(con).read_line(&mut line).ok()?;
    Some(line.trim().to_string())
}

/// y/N question; anything but y/yes is no.
pub fn confirm(question: &str) -> bool {
    matches!(ask(&format!("{question} [y/N] ")).as_deref().map(str::to_ascii_lowercase).as_deref(), Some("y" | "yes"))
}

/// Read a password without showing it. None when nobody can type it.
pub fn password(prompt: &str) -> Option<String> {
    if !has_console() {
        return None;
    }
    let con = console()?;
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    #[cfg(windows)]
    let restore = unsafe {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::Console::{ENABLE_ECHO_INPUT, GetConsoleMode, SetConsoleMode};
        let h = HANDLE(con.as_raw_handle());
        let mut mode = Default::default();
        if GetConsoleMode(h, &mut mode).is_ok() {
            let _ = SetConsoleMode(h, mode & !ENABLE_ECHO_INPUT);
            Some((h, mode))
        } else {
            None
        }
    };
    let mut line = String::new();
    let read = std::io::BufReader::new(&con).read_line(&mut line);
    #[cfg(windows)]
    if let Some((h, mode)) = restore {
        unsafe {
            let _ = windows::Win32::System::Console::SetConsoleMode(h, mode);
        }
    }
    eprintln!();
    read.ok()?;
    Some(line.trim_end_matches(['\r', '\n']).to_string())
}
