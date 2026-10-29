//! Presentation only: never use this text to reopen, compare or persist a path.
use std::{
    fmt::{self, Write},
    path::Path,
};

const MAX_BYTES: usize = 32_768;
const ELLIPSIS: &str = "...";

pub(crate) struct DisplayPath<'a>(&'a Path);

pub(crate) fn display(path: &Path) -> DisplayPath<'_> {
    DisplayPath(path)
}

// Recognize only the two ordinary filesystem namespaces. In particular, keep
// device, volume, relative and malformed prefixes verbatim. Do not scan an
// unbounded synthetic path looking for a UNC separator.
fn readable_parts(text: &str, windows: bool) -> (&str, &str) {
    if !windows || text.len() > MAX_BYTES {
        return ("", text);
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        let bytes = rest.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\'
        {
            return ("", rest);
        }
        if let Some(network) = rest.strip_prefix(r"UNC\") {
            let mut parts = network.splitn(3, '\\');
            let server = parts.next().unwrap_or("");
            let share = parts.next().unwrap_or("");
            if !matches!(server, "" | "." | "..")
                && !matches!(share, "" | "." | "..")
                && !server.contains(['/', ':'])
                && !share.contains(['/', ':'])
            {
                return (r"\\", network);
            }
        }
    }
    ("", text)
}

struct Bounded<'a, 'b> {
    formatter: &'a mut fmt::Formatter<'b>,
    remaining: usize,
    truncated: bool,
}
impl Write for Bounded<'_, '_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if text.len() <= self.remaining {
            self.formatter.write_str(text)?;
            self.remaining -= text.len();
            return Ok(());
        }
        let mut end = self.remaining;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.formatter.write_str(&text[..end])?;
        self.formatter.write_str(ELLIPSIS)?;
        self.truncated = true;
        // Stop the standard Path formatter too, rather than constructing its
        // entire lossy String. This sentinel is consumed only by our formatter.
        Err(fmt::Error)
    }
}
impl fmt::Display for DisplayPath<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut output = Bounded {
            formatter,
            remaining: MAX_BYTES - ELLIPSIS.len(),
            truncated: false,
        };
        let result = if let Some(text) = self.0.to_str() {
            let (prefix, rest) = readable_parts(text, cfg!(windows));
            output
                .write_str(prefix)
                .and_then(|()| output.write_str(rest))
        } else {
            // Invalid Unicode retains the standard display replacement policy
            // and its namespace; it is never treated as a different I/O path.
            write!(&mut output, "{}", self.0.display())
        };
        if output.truncated { Ok(()) } else { result }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_rooted_drive_and_complete_unc_namespaces_are_readable() {
        for (source, expected) in [
            (r"\\?\C:\folder\plot.mfig", r"C:\folder\plot.mfig"),
            (r"\\?\z:\", r"z:\"),
            (
                r"\\?\UNC\server\share\plot.mfig",
                r"\\server\share\plot.mfig",
            ),
            (r"\\?\UNC\server\share", r"\\server\share"),
            (
                "\\\\?\\C:\\lambda-\u{03bb}\\plot.mfig",
                "C:\\lambda-\u{03bb}\\plot.mfig",
            ),
        ] {
            let (prefix, rest) = readable_parts(source, true);
            assert_eq!(format!("{prefix}{rest}"), expected);
            assert_eq!(readable_parts(source, false), ("", source));
        }
        for source in [
            r"\\.\COM1",
            r"\\?\Volume{123}\file",
            r"\??\C:\file",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1",
            r"\\?\C:relative",
            r"\\?\1:\file",
            r"\\?\UNC\server",
            r"\\?\UNC\\share",
            r"\\?\UNC\server\",
            r"\\?\UNC\..\share",
            r"prefix\\?\C:\file",
            r"\\?\\\?\C:\file",
            r"C:\ordinary",
            r"\\server\share",
        ] {
            assert_eq!(readable_parts(source, true), ("", source));
        }
    }
    #[test]
    fn display_is_bounded_on_unicode_boundaries_and_does_not_mutate_paths() {
        let path = std::path::PathBuf::from(format!("C:\\{}", "\u{03bb}".repeat(MAX_BYTES)));
        let original = path.clone();
        let text = display(&path).to_string();
        assert!(text.len() <= MAX_BYTES && text.ends_with(ELLIPSIS));
        assert_eq!(path, original);
        assert_eq!(
            display(Path::new("ordinary.txt")).to_string(),
            "ordinary.txt"
        );
    }
    #[cfg(windows)]
    #[test]
    fn nonunicode_path_keeps_namespace_and_standard_lossy_display() {
        use std::os::windows::ffi::OsStringExt;
        let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&[
            92, 92, 63, 92, 67, 58, 92, 0xd800, 46, 109,
        ]));
        assert_eq!(display(&path).to_string(), path.display().to_string());
    }
}
