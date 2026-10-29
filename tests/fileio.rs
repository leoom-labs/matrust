//! File I/O (fopen/fread/fscanf/textscan/dlmread/...) reference behavior.
use matrust::{Array, Engine};

struct Sandbox {
    dir: std::path::PathBuf,
    e: Engine,
}
impl Sandbox {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("matrust-fileio-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut e = Engine::default();
        e.set_file_directory(&dir).unwrap();
        Sandbox { dir, e }
    }
    fn run(&mut self, code: &str) {
        self.e
            .execute(code)
            .unwrap_or_else(|err| panic!("{code}: {err}"));
    }
    fn err(&mut self, code: &str) -> String {
        match self.e.execute(code) {
            Ok(_) => panic!("{code}: expected an error"),
            Err(e) => {
                let text = e.to_string();
                match text.split_once(": ") {
                    Some((prefix, rest)) if prefix.starts_with("Line ") => rest.to_owned(),
                    _ => text,
                }
            }
        }
    }
    fn nums(&self, name: &str) -> Vec<f64> {
        match self.e.array(name).unwrap() {
            Array::Numeric(m) => m.data().to_vec(),
            Array::Logical(m) => m.data().iter().map(|&v| f64::from(v)).collect(),
            other => panic!("{name}: {other:?}"),
        }
    }
    fn dims(&self, name: &str) -> (usize, usize) {
        match self.e.array(name).unwrap() {
            Array::Numeric(m) => (m.rows(), m.cols()),
            Array::Character(c) => (c.rows(), c.cols()),
            other => panic!("{name}: {other:?}"),
        }
    }
    fn text(&self, name: &str) -> String {
        match self.e.array(name).unwrap() {
            Array::Character(c) => String::from_utf16(c.units()).unwrap(),
            other => panic!("{name}: {other:?}"),
        }
    }
    fn file(&self, name: &str) -> Vec<u8> {
        std::fs::read(self.dir.join(name)).unwrap()
    }
    fn put(&self, name: &str, bytes: &[u8]) {
        std::fs::write(self.dir.join(name), bytes).unwrap();
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn fid_numbering_and_failure() {
    let mut s = Sandbox::new("fid");
    s.run("a = fopen('x.txt','w'); b = fopen('y.txt','w'); fclose(a); c = fopen('z.txt','w');");
    assert_eq!(s.nums("a"), [3.]);
    assert_eq!(s.nums("b"), [4.]);
    assert_eq!(s.nums("c"), [3.], "lowest free fid is reused");
    s.run("[f, m] = fopen('missing.txt'); g = fopen('missing.txt','r');");
    assert_eq!(s.nums("f"), [-1.]);
    assert_eq!(s.text("m"), "No such file or directory");
    assert_eq!(s.nums("g"), [-1.]);
    s.run("all = fopen('all'); n = fclose('all'); none = fopen('all');");
    assert_eq!(s.nums("all"), [3., 4.]);
    assert_eq!(s.nums("n"), [0.]);
    assert_eq!(s.dims("none"), (0, 0));
}

#[test]
fn invalid_fid_messages() {
    let mut s = Sandbox::new("bad");
    let bad = "Invalid file identifier.  Use fopen to generate a valid file identifier.";
    assert_eq!(s.err("fclose(99)"), bad);
    assert_eq!(s.err("fgetl(7)"), bad);
    assert_eq!(s.err("fprintf(7, 'a')"), bad);
    assert_eq!(s.err("fopen()"), "Not enough input arguments.");
    assert_eq!(s.err("fgetl()"), "Not enough input arguments.");
    s.run("f = fopen('a.txt','w'); fclose(f);");
    assert_eq!(s.err("fclose(f)"), bad);
}

#[test]
fn at_most_64_open_files() {
    let mut s = Sandbox::new("limit");
    s.run(
        "for k = 1:64, f(k) = fopen('a.txt','w'); end; [g, m] = fopen('a.txt','w'); fclose('all');",
    );
    assert_eq!(s.nums("g"), [-1.]);
    assert_eq!(s.text("m"), "Too many open files");
    assert_eq!(*s.nums("f").last().unwrap(), 66.);
}

#[test]
fn write_then_read_lines() {
    let mut s = Sandbox::new("lines");
    s.run(
        "f = fopen('t.txt','w'); n = fprintf(f,'hello\\nworld\\n\\nlast'); fclose(f);
         f = fopen('t.txt'); a = fgetl(f); p = ftell(f); b = fgets(f); c = fgetl(f); e1 = feof(f);
         d = fgetl(f); e2 = feof(f); z = fgetl(f); w = fgets(f); fclose(f);",
    );
    assert_eq!(s.nums("n"), [17.]);
    assert_eq!(s.text("a"), "hello");
    assert_eq!(s.nums("p"), [6.]);
    assert_eq!(s.text("b"), "world\n");
    assert_eq!(s.dims("c"), (0, 0));
    assert_eq!(s.nums("e1"), [0.]);
    assert_eq!(s.text("d"), "last");
    assert_eq!(s.nums("e2"), [1.]);
    assert_eq!(s.nums("z"), [-1.]);
    assert_eq!(s.nums("w"), [-1.]);
    assert_eq!(s.file("t.txt"), b"hello\nworld\n\nlast");
}

#[test]
fn crlf_lines_and_terminator_output() {
    let mut s = Sandbox::new("crlf");
    s.put("w.txt", b"ab\r\ncd\n");
    s.run("f = fopen('w.txt'); a = fgetl(f); [b, t] = fgets(f); fclose(f);");
    assert_eq!(s.text("a"), "ab");
    assert_eq!(s.text("b"), "cd\n");
    assert_eq!(s.text("t"), "\n");
}

#[test]
fn seek_tell_rewind() {
    let mut s = Sandbox::new("seek");
    s.put("t.txt", b"0123456789");
    s.run(
        "f = fopen('t.txt'); a = fseek(f, 2, 'bof'); p1 = ftell(f); c1 = fread(f, 1, '*char');
         b = fseek(f, -3, 'eof'); p2 = ftell(f); c2 = fread(f, 1, '*char');
         d = fseek(f, 1, 'cof'); p3 = ftell(f);
         bad1 = fseek(f, -100, 'bof'); bad2 = fseek(f, 100, 'bof'); p4 = ftell(f);
         o = fseek(f, 0, -1); frewind(f); p5 = ftell(f); fclose(f);",
    );
    assert_eq!(s.nums("a"), [0.]);
    assert_eq!(s.nums("p1"), [2.]);
    assert_eq!(s.text("c1"), "2");
    assert_eq!(s.nums("b"), [0.]);
    assert_eq!(s.nums("p2"), [7.]);
    assert_eq!(s.text("c2"), "7");
    assert_eq!(s.nums("d"), [0.]);
    assert_eq!(s.nums("p3"), [9.]);
    assert_eq!(s.nums("bad1"), [-1.]);
    assert_eq!(s.nums("bad2"), [-1.]);
    assert_eq!(s.nums("p4"), [9.]);
    assert_eq!(s.nums("p5"), [0.]);
}

#[test]
fn fread_sizes_and_precisions() {
    let mut s = Sandbox::new("fread");
    s.put("b.bin", &[1, 2, 3, 4, 5, 6, 7]);
    s.run(
        "f = fopen('b.bin'); a = fread(f, 3); [b, n] = fread(f, [2 2]); c = fread(f); e = feof(f);
         frewind(f); d = fread(f, [3 Inf]); frewind(f); g = fread(f, 3, 'uint8=>char'); frewind(f);
         h = fread(f, [1 4], '*char'); frewind(f); k = fread(f, Inf, 'uint16'); fclose(f);",
    );
    assert_eq!(s.nums("a"), [1., 2., 3.]);
    assert_eq!(s.dims("a"), (3, 1));
    assert_eq!(s.nums("b"), [4., 5., 6., 7.]);
    assert_eq!(s.dims("b"), (2, 2));
    assert_eq!(s.nums("n"), [4.]);
    assert_eq!(s.dims("c"), (0, 0));
    assert_eq!(s.nums("e"), [1.]);
    assert_eq!(s.dims("d"), (3, 3));
    assert_eq!(s.nums("d"), [1., 2., 3., 4., 5., 6., 7., 0., 0.]);
    assert_eq!(s.dims("g"), (3, 1));
    assert_eq!(s.dims("h"), (1, 4));
    assert_eq!(s.nums("k"), [513., 1027., 1541.]);
}

#[test]
fn fwrite_saturates_and_roundtrips() {
    let mut s = Sandbox::new("fwrite");
    s.run(
        "f = fopen('b.bin','w'); n = fwrite(f, [72 105 300 -5 2.7]); m = fwrite(f, 'ab');
         k = fwrite(f, [1.5 2.5], 'double'); j = fwrite(f, 258, 'uint16', 0, 'ieee-be'); fclose(f);
         f = fopen('b.bin'); a = fread(f, 5); b = fread(f, 2, '*char'); c = fread(f, 2, 'double');
         d = fread(f, 1, 'uint16', 0, 'b'); fclose(f);",
    );
    assert_eq!(s.nums("n"), [5.]);
    assert_eq!(s.nums("m"), [2.]);
    assert_eq!(s.nums("k"), [2.]);
    assert_eq!(s.nums("j"), [1.]);
    assert_eq!(s.nums("a"), [72., 105., 255., 0., 3.]);
    assert_eq!(s.text("b"), "ab");
    assert_eq!(s.dims("b"), (2, 1));
    assert_eq!(s.nums("c"), [1.5, 2.5]);
    assert_eq!(s.nums("d"), [258.]);
}

#[test]
fn fprintf_to_file_and_screen() {
    let mut s = Sandbox::new("fprintf");
    s.run("f = fopen('p.txt','wt'); fprintf(f,'%5.2f|%s|%c|%d\\n', 3.14159, 'xy', 'z', 7); fclose(f);");
    assert_eq!(s.file("p.txt"), b" 3.14|xy|z|7\r\n");
    let out =
        s.e.execute("fprintf(1,'to stdout\\n'); fprintf('plain\\n'); fprintf(2,'err\\n');")
            .unwrap();
    assert_eq!(out.output.join("\n"), "to stdout\nplain\nerr");
}

#[test]
fn append_and_update_modes() {
    let mut s = Sandbox::new("modes");
    s.put("t.txt", b"abc");
    s.run(
        "f = fopen('t.txt','a'); fprintf(f,'X'); fclose(f);
         f = fopen('t.txt','r+'); fseek(f,0,'eof'); p = ftell(f); fprintf(f,'Y'); frewind(f);
         r = fread(f, Inf, '*char'); fclose(f);
         f = fopen('t.txt','r'); w = fwrite(f, 1); fclose(f);",
    );
    assert_eq!(s.nums("p"), [4.]);
    assert_eq!(s.text("r"), "abcXY");
    assert_eq!(s.nums("w"), [0.]);
    assert_eq!(s.file("t.txt"), b"abcXY");
}

#[test]
fn fscanf_numbers_and_sizes() {
    let mut s = Sandbox::new("fscanf");
    s.put("n.txt", b"1 2 3\n4 5 6\n");
    s.run(
        "f = fopen('n.txt'); a = fscanf(f, '%d'); e = feof(f); frewind(f);
         b = fscanf(f, '%d', [3 Inf]); frewind(f); c = fscanf(f, '%d', 2); d = fscanf(f, '%d', 2);
         frewind(f); [g, n] = fscanf(f, '%f'); fclose(f);",
    );
    assert_eq!(s.nums("a"), [1., 2., 3., 4., 5., 6.]);
    assert_eq!(s.dims("a"), (6, 1));
    assert_eq!(s.nums("e"), [1.]);
    assert_eq!(s.dims("b"), (3, 2));
    assert_eq!(s.nums("b"), [1., 2., 3., 4., 5., 6.]);
    assert_eq!(s.nums("c"), [1., 2.]);
    assert_eq!(s.nums("d"), [3., 4.]);
    assert_eq!(s.nums("n"), [6.]);
}

#[test]
fn fscanf_literals_floats_and_text() {
    let mut s = Sandbox::new("fscanf2");
    s.put("m.txt", b"x=1.5,x=-2e1,x=Inf,x=NaN");
    s.run("f = fopen('m.txt'); a = fscanf(f, 'x=%f,'); fclose(f);");
    let a = s.nums("a");
    assert_eq!(&a[..3], [1.5, -20., f64::INFINITY]);
    assert!(a[3].is_nan());
    s.put("w.txt", b"alpha beta\n");
    s.run(
        "f = fopen('w.txt'); a = fscanf(f, '%s'); frewind(f); b = fscanf(f, '%s %d'); fclose(f);
         f = fopen('w.txt'); c = fscanf(f, '%c', 5); fclose(f);",
    );
    assert_eq!(s.text("a"), "alphabeta");
    assert_eq!(s.text("c"), "alpha");
    assert_eq!(
        s.nums("b").len(),
        5,
        "%s then failing %d keeps the characters read"
    );
    s.put("d.txt", b"3.5 7");
    s.run("f = fopen('d.txt'); a = fscanf(f, '%d'); fclose(f);");
    assert_eq!(s.nums("a"), [3.], "%d stops at the decimal point");
}

#[test]
fn textscan_from_file_and_string() {
    let mut s = Sandbox::new("ts");
    s.put("c.csv", b"1,a,2.5\n2,bb,3.5\n3,ccc,\n");
    s.run(
        "f = fopen('c.csv'); C = textscan(f, '%f %s %f', 'Delimiter', ','); fclose(f);
         x = C{1}; y = C{3}; n = numel(C); z = C{2}; k = z{3};",
    );
    assert_eq!(s.nums("x"), [1., 2., 3.]);
    let y = s.nums("y");
    assert_eq!(&y[..2], [2.5, 3.5]);
    assert!(y[2].is_nan());
    assert_eq!(s.nums("n"), [3.]);
    assert_eq!(s.text("k"), "ccc");
    s.run("D = textscan('1 2 3 4 5 6', '%f %f'); a = D{1}; b = D{2};");
    assert_eq!(s.nums("a"), [1., 3., 5.]);
    assert_eq!(s.nums("b"), [2., 4., 6.]);
    s.run("D = textscan(sprintf('h1\\nh2\\n7 8'), '%f %f', 'HeaderLines', 2); a = D{1}; b = D{2};");
    assert_eq!(s.nums("a"), [7.]);
    s.run("D = textscan('1 2 3 4', '%f', 2); a = D{1};");
    assert_eq!(s.nums("a"), [1., 2.]);
    s.run("D = textscan('1,2,3,4', '%f %f', 'Delimiter', ',', 'CollectOutput', 1); a = D{1}; ok = numel(D);");
    assert_eq!(s.nums("a"), [1., 3., 2., 4.]);
    assert_eq!(s.dims("a"), (2, 2));
}

#[test]
fn dlmread_forms() {
    let mut s = Sandbox::new("dlm");
    s.put("a.csv", b"1,2,3\n4,5,6\n7,8,9\n");
    s.run(
        "a = dlmread('a.csv'); b = csvread('a.csv', 1, 1); c = dlmread('a.csv', ',', [0 1 1 2]);
         d = csvread('a.csv', 2, 0); e = dlmread('a.csv', ',', 1, 0);",
    );
    assert_eq!(s.dims("a"), (3, 3));
    assert_eq!(s.nums("a"), [1., 4., 7., 2., 5., 8., 3., 6., 9.]);
    assert_eq!(s.nums("b"), [5., 8., 6., 9.]);
    assert_eq!(s.nums("c"), [2., 5., 3., 6.]);
    assert_eq!(s.nums("d"), [7., 8., 9.]);
    assert_eq!(s.dims("e"), (2, 3));
    s.put("ragged.txt", b"1 2 3\n4\t\n");
    s.put("tab.txt", b"1\t2\n3\t4\n");
    s.put("semi.txt", b"1;;3\n");
    s.run("r = dlmread('ragged.txt'); t = dlmread('tab.txt'); m = dlmread('semi.txt');");
    assert_eq!(s.nums("r"), [1., 4., 2., 0., 3., 0.]);
    assert_eq!(s.nums("t"), [1., 3., 2., 4.]);
    assert_eq!(s.nums("m"), [1., 0., 3.]);
    // Reference behavior (o2:14, o2:17).
    assert_eq!(
        s.err("dlmread('a.csv', ',', 5, 5)"),
        "Empty format string is not supported at the end of a file."
    );
    assert_eq!(
        s.err("dlmread('nofile.csv')"),
        "The file 'nofile.csv' could not be opened because: No such file or directory"
    );
}

#[test]
fn dlmwrite_and_csvwrite() {
    let mut s = Sandbox::new("dlmw");
    s.run(
        "dlmwrite('a.txt', [1 2.5; 3 pi], 'newline', 'unix');
         dlmwrite('b.txt', [1 2; 3 4], ';', 'newline', 'unix');
         dlmwrite('c.txt', pi, 'precision', 3, 'newline', 'unix');
         dlmwrite('d.txt', [pi 1], 'precision', '%8.3f', 'delimiter', '\\t', 'newline', 'unix');
         csvwrite('e.csv', [1 2; 3 4], 1, 1);
         dlmwrite('f.txt', [1 NaN Inf], 'newline', 'unix');
         dlmwrite('g.txt', [1 2], 'newline', 'unix'); dlmwrite('g.txt', [3 4], '-append', 'newline', 'unix');",
    );
    assert_eq!(s.file("a.txt"), b"1,2.5\n3,3.1416\n");
    assert_eq!(s.file("b.txt"), b"1;2\n3;4\n");
    assert_eq!(s.file("c.txt"), b"3.14\n");
    assert_eq!(s.file("d.txt"), b"   3.142\t   1.000\n");
    let e = String::from_utf8(s.file("e.csv"))
        .unwrap()
        .replace("\r\n", "\n");
    assert_eq!(e, ",,\n,1,2\n,3,4\n");
    assert_eq!(s.file("f.txt"), b"1,NaN,Inf\n");
    assert_eq!(s.file("g.txt"), b"1,2\n3,4\n");
    s.run("m = csvread('e.csv');");
    assert_eq!(s.dims("m"), (3, 3));
}

#[test]
fn type_prints_the_file() {
    let mut s = Sandbox::new("type");
    s.put("t.txt", b"first\nsecond\n");
    s.put("fun.m", b"x = 1;\n");
    // The reference prints a blank line first (o1:59, o2:18).
    let out = s.e.execute("type('t.txt')").unwrap();
    assert_eq!(out.output.join("\n"), "\nfirst\nsecond");
    let out = s.e.execute("type('fun')").unwrap();
    assert_eq!(out.output.join("\n"), "\nx = 1;");
    assert!(s.err("type('nothing.txt')").contains("nothing.txt"));
}

#[test]
fn exist_variable_file_dir() {
    let mut s = Sandbox::new("exist");
    s.put("t.txt", b"x");
    std::fs::create_dir(s.dir.join("sub")).unwrap();
    s.run(
        "v = 1; a = exist('v'); b = exist('t.txt'); c = exist('t.txt', 'file'); d = exist('sub');
         e = exist('sub', 'dir'); f = exist('nothing'); g = exist('v', 'var'); h = exist('t.txt', 'var');
         k = exist('sub', 'file'); m = exist('t.txt', 'dir');",
    );
    let got: Vec<f64> = "abcdefghkm"
        .chars()
        .map(|c| s.nums(&c.to_string())[0])
        .collect();
    assert_eq!(got, [1., 2., 2., 7., 7., 0., 1., 0., 7., 0.]);
}

#[test]
fn paths_are_checked() {
    let mut s = Sandbox::new("paths");
    assert!(s.err("fopen('//server/share/x.txt')").contains("network"));
    assert!(s.err("fopen('http://x/y')").contains("network"));
    assert!(s.err("fopen('a.txt','z')").contains("permission"));
    s.run("f = fopen('sub/none.txt','w');");
    assert_eq!(s.nums("f"), [-1.]);
}

#[cfg(unix)]
#[test]
fn special_files_are_refused() {
    let mut s = Sandbox::new("special");
    s.run("[f, m] = fopen('/dev/zero'); [g, n] = fopen('/dev/null', 'w');");
    assert_eq!(s.nums("f"), [-1.]);
    assert_eq!(s.text("m"), "Not a regular file");
    assert_eq!(s.nums("g"), [-1.]);
    assert!(s.err("dlmread('/dev/zero')").contains("/dev/zero"));
    assert!(s.err("type('/dev/zero')").contains("/dev/zero"));
    assert!(
        s.err("dlmwrite('/dev/null', 1)")
            .contains("Not a regular file")
    );
}

#[test]
fn huge_write_offsets_are_refused_before_building() {
    let mut s = Sandbox::new("bigoff");
    let start = std::time::Instant::now();
    assert!(
        s.err("csvwrite('x.csv', 1, 0, 1000000000)")
            .contains("8 MiB")
    );
    assert!(
        s.err("csvwrite('x.csv', 1, 1000000000, 0)")
            .contains("8 MiB")
    );
    assert!(
        s.err("dlmwrite('x.csv', [1 2], 'roffset', 5000000, 'coffset', 3)")
            .contains("8 MiB")
    );
    assert!(start.elapsed().as_secs_f64() < 1.0, "refused at once");
    assert!(!s.dir.join("x.csv").exists(), "nothing is written");
}

#[test]
fn rejected_dlmwrite_keeps_the_file() {
    let mut s = Sandbox::new("keep");
    s.put("existing.csv", b"7,8\n");
    assert!(
        !s.err("dlmwrite('existing.csv', 1, 'precision', '%z')")
            .is_empty()
    );
    // Reference o2:37 (same message; the reference has no 'mac' either).
    for nl in ["dos", "mac"] {
        assert_eq!(
            s.err(&format!("dlmwrite('existing.csv', 1, 'newline', '{nl}')")),
            "Newline must be either 'pc' or 'unix'."
        );
    }
    assert!(
        !s.err("csvwrite('existing.csv', 1, 0, 1000000000)")
            .is_empty()
    );
    assert_eq!(s.file("existing.csv"), b"7,8\n");
}

#[test]
fn zero_width_scan_reads_nothing() {
    let mut s = Sandbox::new("zero");
    s.put("a.txt", b"abc 12\n");
    // Reference behavior (o2:31, o2:32): an empty result, no error.
    s.run("f = fopen('a.txt'); a = fscanf(f, '%*0s'); b = fscanf(f, '%0d'); c = fscanf(f, '%0c');");
    assert_eq!(s.dims("a"), (0, 0));
    assert_eq!(s.dims("b"), (0, 0));
    assert_eq!(s.dims("c"), (0, 0));
    s.run("x = fscanf(f, '%2c', 2); fclose(f);");
    assert_eq!(s.text("x"), "ab");
}

#[test]
fn fopen_machine_format_is_honoured() {
    let mut s = Sandbox::new("arch");
    s.run(
        "f = fopen('b.bin', 'w', 'ieee-be'); fwrite(f, 258, 'uint16'); fwrite(f, 258, 'uint16', 0, 'l');
         [n, p, m] = fopen(f); fclose(f);
         g = fopen('b.bin', 'r', 'b'); x = fread(g, 2, 'uint16'); fclose(g);
         h = fopen('b.bin', 'r', 'n', 'UTF-8'); y = fread(h, 1, 'uint16'); fclose(h);",
    );
    assert_eq!(s.file("b.bin"), [1, 2, 2, 1]);
    assert_eq!(s.text("m"), "ieee-be");
    assert_eq!(s.nums("x"), [258., 513.]);
    assert_eq!(s.nums("y"), [513.]);
    // Reference behavior (o2:29).
    assert_eq!(
        s.err("fopen('b.bin', 'r', 'vaxd')"),
        "Invalid machine format."
    );
    assert!(
        s.err("fopen('b.bin', 'r', 'n', 'Shift_JIS')")
            .contains("not supported yet")
    );
    s.run("none = fopen('all');");
    assert_eq!(
        s.dims("none"),
        (0, 0),
        "a rejected fopen leaves no file open"
    );
}

#[test]
fn csvread_offsets_with_range() {
    let mut s = Sandbox::new("csvrange");
    s.put("a.csv", b"1,2,3\n4,5,6\n7,8,9\n");
    s.run("a = csvread('a.csv', 0, 1, [0 1 1 2]); b = csvread('a.csv', 1, 1, [1 1 2 2]);");
    assert_eq!(s.nums("a"), [2., 5., 3., 6.]);
    assert_eq!(s.dims("a"), (2, 2));
    assert_eq!(s.nums("b"), [5., 8., 6., 9.]);
    assert!(s.err("csvread('a.csv', 1, 1, [0 1])").contains("range"));
    // Reference behavior (o2:33): only a warning there, and an empty range fails.
    assert!(
        s.err("csvread('a.csv', 1, 1, [0 1 1 2])")
            .contains("R and C should match RANGE(1:2)")
    );
    assert_eq!(
        s.err("csvread('a.csv', 1, 1, [])"),
        "Index exceeds matrix dimensions."
    );
}

#[test]
fn textscan_keeps_fields_before_a_failure() {
    let mut s = Sandbox::new("tspart");
    s.run("C = textscan('1 2 3 x', '%f %f'); a = C{1}; b = C{2};");
    assert_eq!(s.nums("a"), [1., 3.]);
    assert_eq!(s.nums("b"), [2.]);
    s.put("p.txt", b"1 2 3 x");
    s.run("f = fopen('p.txt'); C = textscan(f, '%f %f'); a = C{1}; b = C{2}; p = ftell(f); fclose(f);");
    assert_eq!(s.nums("a"), [1., 3.]);
    assert_eq!(s.nums("b"), [2.]);
    assert_eq!(s.nums("p"), [6.], "stops at the field that failed");
}

#[test]
fn zero_row_writes_keep_the_file_and_stay_bounded() {
    let mut s = Sandbox::new("zerorow");
    s.put("existing.csv", b"7,8\n");
    let t = std::time::Instant::now();
    assert!(
        !s.err("csvwrite('existing.csv', zeros(0,0), 0, 1000000000)")
            .is_empty()
    );
    assert!(
        !s.err("dlmwrite('existing.csv', zeros(0,1), 'precision', '%z')")
            .is_empty()
    );
    assert!(
        !s.err("dlmwrite('existing.csv', zeros(1,0), 'precision', '%z')")
            .is_empty()
    );
    assert!(t.elapsed().as_secs_f64() < 1.0);
    assert_eq!(s.file("existing.csv"), b"7,8\n");
}

#[test]
fn fscanf_character_width_is_clamped_without_overflow() {
    let mut s = Sandbox::new("scanwidth");
    s.put("c.txt", b"abcdef");
    let width = usize::MAX - 1;
    s.run(&format!(
        "f = fopen('c.txt'); a = fscanf(f, '%2c%{width}c'); p = ftell(f);
         frewind(f); b = fscanf(f, '%2c%*{width}c'); fclose(f);"
    ));
    assert_eq!(s.text("a"), "abcdef");
    assert_eq!(s.nums("p"), [6.]);
    assert_eq!(s.text("b"), "ab");
}

#[test]
fn textscan_short_lines_keep_numeric_columns_aligned() {
    let mut s = Sandbox::new("shortnumbers");
    // The LF cases are the reviewer's reference results; CRLF is a regression.
    for (input, options) in [
        ("1,2\\n3\\n5,6", ", 'Delimiter', ','"),
        ("1 2\\n3\\n5 6", ""),
        ("1,2\\r\\n3\\r\\n5,6", ", 'Delimiter', ','"),
        ("1 2\\r\\n3  \\r\\n5 6", ""),
    ] {
        s.run(&format!(
            "C = textscan(sprintf('{input}'), '%f %f'{options}); a = C{{1}}; b = C{{2}};"
        ));
        assert_eq!(s.nums("a"), [1., 3., 5.], "{input}");
        let b = s.nums("b");
        assert_eq!(b.len(), 3, "{input}");
        assert_eq!(b[0], 2.);
        assert!(b[1].is_nan());
        assert_eq!(b[2], 6.);
        assert_eq!(s.dims("a"), (3, 1));
        assert_eq!(s.dims("b"), (3, 1));
    }
    s.put("short.csv", b"1,2\n3\n5,6\n");
    s.run(
        "f = fopen('short.csv'); C = textscan(f, '%f %f', 2, 'Delimiter', ',', 'CollectOutput', 1);
         a = C{1}; p = ftell(f); D = textscan(f, '%f %f', 'Delimiter', ',');
         x = D{1}; y = D{2}; fclose(f);",
    );
    assert_eq!(s.dims("a"), (2, 2));
    let a = s.nums("a");
    assert_eq!(&a[..3], [1., 3., 2.]);
    assert!(a[3].is_nan());
    assert_eq!(s.nums("p"), [6.]);
    assert_eq!(s.nums("x"), [5.]);
    assert_eq!(s.nums("y"), [6.]);
}

#[test]
fn textscan_short_lines_fill_strings_and_skipped_fields() {
    let mut s = Sandbox::new("shortstrings");
    for (input, options) in [
        ("1,a,2\\n3\\n5,c,6\\n", ", 'Delimiter', ','"),
        ("1 a 2\\n3\\n5 c 6\\n", ""),
    ] {
        s.run(&format!(
            "C = textscan(sprintf('{input}'), '%*f %s %f'{options});
             words = C{{1}}; a = words{{1}}; b = words{{2}}; c = words{{3}}; x = C{{2}};"
        ));
        assert_eq!(s.text("a"), "a");
        assert_eq!(s.text("b"), "");
        assert_eq!(s.text("c"), "c");
        let x = s.nums("x");
        assert_eq!(x.len(), 3);
        assert_eq!(x[0], 2.);
        assert!(x[1].is_nan());
        assert_eq!(x[2], 6.);
    }
}

#[test]
fn dlmread_rejects_non_numeric_fields() {
    let mut s = Sandbox::new("dlmmismatch");
    s.put("bad.csv", b"1,abc,3\n");
    let prefix = "Mismatch between file and format string. Trouble reading 'Numeric' field from file (row number 1, field number 2) ==> abc,3";
    for call in ["dlmread('bad.csv')", "csvread('bad.csv')"] {
        assert!(s.err(call).starts_with(prefix), "{call}");
    }
    s.put("bad.txt", b"1 2\n3 4x\n");
    assert!(
        s.err("dlmread('bad.txt')")
            .contains("row number 2, field number 2")
    );
    s.put("empty.csv", b"1,,3\n4,\n");
    s.run("a = dlmread('empty.csv');");
    assert_eq!(s.nums("a"), [1., 4., 0., 0., 3., 0.]);
}

#[test]
fn text_modes_translate_newlines_and_keep_physical_positions() {
    let mut s = Sandbox::new("textmode");
    s.run(
        "f = fopen('t.txt', 'wt'); fprintf(f, 'a\\n'); fclose(f);
         f = fopen('t.txt', 'at'); n = fwrite(f, [98 10]); fclose(f);
         f = fopen('t.txt', 'rt'); a = fgets(f, 2); p = ftell(f); b = fgets(f);
         frewind(f); [r,k] = fread(f); fclose(f);
         f = fopen('t.txt', 'rb'); raw = fread(f); fclose(f);
         f = fopen('binary.txt', 'wb'); fprintf(f, 'a\\n'); fclose(f);",
    );
    assert_eq!(s.file("t.txt"), b"a\r\nb\r\n");
    assert_eq!(s.text("a"), "a\n");
    assert_eq!(s.nums("p"), [3.]);
    assert_eq!(s.text("b"), "b\n");
    assert_eq!(s.nums("n"), [2.]);
    assert_eq!(s.nums("r"), [97., 10., 98., 10.]);
    assert_eq!(s.nums("k"), [4.]);
    assert_eq!(s.nums("raw"), [97., 13., 10., 98., 13., 10.]);
    assert_eq!(s.file("binary.txt"), b"a\n");
    s.put("mixed.txt", b"a\r\nb\rc\n");
    s.run(
        "f = fopen('mixed.txt', 'rt'); a = fgets(f, 1); b = fgets(f, 1); p = ftell(f);
         c = fgets(f); g = fgets(f); frewind(f); [d,t] = fgetl(f); fclose(f);",
    );
    assert_eq!(s.text("a"), "a");
    assert_eq!(s.text("b"), "\n");
    assert_eq!(s.nums("p"), [3.]);
    assert_eq!(s.text("c"), "b\r", "a lone CR ends a line");
    assert_eq!(s.text("g"), "c\n");
    assert_eq!(s.text("d"), "a");
    assert_eq!(s.text("t"), "\n");
}

#[test]
fn text_mode_scanners_and_update_modes_translate_newlines() {
    let mut s = Sandbox::new("textscanmode");
    s.run(
        "f = fopen('t.txt', 'w+t'); fprintf(f, 'a\\nb\\n'); frewind(f);
         a = fscanf(f, '%2c', 2); p = ftell(f); b = fgets(f);
         fseek(f, 0, 'bof'); fprintf(f, 'c\\n'); fclose(f);",
    );
    assert_eq!(s.text("a"), "a\n");
    assert_eq!(s.nums("p"), [3.]);
    assert_eq!(s.text("b"), "b\n");
    assert_eq!(s.file("t.txt"), b"c\r\nb\r\n");
    s.put("n.txt", b"1,2\r\n3\r\n5,6\r\n");
    s.run(
        "f = fopen('n.txt', 'rt'); C = textscan(f, '%f %f', 2, 'Delimiter', ',');
         a = C{1}; b = C{2}; p = ftell(f); c = fgets(f); fclose(f);",
    );
    assert_eq!(s.nums("a"), [1., 3.]);
    let b = s.nums("b");
    assert_eq!(b.len(), 2);
    assert_eq!(b[0], 2.);
    assert!(b[1].is_nan());
    assert_eq!(s.nums("p"), [8.]);
    assert_eq!(s.text("c"), "5,6\n");
}

#[test]
fn text_mode_expansion_is_bounded_before_writing() {
    let mut s = Sandbox::new("textbound");
    let bytes = vec![b'x'; 8 * 1024 * 1024];
    s.put("full.txt", &bytes);
    s.run("f = fopen('full.txt', 'r+t'); fseek(f, -1, 'eof'); p = ftell(f);");
    assert!(s.err("fprintf(f, '\\n')").contains("8 MiB"));
    s.run("q = ftell(f); fclose(f);");
    assert_eq!(s.nums("q"), s.nums("p"));
    assert_eq!(s.file("full.txt"), bytes);
}

#[test]
fn textscan_stops_at_end_of_file_with_uneven_columns() {
    let mut s = Sandbox::new("tseof");
    s.put("e.txt", b"1 2\n3");
    // Reference behavior: C{1}=[1;3], C{2}=2, no NaN padding at the end of the file.
    s.run(
        "f = fopen('e.txt'); C = textscan(f, '%f %f'); a = C{1}; b = C{2};
         p = ftell(f); e = feof(f); fclose(f);",
    );
    assert_eq!(s.nums("a"), [1., 3.]);
    assert_eq!(s.nums("b"), [2.]);
    assert_eq!(s.nums("p"), [5.]);
    assert_eq!(s.nums("e"), [1.]);
    // The while ~feof idiom ends; the bound makes a regression fail instead of hang.
    s.run(
        "f = fopen('e.txt'); n = 0; x = [];
         while ~feof(f) && n < 10, C = textscan(f, '%f %f', 1); x = [x; C{1}]; n = n + 1; end
         fclose(f);",
    );
    assert_eq!(s.nums("n"), [2.]);
    assert_eq!(s.nums("x"), [1., 3.]);
    for (input, options) in [("1 2 3", ""), ("1,2\\n3", ", 'Delimiter', ','")] {
        s.run(&format!(
            "C = textscan(sprintf('{input}'), '%f %f'{options}); a = C{{1}}; b = C{{2}};"
        ));
        assert_eq!(s.nums("a"), [1., 3.], "{input}");
        assert_eq!(s.nums("b"), [2.], "{input}");
    }
}

#[test]
fn fscanf_character_conversion_keeps_trailing_whitespace() {
    let mut s = Sandbox::new("ctrail");
    s.put("c.txt", b"ab \n\t");
    s.run("f = fopen('c.txt'); a = fscanf(f, '%c'); e = feof(f); fclose(f);");
    assert_eq!(s.text("a"), "ab \n\t");
    assert_eq!(s.nums("e"), [1.]);
    s.put("w.txt", b"  \n");
    s.run("f = fopen('w.txt'); b = fscanf(f, '%c'); fclose(f);");
    assert_eq!(s.text("b"), "  \n");
}

#[test]
fn reference_open_line_and_seek_details() {
    let mut s = Sandbox::new("oracle1");
    // Reference o1:7.
    assert_eq!(s.err("fopen('t.txt', 'z')"), "Invalid permission.");
    // Reference o1:17-20: a failed fseek still clears the end-of-file flag.
    s.put("t.txt", b"hello\nworld\n\nlast");
    s.run(
        "f = fopen('t.txt'); fseek(f, -3, 'eof'); l = fgetl(f); e1 = feof(f);
         r = fseek(f, 100, 'bof'); e2 = feof(f); p = ftell(f); fclose(f);",
    );
    assert_eq!(s.text("l"), "ast");
    assert_eq!(s.nums("e1"), [1.]);
    assert_eq!(s.nums("r"), [-1.]);
    assert_eq!(s.nums("e2"), [0.]);
    assert_eq!(s.nums("p"), [17.]);
    // Reference o1:53-54: a file opened only for writing reads '' (not -1).
    s.run("f = fopen('w.txt', 'w'); a = fgetl(f); b = fgets(f); fclose(f);");
    assert_eq!(s.dims("a"), (0, 0));
    assert_eq!(s.dims("b"), (0, 0));
    // Reference o1:58: in text mode a lone CR ends the line read by fgets.
    s.run(
        "f = fopen('n.txt', 'wt'); fprintf(f, 'a\\r\\nb\\n'); fclose(f);
         f = fopen('n.txt', 'rt'); x = fread(f)'; frewind(f); g = fgets(f); fclose(f);",
    );
    assert_eq!(s.file("n.txt"), b"a\r\r\nb\r\n");
    assert_eq!(s.nums("x"), [97., 13., 10., 98., 10.]);
    assert_eq!(s.text("g"), "a\r");
    // Binary mode: CR LF is one terminator (regression, not reference-checked).
    s.put("b.txt", b"ab\r\ncd\r");
    s.run("f = fopen('b.txt'); [l, t] = fgets(f); [m, u] = fgetl(f); fclose(f);");
    assert_eq!(s.text("l"), "ab\r\n");
    assert_eq!(s.text("t"), "\r\n");
    assert_eq!(s.text("m"), "cd");
    assert_eq!(s.text("u"), "\r");
    // Reference o2:3: fscanf has two outputs.
    s.put("d.txt", b"1 2 3\n");
    s.run("f = fopen('d.txt');");
    assert_eq!(
        s.err("[a, b, c] = fscanf(f, '%d')"),
        "Too many output arguments."
    );
    s.run("fclose(f);");
}

#[test]
fn fopen_latin1_encoding() {
    let mut s = Sandbox::new("latin1");
    s.put("l.txt", &[b'c', 0xE9, b' ', 0xFC, b'\n']);
    // Reference o2:30 opens with 'latin1'.
    s.run(
        "[f, m] = fopen('l.txt', 'r', 'n', 'latin1'); a = fgetl(f); frewind(f);
         b = fscanf(f, '%s'); frewind(f); C = textscan(f, '%s'); c = C{1}; c2 = c{2};
         [n1, p1, m1, enc] = fopen(f); fclose(f);",
    );
    assert_eq!(s.nums("f"), [3.]);
    assert_eq!(s.dims("m"), (0, 0));
    assert_eq!(s.text("a"), "c\u{e9} \u{fc}");
    assert_eq!(s.text("b"), "c\u{e9}\u{fc}");
    assert_eq!(s.text("c2"), "\u{fc}");
    assert_eq!(s.text("enc"), "ISO-8859-1");
    s.run("f = fopen('o.txt', 'w', 'n', 'ISO-8859-1'); fprintf(f, '%s', a);");
    assert!(s.err("fprintf(f, '%s', char(937))").contains("ISO-8859-1"));
    s.run("fclose(f);");
    assert_eq!(s.file("o.txt"), [b'c', 0xE9, b' ', 0xFC]);
}
