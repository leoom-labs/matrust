//! `gscatter(x, y, g, ...)` grouped scatter plot using `table_fns::groups::group_rows`.

use super::{Engine, P2Ctx, Value, p2_on_off, p2_text, p2_vec};
use crate::engine::plot_properties::color as parse_color;
use crate::engine::table_fns::groups::group_rows;
use crate::plot::{LinePattern, MarkerShape, PlotKind, Series};
use crate::{Array, Result};

fn format_num(v: f64) -> String {
    if v == 0. {
        return "0".to_owned();
    }
    if v.fract() == 0. && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    format!("{v}")
}

fn group_label(g: &Array, row: usize) -> Result<String> {
    Ok(match g {
        Array::Numeric(m) => format_num(m.data()[row]),
        Array::Logical(m) => if m.data()[row] != 0 { "true" } else { "false" }.to_owned(),
        Array::Typed(t) if t.is_integer() => format!("{}", t.int(row)),
        Array::Typed(t) => format_num(t.real(row)),
        Array::Categorical(c) => c.text(row).to_owned(),
        Array::Str(s) => s.data()[row].as_deref().unwrap_or("").to_owned(),
        Array::Cell(cell) => match cell.get_linear(row)?.as_ref() {
            Array::Character(ch) => String::from_utf16_lossy(ch.units()),
            _ => String::new(),
        },
        Array::Character(ch) => {
            let (cr, cc) = (ch.rows(), ch.cols());
            let units: Vec<u16> = (0..cc).map(|k| ch.units()[k * cr + row]).collect();
            String::from_utf16_lossy(&units).trim_end().to_owned()
        }
        Array::Duration(d) => format_num(d.data()[row]),
        Array::Datetime(d) => format!("{:?}", d.data()[row]),
        _ => String::new(),
    })
}

fn parse_colors(e: &Engine, value: &Value) -> Result<Vec<[u8; 3]>> {
    if let Some(s) = p2_text(e, value) {
        if s.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(s.len());
        for ch in s.chars() {
            let single = Value::Text(ch.to_string());
            out.push(parse_color(&single)?);
        }
        return Ok(out);
    }
    match value {
        Value::Matrix(m) if m.is_empty() => Ok(Vec::new()),
        Value::Matrix(m) => {
            if m.cols() != 3 {
                return Err("gscatter: clr matrix must have 3 columns".into());
            }
            let mut out = Vec::with_capacity(m.rows());
            for r in 0..m.rows() {
                let rgb = [m.get(r, 0), m.get(r, 1), m.get(r, 2)];
                if rgb
                    .iter()
                    .any(|v| !v.is_finite() || !(0. ..=1.).contains(v))
                {
                    return Err("gscatter: RGB color values must be in [0,1]".into());
                }
                out.push(rgb.map(|v| (v * 255.).round() as u8));
            }
            Ok(out)
        }
        _ => Err("gscatter: clr must be a color string or n-by-3 RGB matrix".into()),
    }
}

fn parse_symbols(e: &Engine, value: &Value) -> Result<Vec<MarkerShape>> {
    if let Some(s) = p2_text(e, value) {
        if s.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::with_capacity(s.len());
        for ch in s.chars() {
            let mut buf = [0u8; 4];
            out.push(MarkerShape::parse(ch.encode_utf8(&mut buf))?);
        }
        return Ok(out);
    }
    match value {
        Value::Matrix(m) if m.is_empty() => Ok(Vec::new()),
        _ => Err("gscatter: sym must be a character vector of marker symbols".into()),
    }
}

fn parse_sizes(value: &Value) -> Result<Vec<f64>> {
    match value {
        Value::Matrix(m) if m.is_empty() => Ok(Vec::new()),
        Value::Matrix(_) => {
            let v = p2_vec(value, "gscatter")?;
            for &s in &v {
                crate::plot::validate_marker_size(s)?;
            }
            Ok(v)
        }
        _ => Err("gscatter: siz must be a numeric scalar or vector".into()),
    }
}

pub(super) fn run(e: &mut Engine, args: &[Value]) -> Result<Value> {
    if args.len() < 3 {
        return Err("gscatter: expected x, y and g".into());
    }
    let x = p2_vec(&args[0], "gscatter")?;
    let y = p2_vec(&args[1], "gscatter")?;
    if x.len() != y.len() {
        return Err("gscatter: x and y must have the same length".into());
    }
    let n = x.len();
    if n > crate::plot::MAX_POINTS {
        return Err("gscatter: input exceeds figure point limit".into());
    }
    e.check_cancel()?;
    let g_array = args[2].clone().array(&e.cancel)?;
    if g_array.cols() == 0 {
        return Err("gscatter: grouping variable must have at least one column".into());
    }
    let groups = group_rows(std::slice::from_ref(&g_array), n, false, "gscatter")?;
    if groups.firsts.len() > crate::plot::MAX_SERIES {
        return Err("gscatter: number of groups exceeds 64-series figure limit".into());
    }

    let rest = &args[3..];
    let nv_start = rest
        .iter()
        .position(|v| p2_text(e, v).is_some_and(|s| s.eq_ignore_ascii_case("doleg")))
        .unwrap_or(rest.len());
    let (pos, nv) = (&rest[..nv_start], &rest[nv_start..]);
    if pos.len() > 4 {
        return Err("gscatter: too many positional arguments".into());
    }
    if !nv.len().is_multiple_of(2) {
        return Err("gscatter: name-value arguments must come in pairs".into());
    }

    let colors = if let Some(v) = pos.first() {
        parse_colors(e, v)?
    } else {
        Vec::new()
    };
    let symbols = if let Some(v) = pos.get(1) {
        parse_symbols(e, v)?
    } else {
        Vec::new()
    };
    let sizes = if let Some(v) = pos.get(2) {
        parse_sizes(v)?
    } else {
        Vec::new()
    };
    let mut doleg = if let Some(v) = pos.get(3) {
        p2_on_off(e, v, "gscatter")?
    } else {
        true
    };
    for pair in nv.chunks(2) {
        let key = p2_text(e, &pair[0]).ok_or("gscatter: property names must be text")?;
        if key.eq_ignore_ascii_case("doleg") {
            doleg = p2_on_off(e, &pair[1], "gscatter")?;
        } else {
            return Err(format!("gscatter: unsupported property '{key}'"));
        }
    }

    let mut figure = e.p2_target();
    let hold = figure.axes.hold;
    let mut ctx = P2Ctx::of(&figure, e.graphics_theme_context);
    let mut batch = Vec::with_capacity(groups.firsts.len());
    for (k, &first_row) in groups.firsts.iter().enumerate() {
        e.check_cancel()?;
        let points: Vec<[f64; 2]> = groups.members[k]
            .iter()
            .map(|&idx| [x[idx], y[idx]])
            .collect();
        let mut s = Series::new(points, PlotKind::Line);
        s.pattern = LinePattern::None;
        s.markers = true;
        s.marker_shape = Some(if symbols.is_empty() {
            MarkerShape::Circle
        } else {
            symbols[k % symbols.len()]
        });
        if !sizes.is_empty() {
            s.marker_size = Some(sizes[k % sizes.len()]);
        }
        if colors.is_empty() {
            let (auto_palette, rgb) = ctx.next_color();
            s.auto_palette = auto_palette;
            s.color = rgb;
            s.explicit_color = false;
        } else {
            s.color = colors[k % colors.len()];
            s.explicit_color = true;
        }
        s.label = group_label(&g_array, first_row)?;
        batch.push(s);
    }

    let first = figure.axes.series.len();
    crate::engine::graphics_handles::dot_syntax::plot_carry_over(
        e.figure.as_ref(),
        &mut figure,
        &mut batch,
    )?;
    figure.add(batch)?;
    if let Some(cycle) = &mut figure.axes.palette_cycle
        && let Some(start) = ctx.palette
    {
        cycle.next_slot = (start.next_slot + ctx.used) % 7;
    }
    if doleg && !groups.firsts.is_empty() {
        figure.axes.legend = true;
    } else if !doleg && !hold {
        figure.axes.legend = false;
    }
    figure.validate()?;
    figure.prepare_legend_layout()?;
    let handles = e.graphics_series_result(&figure, first)?;
    e.check_cancel()?;
    e.figure = Some(figure);
    Ok(handles)
}
