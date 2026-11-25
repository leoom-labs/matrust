//! The one policy of the added-folder read fallback.
//!
//! A relative data-file name that is missing from the current folder may be
//! read from a folder of the user search path (`addpath`); `load`, `fopen`,
//! `exist`, `which` and the file readers share it. Which `fopen` mode, which
//! name and which found file qualify is decided here and nowhere else,
//! including the platform-specific path rules (`std::path::Component`); the
//! engine supplies only the folder list and the current-folder miss.
//!
//! Explicit local and absolute paths never pass through this module. It grants
//! no access: its only effect is to turn a would-be fallback hit into the
//! ordinary missing-file result of the current folder.
use std::path::{Component, Path, PathBuf};

pub(crate) fn reject_drive_relative(path: &Path) -> crate::Result<()> {
    if !path.has_root() && matches!(path.components().next(), Some(Component::Prefix(_))) {
        return Err("Drive-relative filenames are unsupported; use a drive-absolute or workspace-relative path".into());
    }
    Ok(())
}

/// A suffix can extend only a filename. Path::file_name normalizes a trailing
/// separator and `/.`, so inspect the user's last component before joining.
pub(crate) fn can_append_suffix(name: &str) -> bool {
    !matches!(
        name.rsplit(std::path::is_separator).next(),
        None | Some("" | "." | "..")
    )
}

/// Whether an `fopen` access mode (the permission without its `b`/`t` flag)
/// may fall back to an added folder: only the read-only `r`, which is also the
/// default. `r+` and every `w`/`a` form can write, so they resolve in the
/// current folder alone and a fallback never hands out a writable file.
pub fn mode_is_eligible(mode: &str) -> bool {
    mode == "r"
}

/// Whether `name` may be looked up in an added folder: relative, without a
/// drive or UNC prefix, a root or a `..` component, naming at least one file
/// or folder, and not asking for a folder by ending in a separator or in
/// `/.`. A leading or inner `.` is skipped, so `./x.txt` and `nested/x.txt`
/// search; `nested/../x.txt`, `x.txt/`, `x.txt/.`, `""` and `.` do not.
pub fn name_is_eligible(name: &Path) -> bool {
    // `components` drops a trailing separator and a trailing `.`, so those
    // are read from the text; the separators are ASCII in every encoding.
    let is_separator = |byte: &u8| std::path::is_separator(char::from(*byte));
    let asks_for_folder = match name.as_os_str().as_encoded_bytes() {
        [.., last] if is_separator(last) => true,
        [.., before, b'.'] => is_separator(before),
        _ => false,
    };
    let mut named = false;
    !asks_for_folder
        && name.components().all(|component| match component {
            Component::Normal(_) => {
                named = true;
                true
            }
            Component::CurDir => true,
            _ => false,
        })
        && named
}

/// Whether `..` in a name may step out of the folder it is joined to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParentDir {
    /// A local name: `..` removes the last folder, as the OS would.
    Lexical,
    /// An added-folder name: `..` refuses the whole name.
    Refuse,
}

/// `name` pushed onto `root` one component at a time: split on `/`, and on
/// `\` too when `backslash` is set; empty and `.` parts skipped; `..` handled
/// by `parent`. A name that ends in a separator or in `.` asks for a folder
/// and keeps a trailing separator, so `x.txt/` is never the file `x.txt`.
/// The result never relies on the OS to clean the name, which a Windows
/// `\\?\` (verbatim) root, as made by `canonicalize`, `addpath` and `cd`,
/// does not do.
///
/// `None` for a refused `..`, and for a part that would replace `root`
/// instead of extending it (a drive part such as `C:x.txt`; with `backslash`
/// set it is recognised on every platform): the result is always `root` with
/// components added or, for a lexical `..`, removed.
///
/// Pure path logic: no filesystem access. Callers in the engine use
/// [`native_join`]; this form takes the Windows rules as the `backslash` flag
/// so it can be tested on every platform.
pub fn join_components(
    root: &Path,
    name: &str,
    backslash: bool,
    parent: ParentDir,
) -> Option<PathBuf> {
    let mut path = root.to_path_buf();
    let mut asks_for_folder = false;
    for part in name.split(|c| c == '/' || (backslash && c == '\\')) {
        asks_for_folder = matches!(part, "" | ".");
        match part {
            "" | "." => {}
            ".." => match parent {
                // At the root `..` stays there, as the OS treats it.
                ParentDir::Lexical => {
                    path.pop();
                }
                ParentDir::Refuse => return None,
            },
            part if part_replaces_root(part, backslash) => return None,
            part => path.push(part),
        }
    }
    if asks_for_folder {
        // An empty push adds only the separator.
        path.push("");
    }
    Some(path)
}

/// Whether `PathBuf::push` of the single name part `part` would replace the
/// path it is pushed onto instead of extending it: the part has a root or a
/// drive or UNC prefix on this platform, or, under the Windows rules
/// (`windows`), starts with a drive (`C:x.txt`, `C:`), which names a file
/// relative to that drive's own current folder and not to the root being
/// built.
fn part_replaces_root(part: &str, windows: bool) -> bool {
    let path = Path::new(part);
    path.has_root()
        || matches!(path.components().next(), Some(Component::Prefix(_)))
        || (windows && matches!(part.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic()))
}

/// [`join_components`] with this platform's rules (`\` and drive parts only
/// on Windows). A name with a root or prefix, or one that is not UTF-8, is
/// joined as before; the policy never lets such a name search.
pub fn native_join(root: &Path, name: &Path, parent: ParentDir) -> Option<PathBuf> {
    let plain = !name.has_root() && !matches!(name.components().next(), Some(Component::Prefix(_)));
    match name.to_str() {
        Some(text) if plain => join_components(root, text, cfg!(windows), parent),
        _ => Some(root.join(name)),
    }
}

/// The current-folder path of a relative `name`. On Windows it is built
/// component by component, so `nested/x.txt`, `.\x.txt` and `../beside/l.txt`
/// work below a verbatim current folder (`..` is lexical there, as Win32
/// treats it); a name the builder refuses (a drive part such as `./C:x.txt`)
/// stays `base.join(name)`. Elsewhere it is `base.join(name)`, left to the OS
/// as before.
pub fn local_file(base: &Path, name: &Path) -> PathBuf {
    if cfg!(windows) {
        native_join(base, name, ParentDir::Lexical).unwrap_or_else(|| base.join(name))
    } else {
        base.join(name)
    }
}

/// The file `name` in the added folder `root`, built by [`native_join`] (so
/// `./x.txt` is `root/x.txt`). `None` is the ordinary miss, never an
/// error: the name is not eligible, nothing is there (a missing or unreadable
/// `root` included), it is not a regular file, or it resolves outside `root`
/// (a link that leaves the folder).
///
/// `root` is a folder as `addpath` stores it, absolute and canonical; a file
/// below a `root` that has since been replaced by a link is a miss as well.
pub fn contained_file(root: &Path, name: &Path) -> Option<PathBuf> {
    if !name_is_eligible(name) {
        return None;
    }
    let candidate = native_join(root, name, ParentDir::Refuse)?;
    if !candidate.is_file() {
        return None;
    }
    // One canonicalize per existing candidate, compared component-wise so a
    // sibling such as `added-other` never counts as inside `added`. The caller
    // opens `candidate` afterwards; that check-then-open window is accepted
    // for this desktop tool and is out of scope here.
    let resolved = candidate.canonicalize().ok()?;
    resolved.starts_with(root).then_some(candidate)
}
