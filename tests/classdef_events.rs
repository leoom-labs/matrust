//! classdef events, listeners, enumerations and metaclass. Behaviour from the
//! MATLAB documentation (events, addlistener, notify, enumeration,
//! metaclass); display and error texts are this project's own.
use matrust::Engine;
use std::fs;

fn folder(files: &[(&str, &str)]) -> (tempfile::TempDir, Engine) {
    let dir = tempfile::tempdir().unwrap();
    for (name, text) in files {
        fs::write(dir.path().join(name), text).unwrap();
    }
    let mut e = Engine::default();
    e.set_function_directory(dir.path()).unwrap();
    (dir, e)
}
fn output(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(out) => out.output.join("\n"),
        Err(error) => panic!("{code}\n{error}"),
    }
}
fn error(e: &mut Engine, code: &str) -> String {
    match e.execute(code) {
        Ok(out) => panic!("{code} succeeded: {:?}", out.output),
        Err(error) => error,
    }
}
fn scalar(e: &mut Engine, name: &str) -> f64 {
    output(e, &format!("disp(double({name}))"))
        .trim()
        .parse()
        .unwrap()
}

const SENSOR: &str = "classdef Sensor < handle
  properties
    Value = 0
  end
  events
    Changed
    Reset
  end
  methods
    function set(obj, v)
      obj.Value = v;
      notify(obj, 'Changed');
    end
  end
end
";
const PROBE: &str = "classdef Probe < Sensor
  events
    Alarm
  end
end
";
const COLOR: &str = "classdef Color
  properties
    R
    G
    B
  end
  methods
    function c = Color(r, g, b)
      c.R = r; c.G = g; c.B = b;
    end
  end
  enumeration
    Red (1, 0, 0)
    Green (0, 1, 0)
    Blue (0, 0, 1)
  end
end
";
const DAY: &str = "classdef Day
  enumeration
    Mon, Tue
    Wed
  end
end
";

#[test]
fn listeners_run_in_order_with_source_and_event_name() {
    let (_dir, mut e) = folder(&[("Sensor.m", SENSOR)]);
    let text = output(
        &mut e,
        "s = Sensor(); log = {};
         lh = addlistener(s, 'Changed', @(src, evt) fprintf('A %g %s\\n', src.Value, evt.EventName));
         addlistener(s, 'Changed', @(src, evt) fprintf('B\\n'));
         addlistener(s, 'Reset', @(src, evt) fprintf('R %d\\n', evt.Source == s));
         s.set(5); notify(s, 'Reset'); n = lh.EventName;",
    );
    assert_eq!(text, "A 5 Changed\nB\nR 1");
    assert_eq!(e.execute("disp(n)").unwrap().output.join(""), "Changed");
}

#[test]
fn notify_passes_struct_data_and_checks_the_event() {
    let (_dir, mut e) = folder(&[("Sensor.m", SENSOR)]);
    let text = output(
        &mut e,
        "s = Sensor(); addlistener(s, 'Changed', @(src, evt) fprintf('%g\\n', evt.Level));
         notify(s, 'Changed', struct('Level', 7));",
    );
    assert_eq!(text, "7");
    assert!(error(&mut e, "addlistener(s, 'Gone', @(a, b) 1)").contains("no event named 'Gone'"));
    assert!(error(&mut e, "notify(s, 'Gone')").contains("no event named 'Gone'"));
    assert!(error(&mut e, "addlistener(s, 'Changed', 3)").contains("function handle"));
    // A handle class without a delete method: delete(s) as a statement deletes
    // the object (it is not the file-deleting delete).
    e.execute("delete(s)").unwrap();
    assert_eq!(output(&mut e, "disp(isvalid(s))").trim(), "0");
    assert!(error(&mut e, "notify(s, 'Changed')").contains("Invalid or deleted object"));
}

#[test]
fn events_lists_own_then_inherited_and_needs_a_handle_class() {
    let (_dir, mut e) = folder(&[
        ("Sensor.m", SENSOR),
        ("Probe.m", PROBE),
        ("Val.m", "classdef Val\n  events\n    E\n  end\nend\n"),
    ]);
    output(&mut e, "c = events('Probe'); p = Probe(); d = events(p);");
    assert_eq!(
        output(
            &mut e,
            "fprintf('%s ', c{:}); fprintf('| %d', isequal(c, d))"
        ),
        "Alarm Changed Reset | 1"
    );
    assert_eq!(
        output(&mut e, "events('Sensor')"),
        "Events for class Sensor:\n\n    Changed\n    Reset\n"
    );
    // Inherited events reach listeners of the subclass.
    assert_eq!(
        output(
            &mut e,
            "addlistener(p, 'Changed', @(s, v) disp(class(s))); p.set(1);"
        ),
        "Probe"
    );
    assert!(error(&mut e, "Val()").contains("only handle classes can define events"));
}

#[test]
fn enumeration_members_compare_and_convert() {
    let (_dir, mut e) = folder(&[("Color.m", COLOR), ("Day.m", DAY)]);
    output(
        &mut e,
        "c = Color.Red; r = c.R; g = Color.Green.G; same = c == Color.Red; diff = c ~= Color.Blue;
         byname = c == 'Red'; t = Day.Wed; [m, names] = enumeration('Color'); k = numel(m);
         b = m{3}.B; name3 = names{3};",
    );
    assert_eq!(scalar(&mut e, "r"), 1.);
    assert_eq!(scalar(&mut e, "g"), 1.);
    assert_eq!(scalar(&mut e, "same"), 1.);
    assert_eq!(scalar(&mut e, "diff"), 1.);
    assert_eq!(scalar(&mut e, "byname"), 1.);
    assert_eq!(scalar(&mut e, "k"), 3.);
    assert_eq!(scalar(&mut e, "b"), 1.);
    assert_eq!(output(&mut e, "disp(char(t)); disp(name3)"), "Wed\nBlue");
    assert_eq!(
        output(&mut e, "c"),
        "\nc = \n\n  Color enumeration\n\n    Red\n"
    );
    assert_eq!(
        output(&mut e, "enumeration('Day')"),
        "Enumeration members for class 'Day':\n\n    Mon\n    Tue\n    Wed\n"
    );
    assert!(error(&mut e, "Color.Purple").contains("Purple"));
}

#[test]
fn metaclass_describes_the_class() {
    let (_dir, mut e) = folder(&[("Sensor.m", SENSOR), ("Probe.m", PROBE)]);
    output(
        &mut e,
        "mc = metaclass(Probe()); n = mc.Name; h = mc.HandleCompatible;
         ev = {mc.EventList.Name}; pr = {mc.PropertyList.Name}; sup = {mc.SuperclassList.Name};",
    );
    assert_eq!(
        output(&mut e, "fprintf('%s|', n, ev{:}, pr{:}, sup{:})"),
        "Probe|Alarm|Changed|Reset|Value|Sensor|"
    );
    assert_eq!(scalar(&mut e, "h"), 1.);
}

#[test]
fn names_exist() {
    let mut e = Engine::default();
    for name in [
        "events",
        "addlistener",
        "notify",
        "enumeration",
        "metaclass",
    ] {
        let text = output(&mut e, &format!("disp(exist('{name}'))"));
        assert!(text.trim() != "0", "{name}: {text}");
    }
}

#[test]
fn review_fixes_listener_captures_errors_and_isequal() {
    let (_dir, mut e) = folder(&[("Sensor.m", SENSOR), ("Day.m", DAY)]);
    // A value captured only by a listener stays alive across executions.
    output(
        &mut e,
        "s = Sensor(); t = Sensor(); t.Value = 4; addlistener(s, 'Changed', @(a, b) disp(t.Value)); clear t",
    );
    assert_eq!(output(&mut e, "notify(s, 'Changed')").trim(), "4");
    // A failing callback warns; later listeners and the caller go on.
    let (_dir, mut e) = folder(&[("Sensor.m", SENSOR)]);
    let text = output(
        &mut e,
        "s = Sensor(); addlistener(s, 'Reset', @(a, b) error('boom'));
         addlistener(s, 'Reset', @(a, b) disp('second')); notify(s, 'Reset'); disp('after')",
    );
    assert!(
        text.contains(
            "Warning: Error occurred while executing the listener callback for event Reset"
        ),
        "{text}"
    );
    assert!(
        text.contains("boom") && text.contains("second") && text.ends_with("after"),
        "{text}"
    );
    let (_dir, mut e) = folder(&[("Day.m", DAY)]);
    assert_eq!(
        output(&mut e, "disp([isequal(Day.Mon, 'Mon'), isequal('Tue', Day.Tue), isequal(Day.Mon, Day.Tue), isequal(Day.Wed, Day.Wed)])").trim(),
        "1     1     0     1"
    );
}
