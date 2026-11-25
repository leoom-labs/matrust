use matrust::{Engine, Matrix, files};
use std::{collections::BTreeMap, fs};

fn run(engine: &mut Engine, code: &str) {
    engine
        .execute(code)
        .unwrap_or_else(|error| panic!("{code}: {error}"));
}

fn workspace() -> (Engine, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("sub")).unwrap();
    let mut engine = Engine::default();
    let folder = dir.path().to_string_lossy().replace('\'', "''");
    run(&mut engine, &format!("cd('{folder}');"));
    (engine, dir)
}

#[test]
fn path_file_lookup_load_and_rmpath() {
    let (mut engine, dir) = workspace();
    files::save_workspace(
        &dir.path().join("sub/data.mat"),
        &BTreeMap::from([("marker".into(), Matrix::scalar(7.))]),
    )
    .unwrap();
    fs::write(dir.path().join("sub/values.txt"), "8 9\n").unwrap();

    let missing = engine.execute("load data.mat").unwrap_err();
    run(
        &mut engine,
        "addpath sub
         load data.mat
         assert(marker == 7); clear marker;
         load('data');
         assert(marker == 7); clear marker;
         result = load('data');
         assert(result.marker == 7 && exist('marker', 'var') == 0);
         load values.txt -ascii
         assert(isequal(values, [8 9]));
         assert(isequal(load('values.txt', '-ascii'), [8 9]));
         rmpath sub",
    );
    assert_eq!(engine.execute("load data.mat").unwrap_err(), missing);
    run(
        &mut engine,
        "assert(exist('data.mat', 'file') == 0 && isempty(which('data.mat')));",
    );
}

#[test]
fn path_file_lookup_order_and_open_modes() {
    let (mut engine, dir) = workspace();
    let root = dir.path();
    fs::create_dir(root.join("last")).unwrap();
    fs::write(root.join("last/t.csv"), "2\n").unwrap();
    fs::write(root.join("sub/t.csv"), "1\n").unwrap();
    fs::write(root.join("sub/x.txt"), "first\n").unwrap();
    run(
        &mut engine,
        "addpath('sub', 'last');
         assert(readmatrix('t.csv') == 1);
         assert(exist('t.csv', 'file') == 2);
         assert(strcmp(which('t.csv'), fullfile(pwd, 'sub', 't.csv')));
         assert(readmatrix(fullfile(pwd, 'last', 't.csv')) == 2);
         fid = fopen('x.txt'); assert(fid > 0); fclose(fid);
         fid = fopen('new.txt', 'w'); assert(fid > 0); fclose(fid);
         addpath('sub', '-end');
         assert(readmatrix('t.csv') == 2);",
    );
    let printed = engine.execute("which t.csv").unwrap().output.join("\n");
    let full_path = root.join("last/t.csv").canonicalize().unwrap();
    let full_path = full_path.to_string_lossy();
    assert_eq!(printed.trim(), full_path.trim_start_matches(r"\\?\"));
    assert!(root.join("new.txt").is_file());
    assert!(!root.join("sub/new.txt").exists());

    // Read-only opens (access mode `r` once the b/t flag is removed, which is
    // also the default) are found in the added folder.
    run(
        &mut engine,
        "fid = fopen('x.txt');
         assert(fid > 0 && strcmp(fgetl(fid), 'first')); fclose(fid);",
    );
    for mode in ["r", "rb", "rt", "br", "tr"] {
        run(
            &mut engine,
            &format!(
                "fid = fopen('x.txt', '{mode}');
                 assert(fid > 0 && strcmp(fgetl(fid), 'first')); fclose(fid);"
            ),
        );
    }
    // Review decision 2026-11-21 (docs/steps/corpus-rounds.md 7b): update modes
    // never search the added folders. The miss is the ordinary missing-file
    // result and the file in the added folder is left as it was.
    for mode in ["r+", "rt+", "rb+", "r+b", "r+t", "br+", "tr+"] {
        run(
            &mut engine,
            &format!(
                "[fid, message] = fopen('x.txt', '{mode}');
                 [nowhere, expected] = fopen('nowhere.txt', '{mode}');
                 assert(fid == -1 && nowhere == -1);
                 assert(~isempty(message) && strcmp(message, expected));"
            ),
        );
        assert!(!root.join("x.txt").exists());
        assert_eq!(fs::read(root.join("sub/x.txt")).unwrap(), b"first\n");
    }
    for mode in ["w", "a", "w+", "a+", "wb", "at+"] {
        run(
            &mut engine,
            &format!("fid = fopen('x.txt', '{mode}'); assert(fid > 0); fclose(fid);"),
        );
        assert_eq!(fs::read(root.join("x.txt")).unwrap(), b"");
        assert_eq!(fs::read(root.join("sub/x.txt")).unwrap(), b"first\n");
        fs::remove_file(root.join("x.txt")).unwrap();
    }

    // An update open of a file in the current folder works as before.
    fs::write(root.join("x.txt"), "local\n").unwrap();
    for mode in ["r+", "rt+"] {
        run(
            &mut engine,
            &format!(
                "fid = fopen('x.txt', '{mode}');
                 assert(fid > 0 && strcmp(fgetl(fid), 'local'));
                 fprintf(fid, 'more'); fclose(fid);"
            ),
        );
        assert_eq!(fs::read(root.join("x.txt")).unwrap(), b"local\nmore");
        assert_eq!(fs::read(root.join("sub/x.txt")).unwrap(), b"first\n");
        fs::write(root.join("x.txt"), "local\n").unwrap();
    }
    fs::remove_file(root.join("x.txt")).unwrap();

    fs::write(root.join("t.csv"), "3\n").unwrap();
    run(
        &mut engine,
        "assert(readmatrix('t.csv') == 3);
         assert(strcmp(which('t.csv'), fullfile(pwd, 't.csv')));",
    );
}

#[test]
fn path_file_lookup_reader_routes() {
    let (mut engine, dir) = workspace();
    let sub = dir.path().join("sub");
    fs::write(sub.join("t.csv"), "1,2\n3,4\n").unwrap();
    fs::create_dir(sub.join("nested")).unwrap();
    fs::write(sub.join("nested/x.txt"), "alpha\nbeta\n").unwrap();
    fs::write(
        sub.join("book.xlsx"),
        include_bytes!("fixtures/readers/book.xlsx"),
    )
    .unwrap();
    run(
        &mut engine,
        r#"addpath sub
         expected = [1 2; 3 4];
         assert(isequal(readmatrix('t.csv'), expected));
         assert(isequal(csvread('t.csv'), expected));
         assert(isequal(dlmread('t.csv', ','), expected));
         assert(isequal(importdata('t.csv'), expected));
         assert(isequal(readcell('t.csv'), {1 2; 3 4}));
         t = readtable('t.csv', 'ReadVariableNames', false);
         assert(isequal(t{:, :}, expected));
         opts = detectImportOptions('t.csv');
         assert(isequal(readmatrix('t.csv', opts), expected));
         assert(isequal(readmatrix('book.xlsx', 'Sheet', 'Offset', 'Range', 'B4:C4'), [1 2]));
         filename = fullfile('nested', 'x.txt');
         assert(strcmp(fileread(filename), sprintf('alpha\nbeta\n')));
         assert(isequal(readlines(filename), ["alpha"; "beta"]));
         fid = fopen('t.csv');
         scanned = textscan(fid, '%f %f', 'Delimiter', ','); fclose(fid);
         assert(isequal([scanned{1} scanned{2}], expected));
         img = uint8(expected);
         imwrite(img, fullfile('sub', 'image.png'));
         assert(isequal(imread('image.png'), img));
         assert(isequal(imread('image', 'png'), img));
         audiowrite(fullfile('sub', 'tone.wav'), [0; 0], 8000);
         [samples, rate] = audioread('tone.wav');
         assert(isequal(samples, [0; 0]) && rate == 8000);"#,
    );
}

#[test]
fn path_file_lookup_policy_modes_and_names() {
    use matrust::files::read_fallback::{mode_is_eligible, name_is_eligible};
    use std::path::Path;

    // fopen passes its access mode with the b/t flag already removed.
    assert!(mode_is_eligible("r"));
    for mode in ["r+", "w", "w+", "a", "a+", ""] {
        assert!(!mode_is_eligible(mode), "{mode}");
    }

    // Plain and nested file names search an added folder; `.` is skipped.
    for name in [
        "x.txt",
        "data.mat",
        "no_extension",
        ".hidden",
        "a..b.txt",
        "nested/x.txt",
        "a/b/c.csv",
        "nested/./x.txt",
        "./x.txt",
        "./nested/x.txt",
        "x.",
        "nested/...",
    ] {
        assert!(name_is_eligible(Path::new(name)), "{name}");
    }
    // A root or any `..` keeps the name relative to the current folder only,
    // and a name must name something. A name that ends in a separator or in
    // `/.` asks for a folder, so it is not the file of that name either.
    for name in [
        "x.txt/",
        "x.txt/.",
        "x.txt//",
        "x.txt/./",
        "./x.txt/",
        "nested/",
        "nested/x.txt/",
        "nested/x.txt/.",
        "",
        ".",
        "./.",
        "..",
        "../x.txt",
        "nested/../x.txt",
        "nested/../../x.txt",
        "./../x.txt",
        "/",
        "/x.txt",
    ] {
        assert!(!name_is_eligible(Path::new(name)), "{name}");
    }
    // Backslashes and drive letters are separators and prefixes on Windows
    // and ordinary file-name characters elsewhere.
    for name in [
        r"..\x.txt",
        r"nested\..\x.txt",
        r"\x.txt",
        r"C:\x.txt",
        "c:x.txt",
        r"x.txt\",
        r"x.txt\.",
        r"nested\x.txt\",
    ] {
        assert_eq!(name_is_eligible(Path::new(name)), !cfg!(windows), "{name}");
    }
}

#[test]
fn path_file_lookup_policy_component_builder() {
    use matrust::files::read_fallback::{ParentDir, join_components};
    use std::path::{Path, PathBuf};

    // Pure path logic, no filesystem: a Windows verbatim root as text (on
    // Linux one odd component, which is enough to show nothing relies on the
    // OS to clean the name) and a plain absolute root.
    let push = |root: &Path, parts: &[&str]| -> PathBuf {
        let mut path = root.to_path_buf();
        for part in parts {
            path.push(part);
        }
        path
    };
    for root in [
        Path::new(r"\\?\C:\root"),
        Path::new(r"\\?\C:\work\root"),
        Path::new("/work/root"),
    ] {
        for parent in [ParentDir::Lexical, ParentDir::Refuse] {
            for backslash in [true, false] {
                let join = |name| join_components(root, name, backslash, parent);
                assert_eq!(join("x.txt"), Some(push(root, &["x.txt"])));
                assert_eq!(join("nested/x.txt"), Some(push(root, &["nested", "x.txt"])));
                assert_eq!(join("./x.txt"), Some(push(root, &["x.txt"])));
                assert_eq!(
                    join("nested//./x.txt"),
                    Some(push(root, &["nested", "x.txt"]))
                );
                assert_eq!(join("."), Some(root.to_path_buf()));
            }
            // A backslash separates only when asked (Windows).
            let join = |name| join_components(root, name, true, parent);
            assert_eq!(join(r".\x.txt"), Some(push(root, &["x.txt"])));
            assert_eq!(
                join(r"nested\x.txt"),
                Some(push(root, &["nested", "x.txt"]))
            );
            assert_eq!(
                join(r".\nested/x.txt"),
                Some(push(root, &["nested", "x.txt"]))
            );
            assert_eq!(
                join_components(root, r".\x.txt", false, parent),
                Some(push(root, &[r".\x.txt"]))
            );

            // A name that ends in a separator or in `.` asks for a folder and
            // keeps one trailing separator, so `x.txt/` is never the file
            // `x.txt`. Compared as text: `PathBuf` equality ignores it.
            let text = |name, backslash| {
                let path = join_components(root, name, backslash, parent).unwrap();
                path.into_os_string().into_string().unwrap()
            };
            let file = text("x.txt", false);
            assert!(!file.ends_with(std::path::is_separator), "{file}");
            let folder = format!("{file}{}", std::path::MAIN_SEPARATOR);
            for name in ["x.txt/", "x.txt/.", "x.txt//", "x.txt/./", "./x.txt/"] {
                assert_eq!(text(name, false), folder, "{name}");
                assert_eq!(text(name, true), folder, "{name}");
            }
            for name in [r"x.txt\", r"x.txt\.", r".\x.txt\", r"x.txt/\"] {
                assert_eq!(text(name, true), folder, "{name}");
            }
            assert_eq!(text("nested/x.txt", true), text(r"nested\x.txt", true));
            assert!(!text("nested/x.txt", true).ends_with(std::path::is_separator));

            // A part that starts with a drive would replace the root when
            // pushed (`C:x.txt` is relative to that drive's own current
            // folder), so under the Windows rules it refuses the whole name,
            // local or added: the result never leaves the root that way.
            for name in [
                "./C:x.txt",
                "nested/C:x.txt",
                "./C:/a",
                "./c:",
                "C:x.txt",
                r".\C:x.txt",
                r"nested\C:\a",
            ] {
                assert_eq!(join_components(root, name, true, parent), None, "{name}");
            }
            // Without them a drive is a prefix only where the platform says so.
            assert_eq!(
                join_components(root, "./C:x.txt", false, parent),
                (!cfg!(windows)).then(|| push(root, &["C:x.txt"]))
            );
            assert_eq!(
                join_components(root, "nested/C:x.txt", false, parent),
                (!cfg!(windows)).then(|| push(root, &["nested", "C:x.txt"]))
            );
            // A colon elsewhere is not a drive: the part stays below the root.
            assert_eq!(
                join_components(root, "nested/x.txt:s", true, parent),
                Some(push(root, &["nested", "x.txt:s"]))
            );
        }
        // `..`: a local name steps up lexically, an added-folder name is refused.
        let mut up = root.to_path_buf();
        up.pop();
        for (name, backslash) in [
            ("../beside/l.txt", false),
            ("../beside/l.txt", true),
            (r"..\beside\l.txt", true),
            ("nested/../../beside/l.txt", false),
        ] {
            assert_eq!(
                join_components(root, name, backslash, ParentDir::Lexical),
                Some(push(&up, &["beside", "l.txt"])),
                "{name}"
            );
            assert_eq!(
                join_components(root, name, backslash, ParentDir::Refuse),
                None,
                "{name}"
            );
        }
        assert_eq!(
            join_components(root, "nested/../x.txt", false, ParentDir::Lexical),
            Some(push(root, &["x.txt"]))
        );
        assert_eq!(
            join_components(root, "nested/../x.txt", false, ParentDir::Refuse),
            None
        );
    }
    // `..` at the top of the root stays at the top, as the OS treats it.
    assert_eq!(
        join_components(Path::new("/"), "../x.txt", false, ParentDir::Lexical),
        Some(PathBuf::from("/x.txt"))
    );
}

#[test]
fn path_file_lookup_policy_containment() {
    use matrust::files::read_fallback::{ParentDir, contained_file, native_join};
    use std::path::Path;

    // addpath stores canonical folders, so the policy is given one.
    // `base` may be a Windows verbatim path, where the OS does not clean a
    // joined name: every fixture path is built by the policy's own builder.
    let at =
        |root: &Path, name: &str| native_join(root, Path::new(name), ParentDir::Lexical).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let added = at(&base, "added");
    fs::create_dir_all(at(&added, "nested")).unwrap();
    fs::create_dir(at(&base, "outside")).unwrap();
    fs::create_dir(at(&base, "added-other")).unwrap();
    fs::write(at(&added, "x.txt"), "x").unwrap();
    fs::write(at(&added, "nested/x.txt"), "nested").unwrap();
    fs::write(at(&base, "outside/s.txt"), "s").unwrap();
    fs::write(at(&base, "added-other/o.txt"), "o").unwrap();

    // A hit is the name pushed component by component onto the folder.
    for name in [
        "x.txt",
        "nested/x.txt",
        "./x.txt",
        "./nested/x.txt",
        "nested/./x.txt",
    ] {
        let hit = contained_file(&added, Path::new(name));
        assert!(hit.is_some(), "{name}");
        assert_eq!(
            hit,
            native_join(&added, Path::new(name), ParentDir::Refuse),
            "{name}"
        );
    }
    assert_eq!(
        contained_file(&added, Path::new("./nested/x.txt")),
        Some(at(&added, "nested/x.txt"))
    );
    #[cfg(windows)]
    for name in [r".\x.txt", r"nested\x.txt", r".\nested/x.txt"] {
        assert!(contained_file(&added, Path::new(name)).is_some(), "{name}");
    }
    // The ordinary miss: nothing there, a folder, or a name with `..`,
    // whether or not it would come back into the added folder.
    for name in [
        "absent.txt",
        "nested/absent.txt",
        "nested",
        "",
        ".",
        "x.txt/",
        "x.txt/.",
        "nested/x.txt/",
        "./C:x.txt",
        "nested/../x.txt",
        "../added/x.txt",
        "../outside/s.txt",
        "../added-other/o.txt",
    ] {
        assert_eq!(contained_file(&added, Path::new(name)), None, "{name}");
    }
    assert_eq!(contained_file(&added, &at(&added, "x.txt")), None);
    // An absent folder is skipped, not an error.
    assert_eq!(contained_file(&at(&base, "gone"), Path::new("x.txt")), None);

    // A link is followed only while its target stays inside the added folder;
    // `added-other` shares a text prefix with `added` but is not inside it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(at(&added, "x.txt"), at(&added, "inside.txt")).unwrap();
        symlink(at(&base, "outside/s.txt"), at(&added, "link.txt")).unwrap();
        symlink(at(&base, "outside"), at(&added, "away")).unwrap();
        symlink(at(&base, "added-other/o.txt"), at(&added, "other.txt")).unwrap();
        assert_eq!(
            contained_file(&added, Path::new("inside.txt")),
            Some(at(&added, "inside.txt"))
        );
        for name in ["link.txt", "away/s.txt", "other.txt"] {
            assert_eq!(contained_file(&added, Path::new(name)), None, "{name}");
        }
        // A folder that is itself a link is not the folder addpath stored.
        symlink(&added, at(&base, "alias")).unwrap();
        assert_eq!(
            contained_file(&at(&base, "alias"), Path::new("x.txt")),
            None
        );
    }
}

#[test]
fn path_file_lookup_folder_names_stay_local() {
    // 'x.txt/' and 'x.txt/.' ask for a folder. They are never the file x.txt
    // of an added folder: with or without a file of that name in the current
    // folder, every probe gives exactly what it gives without addpath.
    const ADDED: &str = "from the added folder";
    let (mut engine, dir) = workspace();
    let root = dir.path();
    fs::create_dir(root.join("sub").join("nested")).unwrap();
    fs::write(root.join("sub").join("x.txt"), ADDED).unwrap();
    fs::write(root.join("sub").join("nested").join("x.txt"), ADDED).unwrap();
    // vec!: Windows extends it below (cfg(windows)); clippy on Linux sees no growth.
    #[allow(unused_mut, clippy::useless_vec)]
    let mut names = vec![
        "x.txt/",
        "x.txt/.",
        "x.txt//",
        "./x.txt/",
        "nested/x.txt/",
        "nested/x.txt/.",
    ];
    #[cfg(windows)]
    names.extend([r"x.txt\", r"x.txt\.", r"nested\x.txt\"]);
    let probes: Vec<String> = names
        .iter()
        .flat_map(|name| {
            [
                format!("disp(fileread('{name}'))"),
                format!("disp(readlines('{name}'))"),
                format!("disp(exist('{name}', 'file'))"),
                format!("disp(exist('{name}'))"),
                format!("disp(which('{name}'))"),
                format!("fid = fopen('{name}'); fclose('all'); disp(fid > 0)"),
            ]
        })
        .collect();
    let snapshot = |engine: &mut Engine| -> Vec<Result<Vec<String>, String>> {
        let results: Vec<_> = probes
            .iter()
            .map(|probe| engine.execute(probe).map(|done| done.output))
            .collect();
        for (probe, result) in probes.iter().zip(&results) {
            let lines = result.as_ref().map_or(&[][..], |lines| &lines[..]);
            assert!(!lines.iter().any(|line| line.contains(ADDED)), "{probe}");
        }
        results
    };

    // Only the added folder has x.txt: the plain name finds it, a folder
    // name does not.
    let without = snapshot(&mut engine);
    run(
        &mut engine,
        &format!(
            "addpath sub
             assert(strcmp(fileread('x.txt'), '{ADDED}') && exist('x.txt', 'file') == 2);
             assert(strcmp(fileread('nested/x.txt'), '{ADDED}'));"
        ),
    );
    assert_eq!(snapshot(&mut engine), without);
    run(
        &mut engine,
        "assert(exist('x.txt/', 'file') == 0 && exist('x.txt/.', 'file') == 0);
         assert(fopen('x.txt/') == -1 && isempty(which('x.txt/')));",
    );
    assert!(engine.execute("fileread('x.txt/')").is_err());
    assert!(engine.execute("fileread('x.txt/.')").is_err());

    // A file x.txt in the current folder as well: the folder name is still
    // answered by the current folder alone (there it is not a folder).
    run(&mut engine, "rmpath sub");
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("x.txt"), "local").unwrap();
    fs::write(root.join("nested").join("x.txt"), "local").unwrap();
    let with_local = snapshot(&mut engine);
    run(
        &mut engine,
        "addpath sub
         assert(strcmp(fileread('x.txt'), 'local'));",
    );
    assert_eq!(snapshot(&mut engine), with_local);
    #[cfg(unix)]
    {
        run(
            &mut engine,
            "assert(exist('x.txt/', 'file') == 0 && exist('x.txt/.', 'file') == 0);
             assert(fopen('x.txt/') == -1 && fopen('nested/x.txt/') == -1);",
        );
        for probe in ["fileread('x.txt/')", "fileread('x.txt/.')"] {
            assert!(engine.execute(probe).is_err(), "{probe}");
        }
    }
}

#[test]
fn path_file_lookup_stays_inside_added_folders() {
    // One temporary tree. The current folder is here/deep, tree/added is put
    // on the path and tree/outside is its sibling. From here/deep the name
    // '../outside/s.txt' is here/outside/s.txt, which does not exist, so only
    // the added folder could supply it.
    use matrust::files::read_fallback::{ParentDir, native_join};
    use std::path::Path;
    // Fixture paths come from the policy's builder, so they hold on a Windows
    // verbatim base as well.
    let at =
        |root: &Path, name: &str| native_join(root, Path::new(name), ParentDir::Lexical).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let (cwd, added, outside) = (
        at(base, "here/deep"),
        at(base, "tree/added"),
        at(base, "tree/outside"),
    );
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir(at(base, "here/beside")).unwrap();
    fs::create_dir_all(at(&added, "nested")).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(at(&added, "x.txt"), "1,2\n").unwrap();
    fs::write(at(&added, "nested/x.txt"), "5,6\n").unwrap();
    fs::write(at(&outside, "s.txt"), "3,4\n").unwrap();
    fs::write(at(base, "here/beside/l.txt"), "7,8\n").unwrap();
    // The builder resolves `..` lexically for a local name.
    assert_eq!(at(&cwd, "../beside/l.txt"), at(base, "here/beside/l.txt"));
    files::save_workspace(
        &at(&outside, "s.mat"),
        &BTreeMap::from([("marker".into(), Matrix::scalar(7.))]),
    )
    .unwrap();
    let quoted = |path: &Path| path.to_string_lossy().replace('\'', "''");
    let mut engine = Engine::default();
    run(
        &mut engine,
        &format!(
            "cd('{}'); outside = '{}';
             imwrite(uint8([1 2; 3 4]), fullfile(outside, 's.png'));
             audiowrite(fullfile(outside, 's.wav'), [0; 0], 8000);",
            quoted(&cwd),
            quoted(&outside)
        ),
    );

    // load, exist, which, fopen and the readers share one lookup: a name that
    // leaves the added folder gets the same missing-file result as it does
    // without addpath.
    #[allow(unused_mut)]
    let mut probes = vec![
        "load('../outside/s.mat')",
        "load ../outside/s.mat",
        "load('../outside/s.txt', '-ascii')",
        "fileread('../outside/s.txt')",
        "readlines('../outside/s.txt')",
        "readmatrix('../outside/s.txt')",
        "readcell('../outside/s.txt')",
        "readtable('../outside/s.txt')",
        "detectImportOptions('../outside/s.txt')",
        "importdata('../outside/s.txt')",
        "csvread('../outside/s.txt')",
        "dlmread('../outside/s.txt', ',')",
        "imread('../outside/s.png')",
        "imread('../outside/s', 'png')",
        "audioread('../outside/s.wav')",
        "fileread('nested/../x.txt')",
        "readmatrix('nested/../x.txt')",
    ];
    // A link inside the added folder that points out of it is a miss as well.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        symlink(at(&outside, "s.txt"), at(&added, "link.txt")).unwrap();
        symlink(&outside, at(&added, "away")).unwrap();
        symlink(at(&added, "x.txt"), at(&added, "inside.txt")).unwrap();
        probes.extend([
            "fileread('link.txt')",
            "readmatrix('link.txt')",
            "load('link.txt', '-ascii')",
            "fileread('away/s.txt')",
            "load('away/s.mat')",
        ]);
    }
    let missing: Vec<String> = probes
        .iter()
        .map(|probe| engine.execute(probe).unwrap_err())
        .collect();
    run(
        &mut engine,
        &format!(
            "[fid, before] = fopen('../outside/s.txt'); assert(fid == -1);
             addpath('{}');",
            quoted(&added)
        ),
    );
    for (probe, expected) in probes.iter().zip(&missing) {
        assert_eq!(&engine.execute(probe).unwrap_err(), expected, "{probe}");
    }
    run(
        &mut engine,
        "name = '../outside/s.txt';
         assert(exist(name, 'file') == 0 && exist(name) == 0 && isempty(which(name)));
         [fid, message] = fopen(name); assert(fid == -1 && strcmp(message, before));
         assert(exist('nested/../x.txt', 'file') == 0 && fopen('nested/../x.txt') == -1);
         assert(exist('marker', 'var') == 0);",
    );
    #[cfg(unix)]
    run(
        &mut engine,
        &format!(
            "assert(exist('link.txt', 'file') == 0 && fopen('link.txt') == -1);
             assert(exist('away/s.txt', 'file') == 0 && isempty(which('link.txt')));
             assert(isequal(readmatrix('inside.txt'), [1 2]));
             assert(strcmp(which('inside.txt'), fullfile('{}', 'inside.txt')));",
            quoted(&added.canonicalize().unwrap())
        ),
    );
    // Unchanged: plain, nested and `./` names are found in the added folder,
    // and a path that exists relative to the current folder is that file.
    run(
        &mut engine,
        "assert(exist('x.txt', 'file') == 2 && isequal(readmatrix('x.txt'), [1 2]));
         assert(exist('nested/x.txt', 'file') == 2 && ~isempty(which('nested/x.txt')));
         assert(isequal(readmatrix('nested/x.txt'), [5 6]));
         fid = fopen('nested/x.txt'); assert(fid > 0 && strcmp(fgetl(fid), '5,6')); fclose(fid);
         assert(exist('./x.txt', 'file') == 2 && isequal(readmatrix('./x.txt'), [1 2]));
         assert(exist('../beside/l.txt', 'file') == 2);
         assert(isequal(readmatrix('../beside/l.txt'), [7 8]));
         assert(isequal(readmatrix(fullfile(outside, 's.txt')), [3 4]));
         load(fullfile(outside, 's.mat')); assert(marker == 7);",
    );
    assert_eq!(fs::read(at(&outside, "s.txt")).unwrap(), b"3,4\n");
}
