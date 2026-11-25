//! xlswrite (round trips through xlsread/readcell, sheet and range
//! placement, updating a workbook) and webread/websave/weboptions against a
//! local HTTP server started by the test (no network). Expected values
//! follow the published xlswrite/webread/websave descriptions.
use matrust::{Array, Engine};
use std::{
    io::{Read, Write},
    net::TcpListener,
};

fn engine() -> (Engine, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_current_folder(dir.path()).unwrap();
    (e, dir)
}

fn run(e: &mut Engine, source: &str) -> String {
    match e.execute(source) {
        Ok(x) => x.output.join("\n"),
        Err(error) => panic!("{source}: {error}"),
    }
}

fn fails(e: &mut Engine, source: &str) -> String {
    match e.execute(source) {
        Ok(x) => panic!("{source} should fail, printed {:?}", x.output),
        Err(error) => error.to_string(),
    }
}

#[track_caller]
fn check(e: &mut Engine, expr: &str) {
    run(e, &format!("t__ = {expr};"));
    let ok = match e.array("t__") {
        Some(Array::Logical(m)) => !m.data().is_empty() && m.data().iter().all(|v| *v != 0),
        Some(Array::Numeric(m)) => !m.data().is_empty() && m.data().iter().all(|v| *v != 0.),
        other => panic!("{expr}: not logical: {other:?}"),
    };
    assert!(ok, "{expr} is false");
}

#[test]
fn xlswrite_matrix_reads_back_equal() {
    let (mut e, dir) = engine();
    run(
        &mut e,
        "A = [magic(4); 0.1 -2.5e-7 1e300 -0]; xlswrite('m.xlsx', A);",
    );
    check(&mut e, "isequal(xlsread('m.xlsx'), A)");
    // The package is a zip with the parts every reader needs.
    let bytes = std::fs::read(dir.path().join("m.xlsx")).unwrap();
    assert!(bytes.starts_with(b"PK\x03\x04"));
    // No extension: .xlsx is added; the status output is true.
    run(&mut e, "s = xlswrite('plain', [1 2; 3 4]);");
    check(&mut e, "s == true && islogical(s)");
    check(&mut e, "isequal(readmatrix('plain.xlsx'), [1 2; 3 4])");
}

#[test]
fn xlswrite_cells_text_logicals_and_gaps() {
    let (mut e, _dir) = engine();
    run(
        &mut e,
        "xlswrite('c.xlsx', {'name', 'value'; 'a<b & \"c\"', 3; ' pad ', true; '', NaN});",
    );
    run(&mut e, "[n, t, r] = xlsread('c.xlsx');");
    check(&mut e, "isequal(n, [3; 1])");
    check(
        &mut e,
        "isequal(t, {'name', 'value'; 'a<b & \"c\"', ''; ' pad ', ''})",
    );
    check(&mut e, "islogical(r{3, 2}) && r{3, 2}");
    // The all-empty last row is not part of the used area.
    check(&mut e, "isequal(size(r), [3 2])");
    // A string array and a char vector.
    run(
        &mut e,
        "xlswrite('s.xlsx', [\"x\" \"y\"]); xlswrite('v.xlsx', 'hello');",
    );
    check(&mut e, "isequal(readcell('s.xlsx'), {'x', 'y'})");
    check(&mut e, "isequal(readcell('v.xlsx'), {'hello'})");
    // Integers are written as numbers.
    run(&mut e, "xlswrite('i.xlsx', int8([1 -2]));");
    check(&mut e, "isequal(xlsread('i.xlsx'), [1 -2])");
}

#[test]
fn xlswrite_places_data_by_sheet_and_range() {
    let (mut e, _dir) = engine();
    run(&mut e, "xlswrite('b.xlsx', magic(3));");
    run(&mut e, "xlswrite('b.xlsx', [7 8], 'Extra', 'C2');");
    run(&mut e, "xlswrite('b.xlsx', [1 2 3 4], 'B2:C2');");
    run(&mut e, "xlswrite('b.xlsx', 5, 4);");
    // Sheet 1 keeps magic(3) except the clipped range B2:C2.
    check(
        &mut e,
        "isequal(xlsread('b.xlsx', 1), [8 1 6; 3 1 2; 4 9 2])",
    );
    run(&mut e, "[~, ~, r] = xlsread('b.xlsx', 'Extra', 'A1:D2');");
    check(
        &mut e,
        "isnan(r{1, 1}) && isnan(r{2, 2}) && r{2, 3} == 7 && r{2, 4} == 8",
    );
    // Index 4 added Sheet2 and Sheet3 after Sheet1 and Extra.
    check(
        &mut e,
        "isequal(xlsread('b.xlsx', 4), 5) && isequal(xlsread('b.xlsx', 'Sheet3'), 5)",
    );
    check(&mut e, "isempty(xlsread('b.xlsx', 3))");
    // Writing NaN clears a cell of an existing sheet.
    run(&mut e, "xlswrite('b.xlsx', NaN, 'Sheet1', 'A1');");
    run(&mut e, "[~, ~, r] = xlsread('b.xlsx', 1);");
    check(&mut e, "isnan(r{1, 1}) && r{1, 2} == 1");
}

#[test]
fn xlswrite_rejects_what_it_cannot_write() {
    let (mut e, dir) = engine();
    assert!(fails(&mut e, "xlswrite('a.xls', 1)").contains("only .xlsx"));
    assert!(fails(&mut e, "xlswrite('a.xlsx', [1+2i 3])").contains("complex"));
    assert!(fails(&mut e, "xlswrite('a.xlsx', 1, 'bad/name')").contains("not a valid sheet name"));
    assert!(fails(&mut e, "xlswrite('a.xlsx', {[1 2]})").contains("cell (1,1)"));
    assert!(fails(&mut e, "xlswrite('a.xlsx', ones(2,2,2))").contains("N-D"));
    assert!(fails(&mut e, "xlswrite('a.xlsx', 1, 'A1:B2:C3')").contains("Range"));
    // Updating a file that is not an .xlsx package leaves it untouched.
    std::fs::write(dir.path().join("t.xlsx"), b"not a workbook").unwrap();
    assert!(fails(&mut e, "xlswrite('t.xlsx', 1)").contains("cannot update"));
    assert_eq!(
        std::fs::read(dir.path().join("t.xlsx")).unwrap(),
        b"not a workbook"
    );
    assert!(!dir.path().join("a.xlsx").exists());
}

// ---------------------------------------------------------------------------
// webread / websave against a local server

fn have_curl() -> bool {
    let ok = std::process::Command::new("curl")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("curl not found: web tests skipped");
    }
    ok
}

/// Serves canned responses on 127.0.0.1; `/echo` returns the request head.
fn server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") && head.len() < 65536 {
                match stream.read(&mut byte) {
                    Ok(1) => head.push(byte[0]),
                    _ => break,
                }
            }
            let head = String::from_utf8_lossy(&head).into_owned();
            let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (status, kind, body): (&str, &str, Vec<u8>) = match path.split('?').next().unwrap()
            {
                "/json" => (
                    "200 OK",
                    "application/json; charset=utf-8",
                    "{\"name\":\"caf\u{e9}\",\"values\":[1,2,3],\"ok\":true}"
                        .as_bytes()
                        .to_vec(),
                ),
                "/text" => (
                    "200 OK",
                    "text/plain; charset=utf-8",
                    "h\u{e9}llo\n".as_bytes().to_vec(),
                ),
                "/latin" => ("200 OK", "text/plain; charset=iso-8859-1", vec![b'h', 0xE9]),
                "/bin" => ("200 OK", "application/octet-stream", vec![0, 1, 255]),
                "/echo" => ("200 OK", "text/plain", head.clone().into_bytes()),
                "/moved" => ("302 Found", "text/plain", Vec::new()),
                "/longtype" => ("200 OK", "LONG", b"ok".to_vec()),
                _ => ("404 Not Found", "text/plain", b"missing".to_vec()),
            };
            // A 70 000-byte Content-Type: curl's --write-out output exceeds a pipe buffer.
            let long = format!("text/plain; x={}", "a".repeat(70_000));
            let kind = if kind == "LONG" { long.as_str() } else { kind };
            let location = if path == "/moved" {
                "Location: /json\r\n"
            } else {
                ""
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{location}Connection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    format!("http://127.0.0.1:{port}")
}

#[test]
fn webread_decodes_json_text_and_bytes() {
    if !have_curl() {
        return;
    }
    let url = server();
    let (mut e, _dir) = engine();
    run(&mut e, &format!("d = webread('{url}/json');"));
    check(
        &mut e,
        "isstruct(d) && isequal(d.values, [1; 2; 3]) && d.ok",
    );
    check(&mut e, "isequal(d.name, char([99 97 102 233]))");
    run(&mut e, &format!("t = webread('{url}/text');"));
    check(
        &mut e,
        "ischar(t) && isequal(double(t), [104 233 108 108 111 10])",
    );
    run(&mut e, &format!("l = webread('{url}/latin');"));
    check(&mut e, "isequal(double(l), [104 233])");
    run(&mut e, &format!("b = webread('{url}/bin');"));
    check(&mut e, "isa(b, 'uint8') && isequal(b, uint8([0; 1; 255]))");
    // Redirects are followed; ContentType forces the output kind.
    run(&mut e, &format!("m = webread('{url}/moved');"));
    check(&mut e, "isequal(m.values, [1; 2; 3])");
    run(
        &mut e,
        &format!("r = webread('{url}/json', weboptions('ContentType', 'text'));"),
    );
    check(&mut e, "ischar(r) && startsWith(r, '{')");
    // A huge Content-Type header does not stall the pipes.
    run(&mut e, &format!("w = webread('{url}/longtype');"));
    check(&mut e, "isequal(w, 'ok')");
}

#[test]
fn webread_sends_query_headers_and_user_agent() {
    if !have_curl() {
        return;
    }
    let url = server();
    let (mut e, _dir) = engine();
    run(
        &mut e,
        "o = weboptions('HeaderFields', {'X-Test', 'yes'}, 'UserAgent', 'probe/1', 'Timeout', 10);",
    );
    check(
        &mut e,
        "o.Timeout == 10 && isequal(o.HeaderFields, {'X-Test', 'yes'})",
    );
    run(
        &mut e,
        &format!("h = webread('{url}/echo', 'q', 'a b&c', 'n', [1 2.5], o);"),
    );
    check(
        &mut e,
        "startsWith(h, 'GET /echo?q=a%20b%26c&n=1%2C2.5 HTTP/1.1')",
    );
    check(
        &mut e,
        "contains(h, 'X-Test: yes') && contains(h, 'User-Agent: probe/1')",
    );
    // Changing a field of the options struct works too.
    run(&mut e, "o.UserAgent = 'other/2';");
    run(&mut e, &format!("h = webread('{url}/echo', o);"));
    check(&mut e, "contains(h, 'User-Agent: other/2')");
}

#[test]
fn websave_writes_the_body_and_keeps_files_on_failure() {
    if !have_curl() {
        return;
    }
    let url = server();
    let (mut e, dir) = engine();
    run(&mut e, &format!("f = websave('out.bin', '{url}/bin');"));
    assert_eq!(
        std::fs::read(dir.path().join("out.bin")).unwrap(),
        [0, 1, 255]
    );
    check(&mut e, "endsWith(f, 'out.bin') && ischar(f)");
    let message = fails(&mut e, &format!("websave('out.bin', '{url}/nothing')"));
    assert!(message.contains("status 404"), "{message}");
    assert_eq!(
        std::fs::read(dir.path().join("out.bin")).unwrap(),
        [0, 1, 255]
    );
    // No temporary files are left behind.
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["out.bin"]);
}

#[test]
fn web_errors_are_clear() {
    let (mut e, _dir) = engine();
    assert!(fails(&mut e, "webread('file:///etc/passwd')").contains("http://"));
    assert!(fails(&mut e, "webread('https://x.org', 'q')").contains("name/value"));
    assert!(fails(&mut e, "weboptions('Timeout', -1)").contains("positive"));
    assert!(fails(&mut e, "weboptions('Password', 'x')").contains("not supported"));
    assert!(fails(&mut e, "weboptions('ContentType', 'image')").contains("not supported"));
    if have_curl() {
        let url = server();
        assert!(fails(&mut e, &format!("webread('{url}/missing')")).contains("status 404"));
        // Nothing listens on port 1.
        assert!(fails(&mut e, "webread('http://127.0.0.1:1/x')").contains("could not read"));
    }
}
