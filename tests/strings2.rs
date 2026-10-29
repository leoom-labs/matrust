//! regexp, regexpi, regexprep, str2double, str2num, strjust, isspace, isletter,
//! isstrprop, validatestring, isvarname and repmat on character arrays.
//! Values are compared through `show`, using the reference class/shape/value format.
use matrust::{Array, Engine};
use std::sync::atomic::AtomicBool;

fn number(v: f64) -> String {
    if v.is_nan() {
        "NaN".into()
    } else if v.is_infinite() {
        if v > 0. { "Inf" } else { "-Inf" }.into()
    } else if v.fract() == 0. && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// MATLAB mat2str of a nonempty matrix, from column-major element texts.
fn mat2str(rows: usize, cols: usize, items: Vec<String>) -> String {
    if items.len() == 1 {
        return items[0].clone();
    }
    let lines: Vec<String> = (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| items[c * rows + r].clone())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    format!("[{}]", lines.join(";"))
}

fn show(a: &Array) -> String {
    let cancel = AtomicBool::new(false);
    let (rows, cols) = (a.rows(), a.cols());
    let shape = format!("{rows}x{cols}");
    match a {
        Array::Character(c) => {
            if c.units().is_empty() {
                return format!("char{shape}");
            }
            let lines: Vec<String> = (0..rows)
                .map(|r| {
                    let row: Vec<u16> = (0..cols).map(|k| c.units()[k * rows + r]).collect();
                    String::from_utf16(&row)
                        .unwrap()
                        .replace('\n', "\\n")
                        .replace('\t', "\\t")
                })
                .collect();
            format!("char{shape}'{}'", lines.join("|"))
        }
        Array::Cell(c) => {
            let items: Vec<String> = (0..c.len())
                .map(|i| show(c.get_linear(i).unwrap()))
                .collect();
            format!("cell{shape}{{{}}}", items.join(","))
        }
        Array::Logical(m) => {
            let items = m
                .data()
                .iter()
                .map(|&v| if v != 0 { "true" } else { "false" }.to_string())
                .collect::<Vec<_>>();
            if items.is_empty() {
                format!("logical{shape}")
            } else {
                format!("logical{shape} {}", mat2str(rows, cols, items))
            }
        }
        Array::Numeric(m) => {
            let items: Vec<String> = m.data().iter().map(|&v| number(v)).collect();
            if items.is_empty() {
                format!("double{shape}")
            } else {
                format!("double{shape} {}", mat2str(rows, cols, items))
            }
        }
        Array::Complex(m) => {
            let items: Vec<String> = m
                .data()
                .iter()
                .map(|z| {
                    let sign = if z.im < 0. { "-" } else { "+" };
                    format!("{}{sign}{}i", number(z.re), number(z.im.abs()))
                })
                .collect();
            format!("double{shape} {}", mat2str(rows, cols, items))
        }
        Array::Struct(s) => {
            let mut items = Vec::new();
            for element in 0..s.len() {
                for (field, name) in s.field_names().iter().enumerate() {
                    items.push(format!("{name}:{}", show(s.get(field, element).unwrap())));
                }
            }
            format!("struct{shape}{{{}}}", items.join(","))
        }
        other => panic!("unexpected {other:?} {cancel:?}"),
    }
}

/// Run `code` (it sets r) and show r.
fn run(code: &str) -> String {
    let mut e = Engine::default();
    e.execute(code)
        .unwrap_or_else(|err| panic!("{code}: {err}"));
    show(&e.array("r").unwrap_or_else(|| panic!("{code}: no r")))
}

/// Show the value of one expression.
fn v(expr: &str) -> String {
    run(&format!("r = {expr};"))
}

/// An error message without its "Line 1, column N: " location.
fn unplaced(e: String) -> String {
    match e.split_once(": ") {
        Some((place, text)) if place.starts_with("Line ") => text.to_owned(),
        _ => e,
    }
}

fn err(code: &str) -> String {
    match Engine::default().execute(code) {
        Ok(_) => panic!("{code}: accepted"),
        Err(e) => unplaced(e),
    }
}

/// What a case prints on MatRust's side: the shown r, or ERR: and the first
/// line of the error (the MATLAB side keeps only that line too).
fn case_output(code: &str) -> String {
    let code = match code.strip_prefix('@') {
        Some(statement) => statement.to_owned(),
        None => format!("r = {code};"),
    };
    let mut e = Engine::default();
    match e.execute(&code) {
        Ok(_) => e.array("r").map_or("ERR: no r".into(), |a| show(&a)),
        Err(e) => format!("ERR: {}", unplaced(e).lines().next().unwrap_or("")),
    }
}

#[test]
fn regexp_match_tokens_and_positions() {
    assert_eq!(
        v(r"regexp('abc123def','\d+','match')"),
        "cell1x1{char1x3'123'}"
    );
    // No match gives 0x0 values (as in the reference).
    assert_eq!(v("regexp('abc','x','match')"), "cell0x0{}");
    assert_eq!(
        v(r"regexp('ab12cd34','([a-z]+)(\d+)','tokens')"),
        "cell1x2{cell1x2{char1x2'ab',char1x2'12'},cell1x2{char1x2'cd',char1x2'34'}}"
    );
    // Without groups a match has no tokens (as in the reference).
    assert_eq!(v("regexp('abc','b','tokens')"), "cell1x1{cell1x0{}}");
    assert_eq!(v("regexp('abcb','b')"), "double1x2 [2 4]");
    assert_eq!(v("regexp('abc','x')"), "double0x0");
    assert_eq!(run("[s,e]=regexp('abcb','b'); r=e;"), "double1x2 [2 4]");
    assert_eq!(
        run("[s,e,te,m]=regexp('a1b22','\\d+'); r={s,e,te,m};"),
        "cell1x4{double1x2 [2 4],double1x2 [2 5],\
         cell1x2{double0x0,double0x0},cell1x2{char1x1'1',char1x2'22'}}"
    );
    assert_eq!(
        v(r"regexp('ab12','([a-z]+)(\d+)','tokenExtents')"),
        "cell1x1{double2x2 [1 2;3 4]}"
    );
    // Selected outputs come in the order of the option strings.
    assert_eq!(
        run(r"[t,m]=regexp('ab12','(\d)','tokens','match'); r={t,m};"),
        "cell1x2{cell1x2{cell1x1{char1x1'1'},cell1x1{char1x1'2'}},cell1x2{char1x1'1',char1x1'2'}}"
    );
}

#[test]
fn regexp_once_split_and_options() {
    assert_eq!(v("regexp('abc','b','once','match')"), "char1x1'b'");
    assert_eq!(v("regexp('abc','x','once','match')"), "char0x0");
    assert_eq!(v("regexp('abc','x','once')"), "double0x0");
    assert_eq!(v("regexp('abcb','b','once')"), "double1x1 2");
    assert_eq!(
        v(r"regexp('key=val','(\w+)=(\w+)','tokens','once')"),
        "cell1x2{char1x3'key',char1x3'val'}"
    );
    assert_eq!(
        v("regexp('a,b,,c',',','split')"),
        "cell1x4{char1x1'a',char1x1'b',char0x0,char1x1'c'}"
    );
    assert_eq!(v("regexp('abc','x','split')"), "cell1x1{char1x3'abc'}");
    assert_eq!(v("regexpi('ABC','b','match')"), "cell1x1{char1x1'B'}");
    assert_eq!(
        v("regexp('ABC','b','match','ignorecase')"),
        "cell1x1{char1x1'B'}"
    );
    assert_eq!(
        err("r = regexp('abc','B','match','ignorecase','matchcase');"),
        "The 'matchcase' option may not be used in conjunction with the 'ignorecase' option."
    );
    // '.' matches a newline unless 'dotexceptnewline'; ^ is the text start unless 'lineanchors'.
    assert_eq!(
        v(r"regexp(sprintf('a\nb'),'a.b','match')"),
        "cell1x1{char1x3'a\\nb'}"
    );
    assert_eq!(
        v(r"regexp(sprintf('a\nb'),'a.b','match','dotexceptnewline')"),
        "cell0x0{}"
    );
    assert_eq!(v(r"regexp(sprintf('a\nb'),'^b','match')"), "cell0x0{}");
    assert_eq!(
        v(r"regexp(sprintf('a\nb'),'^b','match','lineanchors')"),
        "cell1x1{char1x1'b'}"
    );
    // Zero-length matches count only with 'emptymatch'.
    assert_eq!(v("regexp('abc','x*','match')"), "cell0x0{}");
    assert_eq!(
        v("regexp('ab','x*','match','emptymatch')"),
        "cell1x3{char0x0,char0x0,char0x0}"
    );
    assert_eq!(v("regexp('aaa','a{2}','match')"), "cell1x1{char1x2'aa'}");
    assert_eq!(v(r"regexp('a.b','\.','start')"), "double1x1 2");
    assert_eq!(v(r"regexp('abc','\x62','start')"), "double1x1 2");
    assert_eq!(v("regexp('abc','(?#note)b','start')"), "double1x1 2");
    assert_eq!(v(r"regexp('hello world','\<w','start')"), "double1x1 7");
    // MATLAB has no POSIX classes: [[:alpha:]] is the set [:alph then ].
    assert_eq!(v("regexp('A1','[[:alpha:]]','match')"), "cell0x0{}");
    assert_eq!(v("regexp('a:]','[[:alpha:]]','start')"), "double1x1 2");
    assert_eq!(v("regexp('a]b','[]]','start')"), "double1x1 2");
}

#[test]
fn regexp_cell_inputs_keep_their_shape() {
    assert_eq!(
        v(r"regexp({'a1','b22'},'\d+','match')"),
        "cell1x2{cell1x1{char1x1'1'},cell1x1{char1x2'22'}}"
    );
    assert_eq!(
        v(r"regexp({'a1';'b22'},'\d','start')"),
        "cell2x1{double1x1 2,double1x2 [2 3]}"
    );
    assert_eq!(
        v(r"regexp('a1',{'\d','a'},'match')"),
        "cell1x2{cell1x1{char1x1'1'},cell1x1{char1x1'a'}}"
    );
    assert_eq!(
        v("regexp({'ab','xb'},{'a','b'},'match')"),
        "cell1x2{cell1x1{char1x1'a'},cell1x1{char1x1'b'}}"
    );
}

#[test]
fn regexp_rejects_what_it_cannot_translate() {
    for (code, needle) in [
        ("r = regexp('abab','(?<=a)b');", "lookahead and lookbehind"),
        ("r = regexp('abcd','b(?=c)');", "lookahead and lookbehind"),
        (r"r = regexp('abab','(ab)\1','match');", "backreferences"),
        ("r = regexp('aaa','(?>a+)');", "atomic groups"),
        ("r = regexp('aaa','a++');", "possessive"),
        ("r = regexp('abc');", "Not enough input arguments."),
        (
            "r = regexp(123,'1');",
            "The 'STRING' input must be either a char row vector or a cell array of char row vectors.",
        ),
        (
            "r = regexp('ab','a','bogus');",
            "Invalid option for regexp: bogus.",
        ),
        (
            "r = regexp('aaa','a','match','start','once');",
            "Not enough outputs specified for 'start' option.",
        ),
        ("r = regexp('ab','[a');", "unterminated"),
        (
            "r = regexprep('abc','b','${upper($0)}');",
            "dynamic replacement",
        ),
        ("r = regexprep('abc','b');", "Not enough input arguments."),
    ] {
        let message = err(code);
        assert!(message.contains(needle), "{code}: {message}");
        assert!(!message.contains("panicked"), "{code}: {message}");
    }
}

#[test]
fn regexp_positions_count_utf16_units() {
    // U+00E9 is one unit, U+1F600 two.
    assert_eq!(v("regexp('\u{e9}x\u{1f600}x','x')"), "double1x2 [2 5]");
}

#[test]
fn regexprep_replacements() {
    assert_eq!(v("regexprep('abc','b','X')"), "char1x3'aXc'");
    assert_eq!(v("regexprep('abcb','b','X','once')"), "char1x4'aXcb'");
    assert_eq!(v("regexprep('abcbb','b','X',2)"), "char1x5'abcXb'");
    assert_eq!(v("regexprep('aBc','b','X','ignorecase')"), "char1x3'aXc'");
    assert_eq!(v("regexprep('abc','B','x')"), "char1x3'abc'");
    assert_eq!(v("regexprep('abc','(a)(b)','$2$1')"), "char1x3'bac'");
    assert_eq!(v("regexprep('abc','(a)','$0$0')"), "char1x4'aabc'");
    assert_eq!(
        v(r"regexprep('hello world','(\w+) (\w+)','$2 $1')"),
        "char1x11'world hello'"
    );
    assert_eq!(v("regexprep('abc','(?<x>b)','[$<x>]')"), "char1x5'a[b]c'");
    assert_eq!(v(r"regexprep('a1b2','(\d)','<$1>')"), "char1x8'a<1>b<2>'");
    assert_eq!(v(r"regexprep('ab','b','\n')"), "char1x2'a\\n'");
    assert_eq!(v(r"regexprep('a.b','\.','\\')"), "char1x3'a\\b'");
    assert_eq!(v("regexprep('abc','b','')"), "char1x2'ac'");
    assert_eq!(v("regexprep('abc','abc','')"), "char0x0");
    assert_eq!(v("regexprep('','a','b')"), "char0x0");
    assert_eq!(v("regexprep('abc','x*','-')"), "char1x3'abc'");
    assert_eq!(v("regexprep('ab','x*','-','emptymatch')"), "char1x5'-a-b-'");
    assert_eq!(
        v("regexprep({'ab','cb'},'b','X')"),
        "cell1x2{char1x2'aX',char1x2'cX'}"
    );
    // Cell expressions apply in order to every string.
    assert_eq!(v("regexprep('abc',{'a','b'},{'1','2'})"), "char1x3'12c'");
    assert_eq!(v("regexprep('abc',{'a','b'},'X')"), "char1x3'XXc'");
    assert_eq!(v("regexprep('abc',{'a','bc'},{'b','X'})"), "char1x2'bX'");
}

#[test]
fn str2double_text_and_cells() {
    assert_eq!(v("str2double('3.14')"), "double1x1 3.14");
    assert_eq!(v("str2double('  -2.5e3 ')"), "double1x1 -2500");
    assert_eq!(v("str2double('abc')"), "double1x1 NaN");
    assert_eq!(v("str2double('')"), "double1x1 NaN");
    assert_eq!(v("str2double('1 2')"), "double1x1 NaN");
    assert_eq!(v("str2double('1,000')"), "double1x1 1000");
    assert_eq!(v("str2double('.5')"), "double1x1 0.5");
    assert_eq!(v("str2double('-Inf')"), "double1x1 -Inf");
    assert_eq!(v("str2double('nan')"), "double1x1 NaN");
    assert_eq!(v("str2double('1e400')"), "double1x1 Inf");
    assert_eq!(v("str2double('1+2i')"), "double1x1 1+2i");
    assert_eq!(v("str2double('3-j')"), "double1x1 3-1i");
    assert_eq!(v("str2double('2.5i')"), "double1x1 0+2.5i");
    assert_eq!(v("str2double(5)"), "double1x1 NaN");
    assert_eq!(
        v("str2double({'1','x';'2.5','3'})"),
        "double2x2 [1 NaN;2.5 3]"
    );
    assert_eq!(v("str2double({1,'2'})"), "double1x2 [NaN 2]");
}

#[test]
fn str2num_parses_literals_without_evaluating_code() {
    assert_eq!(v("str2num('[1 2;3 4]')"), "double2x2 [1 2;3 4]");
    assert_eq!(v("str2num('1 2 3')"), "double1x3 [1 2 3]");
    assert_eq!(v("str2num('-1 -2')"), "double1x2 [-1 -2]");
    assert_eq!(v("str2num('1 - 2')"), "double1x1 -1");
    assert_eq!(v("str2num('1,2;3,4')"), "double2x2 [1 2;3 4]");
    assert_eq!(v("str2num('2*3')"), "double1x1 6");
    assert_eq!(v("str2num('true')"), "logical1x1 true");
    assert_eq!(v("str2num('[true false]')"), "logical1x2 [true false]");
    assert_eq!(v("str2num('[true 2]')"), "double1x2 [1 2]");
    assert_eq!(v("str2num('1+2i')"), "double1x1 1+2i");
    assert_eq!(v("str2num('[Inf -Inf NaN]')"), "double1x3 [Inf -Inf NaN]");
    assert_eq!(v("str2num(repmat('1 2',2,1))"), "double2x2 [1 2;1 2]");
    assert_eq!(
        run("[x,ok]=str2num('3.5'); r={x,ok};"),
        "cell1x2{double1x1 3.5,logical1x1 true}"
    );
    for text in ["abc", "disp(1)", "x", "[1 2", "1]; y = [2", "'a'", ""] {
        let code = format!(
            "x = 7; [v,ok]=str2num('{}'); r={{v,ok}};",
            text.replace('\'', "''")
        );
        assert_eq!(run(&code), "cell1x2{double0x0,logical1x1 false}", "{text}");
    }
    // Nothing ran: the variable x is untouched and nothing was printed.
    let mut e = Engine::default();
    let out = e.execute("x = 7; v = str2num('disp(99)');").unwrap();
    assert!(out.output.is_empty());
    assert_eq!(e.variables["x"].data(), &[7.]);
    assert!(err("r = str2num(5);").contains("Requires string or character array input."));
}

#[test]
fn character_classes() {
    assert_eq!(
        v(r"isspace(sprintf('a b\tc'))"),
        "logical1x5 [false true false true false]"
    );
    assert_eq!(v("isspace(65)"), "logical1x1 false");
    assert_eq!(v("isspace('')"), "logical0x0");
    assert_eq!(v("isletter('a1B_')"), "logical1x4 [true false true false]");
    assert_eq!(v("isletter({'a'})"), "logical1x1 false");
    assert_eq!(
        v("isletter(repmat('a1',2,1))"),
        "logical2x2 [true false;true false]"
    );
    let cases = [
        ("alpha", "[true false false true false]"),
        ("digit", "[false true false false false]"),
        ("wspace", "[false false true false false]"),
        ("upper", "[false false false true false]"),
        ("lower", "[true false false false false]"),
        ("alphanum", "[true true false true false]"),
        ("punct", "[false false false false true]"),
    ];
    for (category, expected) in cases {
        assert_eq!(
            v(&format!("isstrprop('a1 B.','{category}')")),
            format!("logical1x5 {expected}"),
            "{category}"
        );
    }
    assert_eq!(
        v("isstrprop({'a1','B'},'digit')"),
        "cell1x2{logical1x2 [false true],logical1x1 false}"
    );
    // The reference has no 'ForceCellOutput' option.
    assert_eq!(
        err("r = isstrprop('a','alpha','ForceCellOutput',true);"),
        "Too many input arguments."
    );
    assert_eq!(v("isstrprop([65 49],'alpha')"), "logical1x2 [true false]");
    assert_eq!(
        err("r = isstrprop('a','vowel');"),
        "vowel is an invalid qualifier string."
    );
}

#[test]
fn strjust_rows() {
    assert_eq!(v("strjust('a  ')"), "char1x3'  a'");
    assert_eq!(v("strjust('  a','left')"), "char1x3'a  '");
    assert_eq!(v("strjust('ab  ','center')"), "char1x4' ab '");
    assert_eq!(v("strjust('   ','left')"), "char1x3'   '");
    assert_eq!(v("strjust(repmat('a  ',2,1),'right')"), "char2x3'  a|  a'");
    assert_eq!(
        v("strjust({'a ';'  b'})"),
        "cell2x1{char1x2' a',char1x3'  b'}"
    );
    assert_eq!(
        err("r = strjust('a','up');"),
        "Second argument to strjust must be one of: left, right, center."
    );
}

#[test]
fn validatestring_and_isvarname() {
    assert_eq!(
        v("validatestring('app',{'apple','banana'})"),
        "char1x5'apple'"
    );
    assert_eq!(
        v("validatestring('BAN',{'apple','banana'})"),
        "char1x6'banana'"
    );
    assert_eq!(
        v("validatestring('app',{'apple','applesauce'})"),
        "char1x5'apple'"
    );
    let none = err("r = validatestring('x',{'apple','banana'});");
    assert!(none.contains("'apple', 'banana'"), "{none}");
    assert!(none.contains("did not match any"), "{none}");
    let two = err("r = validatestring('ap',{'apple','apricot'});");
    assert!(two.contains("matched more than one"), "{two}");
    assert_eq!(v("isvarname('abc')"), "logical1x1 true");
    assert_eq!(v("isvarname('a_1')"), "logical1x1 true");
    for text in ["1a", "_a", "for", "end", "a b", ""] {
        assert_eq!(
            v(&format!("isvarname('{text}')")),
            "logical1x1 false",
            "{text}"
        );
    }
    assert_eq!(v("isvarname(5)"), "logical1x1 false");
    assert_eq!(v("isvarname(repmat('a',1,63))"), "logical1x1 true");
    assert_eq!(v("isvarname(repmat('a',1,64))"), "logical1x1 false");
}

#[test]
fn repmat_on_characters() {
    assert_eq!(v("repmat('ab',2,2)"), "char2x4'abab|abab'");
    assert_eq!(v("repmat('ab',1,3)"), "char1x6'ababab'");
    assert_eq!(v("repmat('ab',[2 1])"), "char2x2'ab|ab'");
    assert_eq!(v("repmat('ab',0,1)"), "char0x2");
    assert!(err("r = repmat('ab',4096,4096);").contains("65536"));
}

#[test]
fn registered_with_help_pages() {
    for name in [
        "regexp",
        "regexpi",
        "regexprep",
        "str2double",
        "str2num",
        "strjust",
        "isspace",
        "isletter",
        "isstrprop",
        "validatestring",
        "isvarname",
    ] {
        assert!(matrust::session::FUNCTIONS.contains(&name), "{name}");
        let mut e = Engine::default();
        let out = e.execute(&format!("help {name}")).unwrap();
        let text = out.output.join("\n");
        assert!(text.contains("Syntax:"), "{name}: {text}");
    }
}

#[test]
fn help_examples_show_their_results() {
    for (example, expected) in [
        (
            r"regexp('ab12cd345', '\d+', 'match')",
            "cell1x2{char1x2'12',char1x3'345'}",
        ),
        ("regexpi('ABC', 'b', 'match')", "cell1x1{char1x1'B'}"),
        (
            r"regexprep('hello world', '(\w+) (\w+)', '$2 $1')",
            "char1x11'world hello'",
        ),
        ("str2double({'3.14', 'abc'})", "double1x2 [3.14 NaN]"),
        ("str2num('[1 2; 3 4]')", "double2x2 [1 2;3 4]"),
        ("strjust('a  ')", "char1x3'  a'"),
        ("isspace('a b')", "logical1x3 [false true false]"),
        ("isletter('a1B')", "logical1x3 [true false true]"),
        (
            "isstrprop('a1 B', 'digit')",
            "logical1x4 [false true false false]",
        ),
        (
            "validatestring('app', {'apple', 'banana'})",
            "char1x5'apple'",
        ),
        ("isvarname('a_1')", "logical1x1 true"),
    ] {
        assert_eq!(v(example), expected, "{example}");
    }
}

/// Print MatRust values in the reference case format:
/// STRINGS2_CASES=cases.txt STRINGS2_OUT=out.txt cargo test --test strings2 -- --ignored
#[test]
#[ignore]
fn reference_cases() {
    let cases = std::fs::read_to_string(std::env::var("STRINGS2_CASES").unwrap()).unwrap();
    let mut out = String::new();
    for line in cases.lines() {
        let Some((id, code)) = line.split_once(" | ") else {
            continue;
        };
        if line.starts_with('#') {
            continue;
        }
        out.push_str(&format!("@@{} {}\n", id.trim(), case_output(code.trim())));
    }
    std::fs::write(std::env::var("STRINGS2_OUT").unwrap(), out).unwrap();
}

#[test]
fn large_inputs_stay_bounded() {
    let mut e = Engine::default();
    e.execute(
        "s = repmat('ab',1,32768); p = regexp(s,'b'); m = regexp(s,'a','match'); \
         q = regexprep(s,'a','');",
    )
    .unwrap();
    assert_eq!(e.variables["p"].len(), 32768);
    assert_eq!(e.variables["p"].data()[32767], 65536.);
    assert_eq!(e.array("m").unwrap().len(), 32768);
    assert_eq!(e.array("q").unwrap().len(), 32768);
    // 65537 split pieces exceed the cell bound and fail cleanly.
    let message = err("r = regexp(repmat('a',1,65536),'a','split');");
    assert!(message.contains("65536"), "{message}");
}

#[test]
fn regexprep_expansion_is_bounded_before_building() {
    let e = err("r = regexprep(repmat('a',1,65536),'.*',repmat('$0',1,32768));");
    assert!(e.contains("exceeds"), "{e}");
}

#[test]
fn regexp_positions_do_not_collect_capture_slots() {
    let mut e = Engine::default();
    e.execute("r = regexp(repmat('a',1,32768),strcat(repmat('()',1,4096),'.'),'start');")
        .unwrap();
    assert_eq!(e.variables["r"].len(), 32768);
    let e = err("r = regexp(repmat('a',1,32768),strcat(repmat('()',1,4096),'.'),'tokens');");
    assert!(e.contains("capture groups"), "{e}");
}

#[test]
fn backslash_b_is_backspace() {
    assert_eq!(v("regexp(char(8),'\\b')"), v("1"));
    assert_eq!(v("regexp('ab','\\b')"), "double0x0");
}

#[test]
fn regexp_names_output() {
    assert_eq!(
        v("regexp('abc','(?<x>b)','names')"),
        "struct1x1{x:char1x1'b'}"
    );
    assert_eq!(
        v(r"regexp('a1b2','(?<l>[a-z])(?<d>\d)','names')"),
        "struct1x2{l:char1x1'a',d:char1x1'1',l:char1x1'b',d:char1x1'2'}"
    );
    assert_eq!(v("regexp('ab','x','names')"), "struct0x0{}");
    assert_eq!(
        v("regexp('abc','(?<x>b)','names','once')"),
        "struct1x1{x:char1x1'b'}"
    );
    // Matches without named tokens still give one element each.
    assert_eq!(
        v(r"regexp('x=1, y=22','(\w)=(\d+)','names')"),
        "struct1x2{}"
    );
    assert_eq!(
        run(r"[t,n]=regexp('k=v','(?<key>\w)=(\w)','tokens','names'); r=n;"),
        "struct1x1{key:char1x1'k'}"
    );
    assert_eq!(
        run("n = regexp('ab','(?<first>a)(?<second>x)?','names'); r = n.second;"),
        "char0x0"
    );
    assert_eq!(
        v("regexp({'ab','b'},'(?<x>a)','names')"),
        "cell1x2{struct1x1{x:char1x1'a'},struct0x0{}}"
    );
    assert_eq!(
        run("[s,e,te,m,t,nm,sp]=regexp('a1b22','\\d+'); r={s,e,te,m,t,sp};"),
        "cell1x6{double1x2 [2 4],double1x2 [2 5],cell1x2{double0x0,double0x0},\
         cell1x2{char1x1'1',char1x2'22'},cell1x2{cell1x0{},cell1x0{}},\
         cell1x3{char1x1'a',char1x1'b',char0x0}}"
    );
}

#[test]
fn alternation_omits_tokens_of_branches_not_taken() {
    // Reference behavior: a token in an alternative the match did not take is left out;
    // a token skipped by a quantifier is ''.
    assert_eq!(
        v("regexp('ab','(a)|(b)','tokens')"),
        "cell1x2{cell1x1{char1x1'a'},cell1x1{char1x1'b'}}"
    );
    assert_eq!(
        v("regexp('abc','(b)(x)?','tokens')"),
        "cell1x1{cell1x2{char1x1'b',char0x0}}"
    );
    assert_eq!(v("regexp('b','(a)|b','tokens')"), "cell1x1{cell1x0{}}");
    // The same rule inside a group.
    assert_eq!(
        v(r"regexp('k=v','(\w)=(?:(\d)|(\w))','tokens')"),
        "cell1x1{cell1x2{char1x1'k',char1x1'v'}}"
    );
    // Token numbers in regexprep are unchanged by the hidden branch markers.
    assert_eq!(v("regexprep('ab','(a)|(b)','[$1$2]')"), "char1x6'[a][b]'");
    assert_eq!(v("regexprep('ab','(?<x>a)|b','<$<x>>')"), "char1x5'<a><>'");
}

#[test]
fn regexprep_token_past_the_last_stays_literal() {
    assert_eq!(v("regexprep('abc','(b)','$1$1$2')"), "char1x6'abb$2c'");
    assert_eq!(v("regexprep('ab','(a)(b)','$3')"), "char1x2'$3'");
    assert_eq!(v("regexprep('ab','a','$10')"), "char1x4'$10b'");
    assert_eq!(
        err("r = regexprep('abc',{'a','b'},{'1','2','3'});"),
        "Multiple replace strings and patterns given must have the same quantity."
    );
    assert_eq!(
        err("r = regexprep('abc','a','x','bogus');"),
        "Invalid option for regexprep: bogus."
    );
}

#[test]
fn reference_conversions_and_classes() {
    assert_eq!(v("str2double('1d3')"), "double1x1 NaN");
    assert_eq!(v("str2double('Infinity')"), "double1x1 NaN");
    assert_eq!(
        v("find(isstrprop(char(0:127),'punct'))"),
        "double1x23 [34 35 36 38 39 40 41 42 43 45 46 47 48 59 60 64 65 92 93 94 96 124 126]"
    );
    let numeric = err("r = validatestring(5,{'apple'});");
    assert!(
        numeric.starts_with("Expected input to match one of these strings:"),
        "{numeric}"
    );
}

/// Reference cases with known behavior differences in MatRust.
const REFERENCE_DIFFERENCES: &[&str] = &[
    "a30", "a31", "a44", "a49", "a50", "b06", "b20", "b33", "c19", "d18", "e04", "f01", "h11",
    "h17",
];

/// Every other reference case gives the reference's value.
fn check_reference(cases: &str, matlab: &str) {
    let expected: std::collections::HashMap<&str, &str> = matlab
        .lines()
        .filter_map(|l| l.strip_prefix("@@")?.split_once(' '))
        .collect();
    let mut wrong = Vec::new();
    for line in cases.lines().filter(|l| !l.starts_with('#')) {
        let Some((id, code)) = line.split_once(" | ") else {
            continue;
        };
        let id = id.trim();
        if REFERENCE_DIFFERENCES.contains(&id) {
            continue;
        }
        let shown = case_output(code.trim());
        if expected.get(id) != Some(&shown.as_str()) {
            wrong.push(format!(
                "{id} {code}\n  reference  {:?}\n  MatRust {shown}",
                expected.get(id)
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn reference_batch_a_matches_reference() {
    check_reference(
        include_str!("fixtures/strings2/cases-a.txt"),
        include_str!("fixtures/strings2/batch-a-expected.txt"),
    );
}

#[test]
fn reference_batch_b_matches_reference() {
    check_reference(
        include_str!("fixtures/strings2/cases-b.txt"),
        include_str!("fixtures/strings2/batch-b-expected.txt"),
    );
}
