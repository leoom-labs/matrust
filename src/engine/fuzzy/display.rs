use super::*;
fn value(e: &Engine, a: &Array) -> String {
    if let Some(o) = super::super::objects::fis_value(e, &Value::from(a.clone())) {
        return format!("[{}x{} {}]", o.rows, o.cols, o.class);
    }
    if let Ok(s) = string(a) {
        return format!(
            "\"{}{}\"",
            s.chars().take(512).collect::<String>(),
            if s.chars().count() > 512 { "..." } else { "" }
        );
    }
    match a {
        Array::Numeric(m) => {
            let v = m
                .data()
                .iter()
                .take(32)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ");
            if m.len() == 1 {
                v
            } else {
                format!("[{v}{}]", if m.len() > 32 { " ..." } else { "" })
            }
        }
        Array::Logical(m) if m.len() == 1 => m.data()[0].to_string(),
        Array::Handle(h) => format!("@{}", h.func2str()),
        // Nested opaque values are formatted by the parent with known class.
        _ => "[object]".into(),
    }
}
pub(super) fn body(e: &Engine, o: &Object) -> String {
    let fields = properties::names(o.class);
    if o.len() == 1 {
        let p = &o.items[0];
        let mut s = format!("  {} with properties:\n\n", o.class);
        for &key in fields {
            let v = p
                .get(key)
                .map(|v| value(e, v))
                .unwrap_or_else(|| "[object]".into());
            s.push_str(&format!("    {key:>27}: {v}\n"));
        }
        return s;
    }
    let mut s = format!(
        "  {}x{} {} array with properties:\n\n",
        o.rows, o.cols, o.class
    );
    for key in fields {
        s.push_str(&format!("    {key}\n"));
    }
    s.push_str("\n  Details:\n");
    let columns: &[&str] = match o.class {
        "fisvar" => &["Name", "Range", "MembershipFunctions"],
        "fismf" => &["Name", "Type", "Parameters"],
        "fisrule" => &["Description"],
        // Option objects have no Name property. Their arrays still display
        // their size/class and property names above, without fake details.
        _ => &[],
    };
    s.push_str(&format!("           {}\n", columns.join("        ")));
    s.push_str("           ________________________________________\n");
    for (i, p) in o.items.iter().enumerate().take(256) {
        let row = columns
            .iter()
            .map(|k| {
                p.get(*k)
                    .map(|v| value(e, v))
                    .unwrap_or_else(|| "[object]".into())
            })
            .collect::<Vec<_>>()
            .join("    ");
        s.push_str(&format!("    {}    {}\n", i + 1, row));
    }
    if o.len() > 256 {
        s.push_str("    ...\n");
    }
    s
}
