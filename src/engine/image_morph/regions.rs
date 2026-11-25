//! Connected components (bwlabel, bwconncomp, labelmatrix) and region
//! measurements (regionprops).
use super::morph::dirs;
use super::{
    Cx, Img, check_elements, connectivity, double, image, image_2d, mask_2d, numbers, text_of,
    value,
};
use crate::engine::Value;
use crate::{Array, CellArray, Matrix, NumericClass, Result, StructArray, TypedArray};
use std::sync::Arc;

/// Labels 1..n in the column-major order of each object's first pixel:
/// pixels are scanned in linear-index order and each unlabelled foreground
/// pixel starts a breadth-first fill (Rosenfeld & Pfaltz 1966 give the
/// same partition).
pub(super) fn label(
    cx: &Cx,
    rows: usize,
    cols: usize,
    fg: &[bool],
    conn: u8,
) -> Result<(Vec<u32>, usize)> {
    let mut labels = vec![0u32; rows * cols];
    let mut n = 0u32;
    let mut stack = Vec::new();
    let mut visits = 0usize;
    for start in 0..rows * cols {
        if !fg[start] || labels[start] != 0 {
            continue;
        }
        n += 1;
        labels[start] = n;
        stack.push(start);
        while let Some(p) = stack.pop() {
            visits += 1;
            cx.poll(visits)?;
            let (r, c) = ((p % rows) as isize, (p / rows) as isize);
            for &(dr, dc) in dirs(conn) {
                let (rr, cc) = (r + dr, c + dc);
                if rr < 0 || cc < 0 || rr >= rows as isize || cc >= cols as isize {
                    continue;
                }
                let q = cc as usize * rows + rr as usize;
                if fg[q] && labels[q] == 0 {
                    labels[q] = n;
                    stack.push(q);
                }
            }
        }
    }
    Ok((labels, n as usize))
}

fn conn_arg(cx: &Cx, args: &[Value], at: usize) -> Result<u8> {
    match args.get(at) {
        None => Ok(8),
        Some(v) => connectivity(cx, v),
    }
}

pub(super) fn bwlabel(cx: &Cx, args: &[Value]) -> Result<Vec<Value>> {
    if args.len() > 2 {
        return cx.err("expected bwlabel(BW) or bwlabel(BW, conn)");
    }
    let (rows, cols, fg) = mask_2d(cx, &args[0])?;
    let conn = conn_arg(cx, args, 1)?;
    let (labels, n) = label(cx, rows, cols, &fg, conn)?;
    let data = labels.iter().map(|&l| f64::from(l)).collect();
    Ok(vec![
        double(rows, cols, data)?,
        Value::Matrix(Matrix::scalar(n as f64)),
    ])
}

/// Pixel lists (0-based linear indices, ascending) of every object.
fn objects(labels: &[u32], n: usize) -> Vec<Vec<usize>> {
    let mut lists = vec![Vec::new(); n];
    for (p, &l) in labels.iter().enumerate() {
        if l != 0 {
            lists[l as usize - 1].push(p);
        }
    }
    lists
}

fn index_column(list: &[usize]) -> Result<Array> {
    let data = list.iter().map(|&p| (p + 1) as f64).collect();
    Ok(Array::Numeric(Matrix::from_column_vec(
        list.len(),
        1,
        data,
    )?))
}

fn conncomp_value(
    cx: &Cx,
    rows: usize,
    cols: usize,
    conn: u8,
    lists: &[Vec<usize>],
) -> Result<Value> {
    let cells = lists
        .iter()
        .map(|l| index_column(l).map(Arc::new))
        .collect::<Result<Vec<_>>>()?;
    let cell = CellArray::from_shared(1, lists.len(), cells, cx.cancel)?;
    let names = ["Connectivity", "ImageSize", "NumObjects", "PixelIdxList"]
        .map(String::from)
        .to_vec();
    let slots = vec![
        Arc::new(Array::Numeric(Matrix::scalar(f64::from(conn)))),
        Arc::new(Array::Numeric(Matrix::from_column_vec(
            1,
            2,
            vec![rows as f64, cols as f64],
        )?)),
        Arc::new(Array::Numeric(Matrix::scalar(lists.len() as f64))),
        Arc::new(Array::Cell(cell)),
    ];
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        1, 1, names, slots, cx.cancel,
    )?)))
}

pub(super) fn bwconncomp(cx: &Cx, args: &[Value]) -> Result<Value> {
    if args.len() > 2 {
        return cx.err("expected bwconncomp(BW) or bwconncomp(BW, conn)");
    }
    let (rows, cols, fg) = mask_2d(cx, &args[0])?;
    let conn = conn_arg(cx, args, 1)?;
    let (labels, n) = label(cx, rows, cols, &fg, conn)?;
    conncomp_value(cx, rows, cols, conn, &objects(&labels, n))
}

/// (rows, cols, pixel lists) of a connected-components struct.
fn read_conncomp(cx: &Cx, s: &StructArray) -> Result<(usize, usize, Vec<Vec<usize>>)> {
    let field = |name: &str| -> Result<Arc<Array>> {
        match s.field_index(name) {
            Some(f) if s.len() == 1 => Ok(s.get(f, 0)?.clone()),
            _ => cx.err(format!(
                "the connected components struct needs a {name} field"
            )),
        }
    };
    let size = numbers(
        cx,
        &Value::from(field("ImageSize")?.as_ref().clone()),
        "ImageSize",
    )?;
    let (rows, cols) = match size.as_slice() {
        [r, c] if *r >= 0. && *c >= 0. && r.fract() == 0. && c.fract() == 0. => {
            (*r as usize, *c as usize)
        }
        _ => return cx.err("ImageSize must be [rows cols] (2-D only)"),
    };
    check_elements(cx, rows.saturating_mul(cols))?;
    let list = field("PixelIdxList")?;
    let Array::Cell(cell) = list.as_ref() else {
        return cx.err("PixelIdxList must be a cell array");
    };
    let mut lists = Vec::with_capacity(cell.len());
    for i in 0..cell.len() {
        let v = Value::from(cell.get_linear(i)?.as_ref().clone());
        let mut idx = Vec::new();
        for x in numbers(cx, &v, "PixelIdxList")? {
            if !(x >= 1. && x.fract() == 0. && x <= (rows * cols) as f64) {
                return cx.err("PixelIdxList holds an index outside ImageSize");
            }
            idx.push(x as usize - 1);
        }
        lists.push(idx);
    }
    Ok((rows, cols, lists))
}

pub(super) fn labelmatrix(cx: &Cx, args: &[Value]) -> Result<Value> {
    if args.len() != 1 {
        return cx.err("expected labelmatrix(CC)");
    }
    let Value::Container(Array::Struct(s)) = &args[0] else {
        return cx.err("the input must be a connected components struct (from bwconncomp)");
    };
    let (rows, cols, lists) = read_conncomp(cx, s)?;
    let mut data = vec![0.; rows * cols];
    for (k, list) in lists.iter().enumerate() {
        for &p in list {
            data[p] = (k + 1) as f64;
        }
    }
    // The smallest unsigned class that holds the number of objects.
    let n = lists.len() as u64;
    let class = if n <= 255 {
        Some(NumericClass::Uint8)
    } else if n <= 65535 {
        Some(NumericClass::Uint16)
    } else if n <= u64::from(u32::MAX) {
        Some(NumericClass::Uint32)
    } else {
        None
    };
    Ok(match class {
        Some(c) => Value::Container(Array::Typed(TypedArray::from_reals(
            c,
            rows,
            cols,
            data.into_iter(),
        )?)),
        None => double(rows, cols, data)?,
    })
}

/// The supported properties, in the order 'all' lists them.
const SHAPE: &[&str] = &[
    "Area",
    "Centroid",
    "BoundingBox",
    "PixelIdxList",
    "PixelList",
    "Extent",
    "EquivDiameter",
    "FilledArea",
    "MajorAxisLength",
    "MinorAxisLength",
    "Eccentricity",
    "Orientation",
    "Perimeter",
];
const INTENSITY: &[&str] = &[
    "MeanIntensity",
    "MaxIntensity",
    "MinIntensity",
    "WeightedCentroid",
];
const LATER: &[&str] = &[
    "ConvexHull",
    "ConvexImage",
    "ConvexArea",
    "Solidity",
    "Circularity",
    "EulerNumber",
    "Extrema",
    "FilledImage",
    "Image",
    "SubarrayIdx",
    "PixelValues",
    "MaxFeretProperties",
    "MinFeretProperties",
    "MaxFeretDiameter",
    "MaxFeretAngle",
    "MaxFeretCoordinates",
    "MinFeretDiameter",
    "MinFeretAngle",
    "MinFeretCoordinates",
];

fn property_names(cx: &Cx, args: &[Value], intensity: bool) -> Result<Vec<&'static str>> {
    let mut texts = Vec::new();
    for a in args {
        if let Some(t) = text_of(a) {
            texts.push(t);
        } else if let Value::Container(Array::Cell(c)) = a {
            for i in 0..c.len() {
                match text_of(&Value::from(c.get_linear(i)?.as_ref().clone())) {
                    Some(t) => texts.push(t),
                    None => return cx.err("property names must be text"),
                }
            }
        } else {
            return cx.err("property names must be text or a cell array of text");
        }
    }
    let mut out: Vec<&'static str> = Vec::new();
    let push = |p: &'static str, out: &mut Vec<&'static str>| {
        if !out.contains(&p) {
            out.push(p);
        }
    };
    if texts.is_empty() {
        for p in ["Area", "Centroid", "BoundingBox"] {
            push(p, &mut out);
        }
    }
    for t in texts {
        let lower = t.to_ascii_lowercase();
        match lower.as_str() {
            "basic" => {
                for p in ["Area", "Centroid", "BoundingBox"] {
                    push(p, &mut out);
                }
                continue;
            }
            "all" => {
                for p in SHAPE {
                    push(p, &mut out);
                }
                if intensity {
                    for p in INTENSITY {
                        push(p, &mut out);
                    }
                }
                continue;
            }
            "table" | "struct" => {
                return cx.err("output type arguments ('table', 'struct') are not supported yet; the output is a struct array");
            }
            _ => {}
        }
        if let Some(p) = SHAPE.iter().find(|p| p.eq_ignore_ascii_case(&t)) {
            push(p, &mut out);
        } else if let Some(p) = INTENSITY.iter().find(|p| p.eq_ignore_ascii_case(&t)) {
            if !intensity {
                return cx.err(format!(
                    "the '{p}' property needs an intensity image: regionprops(L, I, '{p}')"
                ));
            }
            push(p, &mut out);
        } else if let Some(p) = LATER.iter().find(|p| p.eq_ignore_ascii_case(&t)) {
            return cx.err(format!("the '{p}' property is not supported yet"));
        } else {
            return cx.err(format!("unknown property '{t}'"));
        }
    }
    Ok(out)
}

/// Normalised second central moments with y pointing up (Haralick &
/// Shapiro 1992): uxx and uyy include the 1/12 variance of a unit pixel.
fn ellipse(xs: &[f64], ys: &[f64]) -> (f64, f64, f64, f64) {
    let n = xs.len() as f64;
    let mx = xs.iter().sum::<f64>() / n;
    let my = ys.iter().sum::<f64>() / n;
    let mut uxx = 0.;
    let mut uyy = 0.;
    let mut uxy = 0.;
    for (x, y) in xs.iter().zip(ys) {
        let dx = x - mx;
        // Row coordinates grow downwards; the moments use y upwards.
        let dy = -(y - my);
        uxx += dx * dx;
        uyy += dy * dy;
        uxy += dx * dy;
    }
    let uxx = uxx / n + 1. / 12.;
    let uyy = uyy / n + 1. / 12.;
    let uxy = uxy / n;
    let common = ((uxx - uyy).powi(2) + 4. * uxy * uxy).sqrt();
    let major = 2. * 2f64.sqrt() * (uxx + uyy + common).sqrt();
    let minor = 2. * 2f64.sqrt() * (uxx + uyy - common).max(0.).sqrt();
    let ecc = if major > 0. {
        2. * ((major / 2.).powi(2) - (minor / 2.).powi(2)).max(0.).sqrt() / major
    } else {
        0.
    };
    let (num, den) = if uyy > uxx {
        (uyy - uxx + common, 2. * uxy)
    } else {
        (2. * uxy, uxx - uyy + common)
    };
    let orientation = if num == 0. && den == 0. {
        0.
    } else {
        (num / den).atan().to_degrees()
    };
    (major, minor, ecc, orientation)
}

/// Outer boundary of the region containing `start` (its first pixel in
/// column-major order), traced clockwise with Moore neighbourhoods, as
/// 0-based (row, col) points of a closed chain (the start is not repeated).
fn trace(mask: &[bool], rows: usize, cols: usize, start: usize) -> Vec<(isize, isize)> {
    // Clockwise in image coordinates (row down): N, NE, E, SE, S, SW, W, NW.
    const MOORE: [(isize, isize); 8] = [
        (-1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
        (1, 0),
        (1, -1),
        (0, -1),
        (-1, -1),
    ];
    let inside = |r: isize, c: isize| {
        r >= 0
            && c >= 0
            && r < rows as isize
            && c < cols as isize
            && mask[c as usize * rows + r as usize]
    };
    let s = ((start % rows) as isize, (start / rows) as isize);
    // The start is the topmost pixel of the leftmost column: its west
    // neighbour is background. `from` is the direction (from the current
    // pixel) of the background pixel the search starts after.
    let mut points = vec![s];
    let mut p = s;
    let mut from = 6usize;
    let mut first_move: Option<((isize, isize), usize)> = None;
    let limit = 4 * rows * cols + 8;
    for _ in 0..limit {
        let mut found = None;
        for k in 1..=8 {
            let d = (from + k) % 8;
            let q = (p.0 + MOORE[d].0, p.1 + MOORE[d].1);
            if inside(q.0, q.1) {
                found = Some((q, d));
                break;
            }
        }
        let Some((q, d)) = found else {
            return points; // an isolated pixel
        };
        // Jacob's stopping rule: back at the start about to repeat the first move.
        if p == s {
            match first_move {
                None => first_move = Some((q, d)),
                Some(m) if m == (q, d) => {
                    points.pop();
                    return points;
                }
                Some(_) => {}
            }
        }
        // The background pixel examined just before q, seen from q.
        let back = MOORE[(d + 7) % 8];
        let b = (p.0 + back.0 - q.0, p.1 + back.1 - q.1);
        from = MOORE.iter().position(|&m| m == b).unwrap_or(0);
        p = q;
        points.push(p);
    }
    points
}

fn perimeter(points: &[(isize, isize)]) -> f64 {
    if points.len() < 2 {
        return 0.;
    }
    let mut total = 0.;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        let (dr, dc) = ((a.0 - b.0).abs(), (a.1 - b.1).abs());
        total += if dr + dc == 2 {
            std::f64::consts::SQRT_2
        } else {
            (dr + dc) as f64
        };
    }
    total
}

/// 0-based (first row, last row, first column, last column) of a region.
fn bounds(list: &[usize], rows: usize) -> (usize, usize, usize, usize) {
    let r0 = list.iter().map(|p| p % rows).min().unwrap_or(0);
    let r1 = list.iter().map(|p| p % rows).max().unwrap_or(0);
    let c0 = list.iter().map(|p| p / rows).min().unwrap_or(0);
    let c1 = list.iter().map(|p| p / rows).max().unwrap_or(0);
    (r0, r1, c0, c1)
}

/// Chain length of the outer boundary of a region, traced inside its
/// bounding box (a mask of the whole image per region would cost
/// regions x pixels). The trace starts at the first pixel in column-major
/// order, the topmost pixel of the leftmost column.
fn region_perimeter(list: &[usize], rows: usize) -> f64 {
    let Some(&start) = list.iter().min() else {
        return 0.;
    };
    let (r0, r1, c0, c1) = bounds(list, rows);
    let (h, w) = (r1 - r0 + 1, c1 - c0 + 1);
    let at = |p: usize| (p / rows - c0) * h + p % rows - r0;
    let mut mask = vec![false; h * w];
    for &p in list {
        mask[at(p)] = true;
    }
    perimeter(&trace(&mask, h, w, at(start)))
}

/// Pixels of a region inside its bounding box with holes filled
/// (4-connected background, the complement of 8-connected objects).
fn filled_area(list: &[usize], rows: usize) -> usize {
    let (r0, r1, c0, c1) = bounds(list, rows);
    let (h, w) = (r1 - r0 + 3, c1 - c0 + 3);
    let mut fg = vec![false; h * w];
    for &p in list {
        fg[(p / rows - c0 + 1) * h + p % rows - r0 + 1] = true;
    }
    let mut seen = vec![false; h * w];
    let mut stack = vec![0usize];
    seen[0] = true;
    let mut outside = 0;
    while let Some(p) = stack.pop() {
        outside += 1;
        let (r, c) = (p % h, p / h);
        let mut visit = |q: usize| {
            if !fg[q] && !seen[q] {
                seen[q] = true;
                stack.push(q);
            }
        };
        if r > 0 {
            visit(p - 1);
        }
        if r + 1 < h {
            visit(p + 1);
        }
        if c > 0 {
            visit(p - h);
        }
        if c + 1 < w {
            visit(p + h);
        }
    }
    h * w - outside
}

fn row(values: &[f64]) -> Result<Array> {
    Ok(Array::Numeric(Matrix::from_column_vec(
        1,
        values.len(),
        values.to_vec(),
    )?))
}

pub(super) fn regionprops(cx: &Cx, args: &[Value]) -> Result<Value> {
    if text_of(&args[0]).is_some_and(|t| t.eq_ignore_ascii_case("table")) {
        return cx.err("table output is not supported yet; the output is a struct array");
    }
    // Regions from a connected-components struct, a binary image (8-connected
    // objects) or a label matrix (region k = pixels equal to k).
    let (rows, cols, lists) = match &args[0] {
        Value::Container(Array::Struct(s)) => read_conncomp(cx, s)?,
        Value::Logical(_) => {
            let (rows, cols, fg) = mask_2d(cx, &args[0])?;
            let (labels, n) = label(cx, rows, cols, &fg, 8)?;
            (rows, cols, objects(&labels, n))
        }
        v => {
            let img = image_2d(cx, v)?;
            let mut max = 0usize;
            for &x in &img.data {
                if !(x >= 0. && x.fract() == 0.) {
                    return cx.err("a label matrix must hold nonnegative integers");
                }
                max = max.max(x as usize);
            }
            if max > 65536 {
                return cx.err("at most 65536 regions are supported");
            }
            let labels: Vec<u32> = img.data.iter().map(|&x| x as u32).collect();
            (img.rows(), img.cols(), objects(&labels, max))
        }
    };
    let (intensity, rest) = match args.get(1) {
        Some(v) if text_of(v).is_none() && !matches!(v, Value::Container(Array::Cell(_))) => {
            let img = image(cx, v)?;
            if img.dims != [rows, cols] {
                return cx
                    .err("the intensity image must be 2-D and the same size as the label image");
            }
            (Some(img), &args[2..])
        }
        _ => (None, &args[1..]),
    };
    let props = property_names(cx, rest, intensity.is_some())?;
    let n = lists.len();
    let mut slots: Vec<Arc<Array>> = Vec::with_capacity(n * props.len());
    for list in &lists {
        // One poll per region: a region costs at most its bounding box.
        cx.check()?;
        let xs: Vec<f64> = list.iter().map(|p| (p / rows + 1) as f64).collect();
        let ys: Vec<f64> = list.iter().map(|p| (p % rows + 1) as f64).collect();
        let area = list.len() as f64;
        let bbox = if list.is_empty() {
            [0.5, 0.5, 0., 0.]
        } else {
            let x0 = xs.iter().copied().fold(f64::INFINITY, f64::min);
            let x1 = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let y0 = ys.iter().copied().fold(f64::INFINITY, f64::min);
            let y1 = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            [x0 - 0.5, y0 - 0.5, x1 - x0 + 1., y1 - y0 + 1.]
        };
        let shape = (!list.is_empty()).then(|| ellipse(&xs, &ys));
        for p in &props {
            let a: Array = match *p {
                "Area" => Array::Numeric(Matrix::scalar(area)),
                "Centroid" => {
                    if list.is_empty() {
                        row(&[f64::NAN, f64::NAN])?
                    } else {
                        row(&[xs.iter().sum::<f64>() / area, ys.iter().sum::<f64>() / area])?
                    }
                }
                "BoundingBox" => row(&bbox)?,
                "PixelIdxList" => index_column(list)?,
                "PixelList" => {
                    let mut data = xs.clone();
                    data.extend(&ys);
                    Array::Numeric(Matrix::from_column_vec(list.len(), 2, data)?)
                }
                "Extent" => Array::Numeric(Matrix::scalar(area / (bbox[2] * bbox[3]))),
                "EquivDiameter" => {
                    Array::Numeric(Matrix::scalar((4. * area / std::f64::consts::PI).sqrt()))
                }
                "FilledArea" => Array::Numeric(Matrix::scalar(if list.is_empty() {
                    0.
                } else {
                    filled_area(list, rows) as f64
                })),
                "MajorAxisLength" => scalar_or_empty(shape.map(|s| s.0)),
                "MinorAxisLength" => scalar_or_empty(shape.map(|s| s.1)),
                "Eccentricity" => scalar_or_empty(shape.map(|s| s.2)),
                "Orientation" => scalar_or_empty(shape.map(|s| s.3)),
                "Perimeter" => Array::Numeric(Matrix::scalar(region_perimeter(list, rows))),
                "MeanIntensity" | "MaxIntensity" | "MinIntensity" | "WeightedCentroid" => {
                    let img = intensity.as_ref().expect("checked");
                    intensity_property(cx, p, img, list, &xs, &ys)?
                }
                _ => unreachable!("checked property"),
            };
            slots.push(Arc::new(a));
        }
    }
    let names = props.iter().map(|p| p.to_string()).collect();
    Ok(Value::Container(Array::Struct(StructArray::from_slots(
        n, 1, names, slots, cx.cancel,
    )?)))
}

fn scalar_or_empty(v: Option<f64>) -> Array {
    Array::Numeric(match v {
        Some(v) => Matrix::scalar(v),
        None => Matrix::filled(0, 0, 0.).expect("empty"),
    })
}

fn intensity_property(
    cx: &Cx,
    p: &str,
    img: &Img,
    list: &[usize],
    xs: &[f64],
    ys: &[f64],
) -> Result<Array> {
    let values: Vec<f64> = list.iter().map(|&q| img.data[q]).collect();
    Ok(match p {
        "MeanIntensity" => Array::Numeric(Matrix::scalar(if values.is_empty() {
            f64::NAN
        } else {
            values.iter().sum::<f64>() / values.len() as f64
        })),
        "WeightedCentroid" => {
            let total: f64 = values.iter().sum();
            let x = xs.iter().zip(&values).map(|(x, v)| x * v).sum::<f64>() / total;
            let y = ys.iter().zip(&values).map(|(y, v)| y * v).sum::<f64>() / total;
            row(&[x, y])?
        }
        _ => {
            // Max/Min keep the class of the intensity image. Like MATLAB
            // max/min they ignore NaN unless every value is NaN: the fold
            // starts from NaN, which the first value replaces.
            let pick = if p == "MaxIntensity" {
                values
                    .iter()
                    .copied()
                    .fold(f64::NAN, |a, b| if b > a || a.is_nan() { b } else { a })
            } else {
                values
                    .iter()
                    .copied()
                    .fold(f64::NAN, |a, b| if b < a || a.is_nan() { b } else { a })
            };
            let one = Img {
                dims: if values.is_empty() {
                    vec![0, 0]
                } else {
                    vec![1, 1]
                },
                data: if values.is_empty() {
                    vec![]
                } else {
                    vec![pick]
                },
                kind: img.kind,
            };
            match value(cx, one)? {
                Value::Matrix(m) => Array::Numeric(m),
                Value::Logical(l) => Array::Logical(l),
                Value::Container(a) => a,
                _ => unreachable!(),
            }
        }
    })
}
