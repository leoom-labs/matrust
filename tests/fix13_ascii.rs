use matrust::{Array, Engine};

#[test]
fn fix15_ascii_bom_numeric_readers_and_table_header() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    std::fs::write(dir.path().join("numeric.csv"), b"\xef\xbb\xbf1,2\n3,4").unwrap();
    std::fs::write(
        dir.path().join("header.csv"),
        b"\xef\xbb\xbfa,b\n1,2\n3,4\n",
    )
    .unwrap();
    e.execute(
        "expected = [1 2; 3 4];
         assert(isequal(readmatrix('numeric.csv'), expected));
         assert(isequal(csvread('numeric.csv'), expected));
         assert(isequal(dlmread('numeric.csv'), expected));
         assert(isequal(dlmread('numeric.csv', ','), expected));
         assert(isequal(load('numeric.csv', '-ascii'), expected));
         assert(isequal(readmatrix('header.csv'), expected));
         t = readtable('header.csv'); names = t.Properties.VariableNames;
         assert(strcmp(names{1}, 'a'));
         assert(isequal(t.a, [1; 3]));",
    )
    .unwrap();
}

#[test]
fn fix15_ascii_bom_text_streams_keep_byte_positions() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    std::fs::write(dir.path().join("text.csv"), b"\xef\xbb\xbf1,2\r\n3,4").unwrap();
    e.execute(
        "fid = fopen('text.csv', 'rt');
         assert(strcmp(fgetl(fid), '1,2'));
         assert(ftell(fid) == 8);
         assert(strcmp(fgetl(fid), '3,4'));
         frewind(fid);
         assert(strcmp(fgets(fid), sprintf('1,2\\n')));
         frewind(fid);
         first = textscan(fid, '%f%f', 1, 'Delimiter', ',');
         assert(isequal(first{1}, 1) && isequal(first{2}, 2));
         assert(ftell(fid) == 8);
         next = textscan(fid, '%f%f', 'Delimiter', ',');
         assert(isequal(next{1}, 3) && isequal(next{2}, 4));
         fclose(fid);
         fid = fopen('text.csv', 'rb');
         assert(isequal(fread(fid, 3), [239; 187; 191]));
         fclose(fid);",
    )
    .unwrap();
}

#[test]
fn fix15_ascii_load_does_not_split_rows_on_semicolon() {
    // GNU Octave 10.3.0 rejects this as an ASCII matrix. The public load page
    // lists the semicolon as a field delimiter within one row.
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    std::fs::write(dir.path().join("rows.txt"), b"1 2;3 4\n").unwrap();
    assert!(e.execute("load('rows.txt', '-ascii')").is_err());
}

#[test]
fn fix13_ascii_readmatrix_header_ragged_width() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    let path = dir.path().join("f.csv");
    std::fs::write(&path, "p,q\n1,\n2,\n3,5\n").unwrap();
    e.execute("A=readmatrix('f.csv');").unwrap();
    let a = &e.variables["A"];
    assert_eq!((a.rows(), a.cols()), (3, 2));
    assert_eq!(&a.data()[..3], &[1., 2., 3.]);
    assert!(a.data()[3].is_nan() && a.data()[4].is_nan());
    assert_eq!(a.data()[5], 5.);

    // The widest row may be beyond the import options detection sample.
    std::fs::write(&path, format!("p,q,r\n{}3,5,7\n", "1,\n".repeat(8200))).unwrap();
    e.execute("A=readmatrix('f.csv');").unwrap();
    let a = &e.variables["A"];
    assert_eq!((a.rows(), a.cols()), (8201, 3));
    assert_eq!(a.data()[8200], 3.);
    assert_eq!(a.data()[16401], 5.);
    assert_eq!(a.data()[24602], 7.);
}

#[test]
fn fix13_ascii_readmatrix_ignores_trailing_spaces_after_header() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    std::fs::write(dir.path().join("f.txt"), "x y\n1 2 \n3 4 \n").unwrap();
    e.execute("A=readmatrix('f.txt');").unwrap();
    let a = &e.variables["A"];
    assert_eq!((a.rows(), a.cols()), (2, 2));
    assert_eq!(a.data(), &[1., 3., 2., 4.]);
}

#[test]
fn fix13_ascii_range_keeps_skipped_row_width() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    std::fs::write(dir.path().join("f.csv"), "1,2\n3\n").unwrap();
    e.execute("A=dlmread('f.csv',',',[1 1 1 1]); B=csvread('f.csv',1,1,[1 1 1 1]);")
        .unwrap();
    for name in ["A", "B"] {
        let a = &e.variables[name];
        assert_eq!((a.rows(), a.cols()), (1, 1));
        assert_eq!(a.data(), &[0.]);
    }
}

#[test]
fn fix13_ascii_numeric_ragged_merge_base_sweep() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    // The merge-base strict matrix reader rejects these; its delimited
    // reader pads absent and empty fields with zero in column-major order.
    for (text, dims, expected) in [
        ("1,2\n3\n", (2, 2), vec![1., 3., 2., 0.]),
        ("1\n2,3\n", (2, 2), vec![1., 2., 0., 3.]),
        ("1,,3\n4,5,6\n", (2, 3), vec![1., 4., 0., 5., 3., 6.]),
        ("1,2,\n3,4,\n", (2, 3), vec![1., 3., 2., 4., 0., 0.]),
        (",2\n3,\n", (2, 2), vec![0., 3., 2., 0.]),
    ] {
        std::fs::write(dir.path().join("sweep.csv"), text).unwrap();
        assert!(e.execute("readmatrix('sweep.csv')").is_err(), "{text:?}");
        e.execute("A=csvread('sweep.csv'); B=dlmread('sweep.csv',',');")
            .unwrap();
        for name in ["A", "B"] {
            let a = &e.variables[name];
            assert_eq!((a.rows(), a.cols()), dims, "{text:?} {name}");
            assert_eq!(a.data(), expected, "{text:?} {name}");
        }
    }
}

#[test]
fn fix13_ascii_load_save_and_readers() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    std::fs::write(dir.path().join("d.txt"), "1 2 3\n4 5 6\n").unwrap();
    std::fs::write(dir.path().join("my-data.txt"), "1,2 % note\n3\t4\n").unwrap();
    std::fs::write(dir.path().join("10-May-data.dat"), "8 9\n").unwrap();
    std::fs::write(dir.path().join("forced.mat"), "7 8\n").unwrap();
    std::fs::write(dir.path().join("bad.txt"), "1 2\n3\n").unwrap();
    std::fs::write(dir.path().join("c.csv"), "a,b\n1,2\n3,4\n").unwrap();
    std::fs::write(dir.path().join("headers.csv"), "title\na,b\n1,2\n3,4\n").unwrap();
    std::fs::write(dir.path().join("col.csv"), "x,1,2\ny,3,4\n").unwrap();
    std::fs::write(
        dir.path().join("range.csv"),
        "header,header,ignored\na,1,2,ignored\nb,3,4,ignored\nignored,ignored\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("bad.csv"), "1,2\n3,no\n").unwrap();
    for code in [
        "load d.txt",
        "M=load('d.txt');",
        "load my-data.txt",
        "load 10-May-data.dat",
        "H=load('forced.mat','-ascii');",
        "A=csvread('c.csv',1,0);",
        "B=dlmread('c.csv',',',1,0);",
        "C=csvread('col.csv',0,1);",
        "D=readmatrix('c.csv');",
        "P=readmatrix('headers.csv');",
        "Q=readmatrix('c.csv','Delimiter',',');",
        "E=dlmread('col.csv',',',[0 1 1 2]);",
        "T=dlmread('range.csv',',',[1 1 2 2]);",
        "x=[1.23456789 2.34567891]; save out.txt x -ascii; save precise.txt x -ascii -double -tabs; F=load('out.txt'); G=load('precise.txt');",
        "z=9; a=2; save order.txt z a -ascii; J=load('order.txt');",
        "z=1+2i; save complex.txt z -ascii; K=load('complex.txt');",
    ] {
        e.execute(code)
            .unwrap_or_else(|err| panic!("{code}: {err}"));
    }
    let nums = |name: &str| match e.array(name).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        x => panic!("{x:?}"),
    };
    assert_eq!(nums("d"), vec![1., 4., 2., 5., 3., 6.]);
    assert_eq!(nums("M"), nums("d"));
    assert_eq!(nums("my_data"), vec![1., 3., 2., 4.]);
    assert_eq!(nums("X10_May_data"), vec![8., 9.]);
    assert_eq!(nums("H"), vec![7., 8.]);
    assert_eq!(nums("J"), vec![9., 2.]);
    assert_eq!(nums("K"), vec![1.]);
    assert!(
        std::fs::read_to_string(dir.path().join("precise.txt"))
            .unwrap()
            .contains('\t')
    );
    for name in ["A", "B", "C", "D", "E", "P", "Q", "T"] {
        assert_eq!(nums(name), vec![1., 3., 2., 4.], "{name}");
    }
    for (a, b) in nums("x").iter().zip(nums("F")) {
        assert!((a - b).abs() / a.abs() < 1e-7);
    }
    assert_eq!(nums("G"), nums("x"));
    let warning = e
        .execute("label='hello'; save mixed.txt label x -ascii;")
        .unwrap();
    assert!(warning.output.join("").contains("Warning:"));
    assert!(
        !std::fs::read_to_string(dir.path().join("mixed.txt"))
            .unwrap()
            .is_empty()
    );
    assert!(
        e.execute("load bad.txt")
            .unwrap_err()
            .to_string()
            .contains("line 2")
    );
    assert!(e.execute("csvread('bad.csv',0,0)").is_err());
    assert!(e.execute("readmatrix('bad.csv')").is_err());
}

#[test]
fn fix13_ascii_readmatrix_headers_use_the_file_delimiter() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    let path = dir.path().join("sensors.txt");
    for delimiter in [',', ';', '\t', ' '] {
        let title = if delimiter == ';' {
            "Measurements, in volts, May, archived"
        } else {
            "Measurements; in volts; May; archived"
        };
        std::fs::write(
            &path,
            format!(
                "{title}\nSensor 1{delimiter}Sensor 2\n1{delimiter}Reading 2\n1{delimiter}2\n3{delimiter}4\n"
            ),
        )
        .unwrap();
        e.execute("A=readmatrix('sensors.txt');").unwrap();
        assert_eq!(e.variables["A"].data(), &[1., 3., 2., 4.], "{delimiter:?}");
        if delimiter != ' ' {
            e.execute(&format!(
                "A=readmatrix('sensors.txt','Delimiter','{delimiter}');"
            ))
            .unwrap();
            assert_eq!(e.variables["A"].data(), &[1., 3., 2., 4.], "{delimiter:?}");
        }
    }
}

#[test]
fn fix13_ascii_save_preserves_an_extensionless_filename() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute("z=9; save noext z -ascii").unwrap();
    let text = std::fs::read_to_string(dir.path().join("noext")).unwrap();
    assert_eq!(text.trim().parse::<f64>().unwrap(), 9.);
    assert!(!dir.path().join("noext.mat").exists());
}

#[test]
fn fix13_ascii_double_round_trips_all_finite_bits() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    e.execute(
        "x=[1.0000000000000002 -1.2345678901234567 4.9406564584124654e-324 1.7976931348623157e308 -0.0];\n\
         save precise.txt x -ascii -double; y=load('precise.txt');",
    )
    .unwrap();
    let bits = |name: &str| {
        e.variables[name]
            .data()
            .iter()
            .map(|x| x.to_bits())
            .collect::<Vec<_>>()
    };
    assert_eq!(bits("y"), bits("x"));
    let text = std::fs::read_to_string(dir.path().join("precise.txt")).unwrap();
    assert_eq!(&text[..26], "    1.0000000000000002e+00");
}

#[test]
fn fix13_ascii_special_values_use_matlab_spelling_and_load_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    for precision in ["", " -double"] {
        e.execute(&format!(
            "x=[Inf,-Inf,NaN]; save special.txt x -ascii{precision}; y=load('special.txt');"
        ))
        .unwrap();
        let text = std::fs::read_to_string(dir.path().join("special.txt")).unwrap();
        assert!(text.split_whitespace().eq(["Inf", "-Inf", "NaN"]));
        let values = e.variables["y"].data();
        assert_eq!(values.len(), 3);
        assert_eq!(&values[..2], &[f64::INFINITY, f64::NEG_INFINITY]);
        assert!(values[2].is_nan());
    }
}

#[test]
fn fix15c_csv_strips_only_one_leading_utf8_bom() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Engine::default();
    e.set_file_directory(dir.path()).unwrap();
    let path = dir.path().join("double_bom.csv");
    std::fs::write(&path, "\u{feff}\u{feff}1,2\n3,4\n").unwrap();
    assert!(e.execute("csvread('double_bom.csv');").is_err());
    assert!(e.execute("dlmread('double_bom.csv', ',');").is_err());
    e.execute("M = readmatrix('double_bom.csv');").unwrap();
    assert_eq!(e.variables["M"].data(), &[3., 4.]);
}
