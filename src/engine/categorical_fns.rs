//! Categorical construction, comparisons and collection operations.
use super::datetime_fns::{polled, polled_map, text_elements};
use super::{Engine, Execution, Value};
use crate::syntax::{Expr, Token};
use crate::{
    Array, CategoricalArray, CellArray, CharacterRow, LogicalMatrix, Matrix, Result, StringArray,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub(super) fn of(v: &Value) -> Option<&CategoricalArray> {
    match v {
        Value::Container(Array::Categorical(c)) => Some(c),
        _ => None,
    }
}
pub(super) fn is_cat(v: &Value) -> bool {
    of(v).is_some()
}
fn value(c: CategoricalArray) -> Value {
    Value::Container(Array::Categorical(c))
}
fn numbers(rows: usize, cols: usize, data: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
}
fn logical(e: &Engine, rows: usize, cols: usize, data: Vec<u8>) -> Result<Value> {
    Ok(Value::Logical(LogicalMatrix::from_column_vec(
        rows, cols, data, &e.cancel,
    )?))
}
fn text(e: &Engine, v: &Value) -> Option<String> {
    let (items, _) = text_elements(v, &e.cancel).ok()??;
    if items.len() == 1 {
        items[0].clone()
    } else {
        None
    }
}
fn text_budget(e: &Engine, c: &CategoricalArray, limit: usize) -> Result<()> {
    let mut bytes = 0usize;
    for i in 0..c.len() {
        if i % 65536 == 0 {
            e.check_cancel()?;
        }
        bytes = bytes.saturating_add(c.text(i).len());
        if bytes > limit {
            return Err("categorical: converted text exceeds output byte limit".into());
        }
    }
    Ok(())
}
/// Text elements and their shape.
type Texts = (Vec<Option<String>>, (usize, usize));

fn texts(e: &Engine, v: &Value) -> Result<Texts> {
    if let Some(c) = of(v) {
        text_budget(e, c, 64 * 1024 * 1024)?;
        return Ok((
            polled(e.cancel.as_ref(), c.len(), |i| {
                if c.codes()[i] == 0 {
                    None
                } else {
                    Some(c.text(i).into())
                }
            })?,
            (c.rows(), c.cols()),
        ));
    }
    text_elements(v, &e.cancel)?
        .ok_or_else(|| "categorical: expected text or categorical values".into())
}

#[derive(Clone, Debug, PartialEq)]
enum Key {
    Integer(i128),
    Number(f64),
    Text(String),
    Missing,
}
fn work_budget(n: usize, bytes_per_element: usize) -> Result<()> {
    if n.saturating_mul(bytes_per_element) > 256 * 1024 * 1024 {
        return Err("categorical: operation exceeds 256 MiB workspace budget".into());
    }
    Ok(())
}
fn inputs(e: &Engine, v: &Value) -> Result<(Vec<Key>, Vec<usize>, bool)> {
    let dims = v.dims().into_owned();
    work_budget(dims.iter().product(), 192)?;
    if let Value::Container(Array::Typed(t)) = v
        && t.is_integer()
    {
        return Ok((
            polled(&e.cancel, t.len(), |i| Key::Integer(t.int(i)))?,
            dims,
            true,
        ));
    }
    if matches!(
        v,
        Value::Matrix(_) | Value::Logical(_) | Value::Container(Array::Typed(_))
    ) {
        let m = v.clone().matrix(&e.cancel)?;
        return Ok((
            polled_map(&e.cancel, m.data(), |x| {
                if x.is_nan() {
                    Key::Missing
                } else {
                    Key::Number(x)
                }
            })?,
            dims,
            true,
        ));
    }
    if matches!(v, Value::Container(Array::Character(c)) if c.rows() > 1 || c.is_nd()) {
        return Err("categorical: character input must be a row".into());
    }
    let (t, shape) = texts(e, v)?;
    let keys = t
        .into_iter()
        .map(|s| match s {
            Some(s) if !s.trim().is_empty() && s != "<missing>" && s != "<undefined>" => {
                Key::Text(s.trim().into())
            }
            _ => Key::Missing,
        })
        .collect();
    Ok((keys, vec![shape.0, shape.1], false))
}
#[derive(Hash, PartialEq, Eq)]
enum KeyId {
    Integer(i128),
    Number(u64),
    Text(String),
    Missing,
}
fn key_id(k: &Key) -> KeyId {
    match k {
        Key::Missing => KeyId::Missing,
        Key::Integer(x) => KeyId::Integer(*x),
        Key::Number(x)
            if x.fract() == 0.0 && *x >= i128::MIN as f64 && *x < -(i128::MIN as f64) =>
        {
            KeyId::Integer(*x as i128)
        }
        Key::Text(s) => KeyId::Text(s.clone()),
        Key::Number(x) => KeyId::Number(if *x == 0.0 { 0 } else { x.to_bits() }),
    }
}
fn key_name(k: &Key) -> String {
    match k {
        Key::Text(s) => s.clone(),
        Key::Integer(x) => x.to_string(),
        Key::Number(x) if *x == f64::INFINITY => "Inf".into(),
        Key::Number(x) if *x == f64::NEG_INFINITY => "-Inf".into(),
        // Integer text only inside the i128 window. Larger finite values are
        // integer-valued in f64 (the ulp exceeds 1) and a saturating cast
        // would name them all i128::MAX. %.17g round-trips, so distinct
        // values stay distinct; 1e308 prints as 1e+308, like num2str.
        Key::Number(x)
            if x.fract() == 0.0 && *x >= i128::MIN as f64 && *x < -(i128::MIN as f64) =>
        {
            format!("{}", *x as i128)
        }
        Key::Number(x) => {
            let prec = if x.abs() >= -(i128::MIN as f64) {
                17
            } else {
                // num2str: max(floor(log10|x|), 0) + 5 significant digits.
                let digits = if x.abs() > 0. {
                    x.abs().log10().floor().max(0.) as usize
                } else {
                    0
                };
                (digits + 5).clamp(5, 16)
            };
            super::text_format::shortest_g(*x, prec)
        }
        Key::Missing => String::new(),
    }
}
fn construct(e: &Engine, args: &[Value]) -> Result<Value> {
    if args.is_empty() {
        return Ok(value(CategoricalArray::new(
            0,
            0,
            vec![],
            Arc::new(vec![]),
            false,
        )?));
    }
    if args.len() == 1
        && let Some(c) = of(&args[0])
    {
        return Ok(value(c.clone()));
    }
    let mut end = args.len();
    let mut ordinal = false;
    if end >= 3 && text(e, &args[end - 2]).is_some_and(|s| s.eq_ignore_ascii_case("Ordinal")) {
        let m = args[end - 1].clone().matrix(&e.cancel)?;
        if m.len() != 1 || !matches!(m.data()[0], 0.0 | 1.0) {
            return Err("categorical: Ordinal must be a logical scalar".into());
        }
        ordinal = m.data()[0] != 0.0;
        end -= 2;
    }
    if !(1..=3).contains(&end) {
        return Err("categorical: unsupported arguments; expected A, valueset, catnames and optional Ordinal".into());
    }
    if end == 1
        && let Some(c) = of(&args[0])
    {
        // categorical(C, 'Ordinal', tf) keeps C's categories and their order.
        let dims = c.nd_dims();
        return Ok(value(
            CategoricalArray::new(
                c.rows(),
                c.cols(),
                c.codes().to_vec(),
                c.categories().clone(),
                ordinal,
            )?
            .with_dims(&dims)?,
        ));
    }
    let (keys, shape, numeric) = inputs(e, &args[0])?;
    let vals = if end >= 2 {
        let (v, s, n) = inputs(e, &args[1])?;
        if n != numeric {
            return Err("categorical: valueset must have the same type as input".into());
        }
        if s.len() > 2 || (s[0] > 1 && s[1] > 1) {
            return Err("categorical: valueset must be a vector".into());
        }
        v
    } else {
        let mut indices: Vec<usize> = (0..keys.len())
            .filter(|&i| keys[i] != Key::Missing)
            .collect();
        e.dtm_stable_sort(&mut indices, |i, j| match (&keys[i], &keys[j]) {
            (Key::Integer(x), Key::Integer(y)) => x.cmp(y),
            (Key::Number(x), Key::Number(y)) => x.partial_cmp(y).unwrap(),
            (Key::Text(x), Key::Text(y)) => x.cmp(y),
            _ => std::cmp::Ordering::Equal,
        })?;
        indices.dedup_by(|a, b| keys[*a] == keys[*b]);
        polled(&e.cancel, indices.len(), |i| keys[indices[i]].clone())?
    };
    let mut seen = HashSet::new();
    if vals.iter().any(|v| !seen.insert(key_id(v))) {
        return Err("categorical: valueset must contain unique values".into());
    }
    let names = if end == 3 {
        let (n, s) =
            text_elements(&args[2], &e.cancel)?.ok_or("categorical: catnames must be text")?;
        if (s.0 > 1 && s.1 > 1) || n.len() != vals.len() {
            return Err("categorical: catnames must match valueset".into());
        }
        n.into_iter()
            .map(|s| {
                s.map(|s| s.trim().to_string())
                    .ok_or_else(|| "categorical: category names cannot be missing".into())
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        vals.iter().map(key_name).collect()
    };
    let mut categories = Vec::new();
    let mut lookup = HashMap::new();
    for (k, name) in vals.iter().zip(names) {
        if end < 3 && *k == Key::Missing {
            continue;
        }
        categories.push(name);
        lookup.insert(key_id(k), categories.len() as u32);
    }
    let codes = polled(&e.cancel, keys.len(), |i| {
        lookup.get(&key_id(&keys[i])).copied().unwrap_or(0)
    })?;
    let rows = shape[0];
    let cols = shape[1..].iter().product();
    Ok(value(
        CategoricalArray::new(rows, cols, codes, Arc::new(categories), ordinal)?
            .with_dims(&shape)?,
    ))
}
fn cellstr(e: &Engine, rows: usize, cols: usize, texts: Vec<String>) -> Result<Value> {
    if rows > crate::script_containers::MAX_CELL_SLOTS
        || cols > crate::script_containers::MAX_CELL_SLOTS
        || texts.len() > crate::script_containers::MAX_CELL_SLOTS
    {
        return Err("cellstr: categorical output exceeds 65536 cells".into());
    }
    let cells = texts
        .into_iter()
        .map(|s| CharacterRow::from_text(&s, &e.cancel).map(|c| Arc::new(Array::Character(c))))
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Container(Array::Cell(CellArray::from_shared(
        rows, cols, cells, &e.cancel,
    )?)))
}
pub(super) fn equal(
    a: &CategoricalArray,
    b: &CategoricalArray,
    missing_equal: bool,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<bool> {
    if a.dims() != b.dims()
        || a.is_ordinal() != b.is_ordinal()
        || (a.is_ordinal() && a.categories() != b.categories())
    {
        return Ok(false);
    }
    for (i, &code) in a.codes().iter().enumerate() {
        if i % 65536 == 0 {
            crate::dsp::check_cancel(cancel)?;
        }
        let same = if code == 0 || b.codes()[i] == 0 {
            missing_equal && code == b.codes()[i]
        } else {
            a.text(i) == b.text(i)
        };
        if !same {
            return Ok(false);
        }
    }
    Ok(true)
}
// Borrow categorical labels: comparisons must not expand repeated category text.
enum Labels<'a> {
    Categories(&'a CategoricalArray),
    Text(Vec<Option<String>>, (usize, usize)),
}
impl Labels<'_> {
    fn shape(&self) -> (usize, usize) {
        match self {
            Self::Categories(c) => (c.rows(), c.cols()),
            Self::Text(_, s) => *s,
        }
    }
    fn len(&self) -> usize {
        let (r, c) = self.shape();
        r * c
    }
    fn at(&self, i: usize) -> Option<&str> {
        match self {
            Self::Categories(c) => {
                if c.codes()[i] == 0 {
                    None
                } else {
                    Some(c.text(i))
                }
            }
            Self::Text(t, _) => t[i].as_deref(),
        }
    }
}
fn labels<'a>(e: &Engine, v: &'a Value) -> Result<Labels<'a>> {
    if let Some(c) = of(v) {
        Ok(Labels::Categories(c))
    } else {
        let (t, s) = texts(e, v)?;
        Ok(Labels::Text(t, s))
    }
}
pub(super) fn binary(e: &Engine, op: &Token, a: &Value, b: &Value) -> Result<Value> {
    let ordered = matches!(
        op,
        Token::Less | Token::LessEqual | Token::Greater | Token::GreaterEqual
    );
    if !ordered && !matches!(op, Token::Equal | Token::NotEqual) {
        return Err("categorical: unsupported operator".into());
    }
    let base = of(a)
        .or_else(|| of(b))
        .ok_or("Expected categorical operand")?;
    if ordered && !base.is_ordinal() {
        return Err("categorical: relational comparisons require ordinal arrays".into());
    }
    if let (Some(x), Some(y)) = (of(a), of(b))
        && (x.is_ordinal() != y.is_ordinal()
            || (x.is_ordinal() && x.categories() != y.categories()))
    {
        return Err("categorical: ordinal category sets must match".into());
    }
    let at = labels(e, a)?;
    let bt = labels(e, b)?;
    let (ashape, bshape) = (at.shape(), bt.shape());
    let dim = |x, y| {
        if x == y || y == 1 {
            Ok(x)
        } else if x == 1 {
            Ok(y)
        } else {
            Err("categorical: incompatible sizes")
        }
    };
    let (rows, cols) = (dim(ashape.0, bshape.0)?, dim(ashape.1, bshape.1)?);
    let n = crate::datetime_array::checked_len(rows, cols, "categorical comparison")?;
    let index = |i: usize, s: (usize, usize)| {
        (if s.0 == 1 { 0 } else { i % rows }) + (if s.1 == 1 { 0 } else { i / rows }) * s.0
    };
    let order: HashMap<&str, usize> = base
        .categories()
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), i))
        .collect();
    if ordered {
        for operand in [&at, &bt] {
            for i in 0..operand.len() {
                if i % 65536 == 0 {
                    e.check_cancel()?;
                }
                if operand.at(i).is_some_and(|s| !order.contains_key(s)) {
                    return Err("categorical: text is not an ordinal category".into());
                }
            }
        }
    }
    let data = polled(&e.cancel, n, |i| {
        let (x, y) = (at.at(index(i, ashape)), bt.at(index(i, bshape)));
        let eq = x.is_some() && y.is_some() && x == y;
        (match op {
            Token::Equal => eq,
            Token::NotEqual => !eq,
            _ => match (x.and_then(|s| order.get(s)), y.and_then(|s| order.get(s))) {
                (Some(x), Some(y)) => match op {
                    Token::Less => x < y,
                    Token::LessEqual => x <= y,
                    Token::Greater => x > y,
                    _ => x >= y,
                },
                _ => false,
            },
        }) as u8
    })?;
    logical(e, rows, cols, data)
}

pub(super) fn transpose(e: &Engine, c: &CategoricalArray) -> Result<Value> {
    let codes = polled(&e.cancel, c.len(), |i| {
        c.codes()[(i % c.cols()) * c.rows() + i / c.cols()]
    })?;
    Ok(value(c.with_codes(c.cols(), c.rows(), codes)?))
}

pub(super) fn concatenate(e: &Engine, grid: Vec<Vec<Value>>) -> Result<Value> {
    let base = grid
        .iter()
        .flatten()
        .find_map(of)
        .ok_or("Expected categorical array")?;
    let mut cats = base.categories().as_ref().clone();
    let mut map: HashMap<String, u32> = cats
        .iter()
        .enumerate()
        .map(|(i, s)| (s.clone(), (i + 1) as u32))
        .collect();
    let mut blocks = Vec::new();
    let mut total = 0usize;
    for row in &grid {
        let mut stripe = Vec::new();
        for v in row {
            if *v.dims() == [0, 0] {
                continue;
            }
            let c = of(v).ok_or("categorical: concatenation requires categorical arrays")?;
            if c.is_ordinal() != base.is_ordinal()
                || (base.is_ordinal() && c.categories() != base.categories())
            {
                return Err("categorical: ordinal category sets must match".into());
            }
            let mut remap = vec![0];
            for s in c.categories().iter() {
                let code = *map.entry(s.clone()).or_insert_with(|| {
                    cats.push(s.clone());
                    cats.len() as u32
                });
                remap.push(code);
            }
            total = total
                .checked_add(c.len())
                .ok_or("categorical: size overflow")?;
            crate::datetime_array::checked_len(total, 1, "categorical concatenation")?;
            stripe.push((
                c.rows(),
                c.cols(),
                polled_map(&e.cancel, c.codes(), |code| remap[code as usize])?,
            ));
        }
        blocks.push(stripe);
    }
    let (r, c, codes) = super::datetime_fns::join_grid(blocks, &e.cancel)?;
    Ok(value(CategoricalArray::new(
        r,
        c,
        codes,
        Arc::new(cats),
        base.is_ordinal(),
    )?))
}

pub(super) fn assign(
    e: &mut Engine,
    name: &str,
    args: &[Expr],
    rhs: Value,
    deletion: bool,
    quiet: bool,
    out: &mut Execution,
) -> Result<bool> {
    let current = e.array(name);
    let base = match current {
        Some(Array::Categorical(c)) => c,
        None if is_cat(&rhs) => of(&rhs).unwrap().with_codes(0, 0, vec![])?,
        Some(_) if is_cat(&rhs) => {
            return Err("Cannot assign categorical into a different array class".into());
        }
        _ => return Ok(false),
    };
    if args.len() > 2 {
        return Err("categorical: N-D indexing is not supported yet".into());
    }
    let mut cats = base.categories().as_ref().clone();
    let mut map: HashMap<String, u32> = cats
        .iter()
        .enumerate()
        .map(|(i, s)| (s.clone(), (i + 1) as u32))
        .collect();
    let (extra, shape) = if deletion {
        (vec![], (0, 0))
    } else {
        if let Some(c) = of(&rhs)
            && (base.is_ordinal() != c.is_ordinal()
                || (base.is_ordinal() && base.categories() != c.categories()))
        {
            return Err("categorical: ordinal category sets must match".into());
        }
        // NaN assigns <undefined>, as the reference allows.
        let (texts, shape) = match &rhs {
            Value::Matrix(m) if !m.is_empty() && m.data().iter().all(|x| x.is_nan()) => {
                (vec![None; m.len()], (m.rows(), m.cols()))
            }
            _ => texts(e, &rhs)?,
        };
        let mut extra = Vec::with_capacity(texts.len());
        for s in texts {
            e.check_cancel()?;
            let Some(s) = s.filter(|s| !s.is_empty() && s != "<undefined>" && s != "<missing>")
            else {
                extra.push(0);
                continue;
            };
            let code = if let Some(&code) = map.get(&s) {
                code
            } else {
                if base.is_ordinal() {
                    return Err("categorical: cannot add a new category to an ordinal array".into());
                }
                cats.push(s.clone());
                let code = cats.len() as u32;
                map.insert(s, code);
                code
            };
            extra.push(code);
        }
        (extra, shape)
    };
    let proxy = Matrix::from_column_vec(
        base.rows(),
        base.cols(),
        polled(&e.cancel, base.len(), |i| (i + 1) as f64)?,
    )?;
    let incoming = Value::Matrix(Matrix::from_column_vec(
        shape.0,
        shape.1,
        polled(&e.cancel, extra.len(), |i| (base.len() + i + 1) as f64)?,
    )?);
    let temp = (0usize..)
        .map(|i| format!("Zq_cat_{i}"))
        .find(|s| !e.contains_array(s))
        .unwrap();
    e.variables.insert(temp.clone(), proxy);
    let result = e.assign_indexed_value(&temp, args, incoming, deletion, true, out);
    let mapped = e.variables.remove(&temp);
    e.logical_variables.remove(&temp);
    e.complex_variables.remove(&temp);
    e.container_variables.remove(&temp);
    result?;
    let mapped = mapped.ok_or("categorical: indexed assignment did not produce a result")?;
    let codes = polled_map(&e.cancel, mapped.data(), |x| {
        if x < 1.0 {
            0
        } else {
            let i = x as usize - 1;
            if i < base.len() {
                base.codes()[i]
            } else {
                extra[i - base.len()]
            }
        }
    })?;
    e.assign_workspace_array(
        name,
        Array::Categorical(CategoricalArray::new(
            mapped.rows(),
            mapped.cols(),
            codes,
            Arc::new(cats),
            base.is_ordinal(),
        )?),
        quiet,
        out,
    )?;
    Ok(true)
}

pub(super) fn outputs(
    e: &mut Engine,
    name: &str,
    args: &[Value],
    count: usize,
) -> Result<Option<Vec<Value>>> {
    let owned = matches!(
        name,
        "categorical" | "categories" | "countcats" | "iscategorical"
    );
    if !owned && !args.iter().any(is_cat) {
        return Ok(None);
    }
    e.check_cancel()?;
    let one = |v| Ok(Some(vec![v]));
    if name == "categorical" {
        if count > 1 {
            return Err("categorical: too many outputs".into());
        }
        return one(construct(e, args)?);
    }
    if name == "iscategorical" {
        if args.len() != 1 || count > 1 {
            return Err("iscategorical: expected one input and output".into());
        }
        return one(Value::Logical(LogicalMatrix::scalar(is_cat(&args[0]))));
    }
    if matches!(name, "isequal" | "isequaln") {
        if args.len() < 2 || count > 1 {
            return Err(format!(
                "{name}: expected at least two inputs and one output"
            ));
        }
        let mut yes = true;
        if let Some(a) = of(&args[0]) {
            for b in &args[1..] {
                if let Some(b) = of(b) {
                    if !equal(a, b, name == "isequaln", &e.cancel)? {
                        yes = false;
                        break;
                    }
                } else {
                    yes = false;
                    break;
                }
            }
        } else {
            yes = false;
        }
        return one(Value::Logical(LogicalMatrix::scalar(yes)));
    }
    if matches!(name, "horzcat" | "vertcat" | "cat") {
        let (dim, parts) = if name == "cat" {
            (
                dimension(e, args.first().ok_or("cat: dimension required")?)?,
                &args[1..],
            )
        } else {
            (if name == "horzcat" { 2 } else { 1 }, args)
        };
        let grid = if dim == 1 {
            parts.iter().cloned().map(|v| vec![v]).collect()
        } else {
            vec![parts.to_vec()]
        };
        return one(concatenate(e, grid)?);
    }
    // Classifier front ends validate response categoricals without converting their labels.
    if matches!(
        name,
        "fitcknn"
            | "fitctree"
            | "fitcsvm"
            | "fitcnb"
            | "fitcdiscr"
            | "fitclinear"
            | "fitcecoc"
            | "fitcensemble"
            | "cvpartition"
            | "loss"
            | "margin"
            | "edge"
            | "fitrtree"
            | "fitrsvm"
            | "fitrgp"
            | "fitrensemble"
            | "fitrlinear"
    ) {
        return Ok(None);
    }
    // These builtins already treat arrays as opaque values and validate their own arity.
    if matches!(
        name,
        "class"
            | "isa"
            | "size"
            | "numel"
            | "length"
            | "isempty"
            | "ndims"
            | "isvector"
            | "isrow"
            | "iscolumn"
            | "isscalar"
            | "ismatrix"
            | "isnumeric"
            | "islogical"
            | "ischar"
            | "iscell"
            | "isstruct"
            | "isfloat"
            | "isinteger"
            | "isstring"
            | "isreal"
            | "issparse"
            | "disp"
            | "display"
            | "table"
            | "timetable"
            | "boxplot"
            | "perfcurve"
            | "plotconfusion"
            | "struct"
            | "deal"
            | "assert"
            | "cellfun"
            | "arrayfun"
            | "isdatetime"
            | "isduration"
            | "isobject"
            | "iscellstr"
            | "is_function_handle"
            | "setfield"
            | "getfield"
            | "isfield"
            | "rmfield"
            | "fieldnames"
            | "feval"
            | "func2str"
            | "builtin"
            | "exist"
            | "error"
            | "warning"
            | "fprintf"
            | "sprintf"
            | "cell2mat"
            | "ishandle"
            | "isgraphics"
            | "ismissing"
            | "rmmissing"
            | "fillmissing"
            | "standardizeMissing"
            | "findgroups"
            | "splitapply"
            | "groupsummary"
            | "varfun"
            | "rowfun"
            | "addvars"
            | "removevars"
            | "renamevars"
            | "movevars"
            | "innerjoin"
            | "outerjoin"
            | "join"
            | "height"
            | "width"
            | "head"
            | "tail"
            | "istable"
            | "array2table"
            | "cell2table"
            | "volshow"
            | "plot"
            | "semilogx"
            | "semilogy"
            | "loglog"
            | "line"
            | "scatter"
            | "bar"
            | "area"
            | "stairs"
            | "stem"
            | "plot3"
            | "scatter3"
            | "xlim"
            | "ylim"
            | "zlim"
            | "xticks"
            | "yticks"
            | "zticks"
            | "set"
    ) {
        return Ok(None);
    }
    let Some(c) = args.first().and_then(of) else {
        if owned {
            return Err(format!("{name}: expected a categorical array"));
        }
        return Err(format!("categorical is not supported by {name} yet"));
    };
    let max = match name {
        "sort" | "ismember" => 2,
        "unique" => 3,
        "size" => usize::MAX,
        _ => 1,
    };
    if count > max {
        return Err(format!("{name}: too many outputs"));
    }
    if matches!(
        name,
        "categories"
            | "isundefined"
            | "string"
            | "cellstr"
            | "char"
            | "double"
            | "transpose"
            | "ctranspose"
    ) && args.len() != 1
    {
        return Err(format!("{name}: expected one input"));
    }
    match name {
        "categories" => one(cellstr(
            e,
            c.categories().len(),
            1,
            c.categories().as_ref().clone(),
        )?),
        "isundefined" => one(logical(
            e,
            c.rows(),
            c.cols(),
            polled_map(&e.cancel, c.codes(), |x| u8::from(x == 0))?,
        )?),
        "double" => one(numbers(
            c.rows(),
            c.cols(),
            polled_map(&e.cancel, c.codes(), |x| {
                if x == 0 { f64::NAN } else { x as f64 }
            })?,
        )?),
        "string" => {
            text_budget(e, c, crate::string_array::MAX_STRING_BYTES)?;
            StringArray::checked_len(c.rows(), c.cols())?;
            one(Value::Container(Array::Str(StringArray::new(
                c.rows(),
                c.cols(),
                polled(&e.cancel, c.len(), |i| {
                    if c.codes()[i] == 0 {
                        None
                    } else {
                        Some(Arc::from(c.text(i)))
                    }
                })?,
            )?)))
        }
        "cellstr" => {
            if c.len() > crate::script_containers::MAX_CELL_SLOTS {
                return Err("cellstr: categorical output exceeds 65536 cells".into());
            }
            text_budget(e, c, 2 * 1024 * 1024)?;
            one(cellstr(
                e,
                c.rows(),
                c.cols(),
                polled(&e.cancel, c.len(), |i| c.text(i).to_string())?,
            )?)
        }
        "char" => {
            if c.len() != 1 {
                return Err("categorical: char requires a scalar".into());
            }
            one(Value::Text(c.text(0).into()))
        }
        "transpose" | "ctranspose" => one(transpose(e, c)?),
        "countcats" => {
            if args.len() > 2 {
                return Err("countcats: too many inputs".into());
            }
            let dim = if args.len() == 2 {
                dimension(e, &args[1])?
            } else if c.rows() == 1 && c.cols() != 1 {
                // First dimension whose size is not 1. A 1-by-0 array is not a
                // column, so the counts run along dimension 2 (1-by-n).
                2
            } else {
                1
            };
            let (rows, cols) = if dim == 1 {
                (c.categories().len(), c.cols())
            } else {
                (c.rows(), c.categories().len())
            };
            let n = crate::datetime_array::checked_len(rows, cols, "countcats")?;
            let mut counts = vec![0.; n];
            for (i, &code) in c.codes().iter().enumerate() {
                if i % 65536 == 0 {
                    e.check_cancel()?;
                }
                if code != 0 {
                    let k = code as usize - 1;
                    let j = if dim == 1 {
                        (i / c.rows()) * rows + k
                    } else {
                        k * rows + i % c.rows()
                    };
                    counts[j] += 1.;
                }
            }
            one(numbers(rows, cols, counts)?)
        }
        "reshape" => {
            let (rows, cols) = super::shape_arguments::shape(&args[1..], Some(c.len()))?;
            one(value(c.reshaped(rows, cols)?))
        }
        "repmat" => {
            let (r, k) = super::shape_arguments::shape(&args[1..], None)?;
            let rows = c.rows().checked_mul(r).ok_or("repmat: size overflow")?;
            let cols = c.cols().checked_mul(k).ok_or("repmat: size overflow")?;
            let n = crate::datetime_array::checked_len(rows, cols, "repmat")?;
            one(value(c.with_codes(
                rows,
                cols,
                polled(&e.cancel, n, |i| {
                    c.codes()[(i / rows % c.cols()) * c.rows() + i % rows % c.rows()]
                })?,
            )?))
        }
        "unique" => unique(e, c, &args[1..], count).map(Some),
        "sort" => sort(e, c, &args[1..], count).map(Some),
        "ismember" => {
            if args.len() != 2 {
                return Err("ismember: expected two inputs".into());
            }
            let other = labels(e, &args[1])?;
            let mut lookup = HashMap::new();
            for i in 0..other.len() {
                if i % 65536 == 0 {
                    e.check_cancel()?;
                }
                if let Some(t) = other.at(i) {
                    lookup.entry(t).or_insert(i + 1);
                }
            }
            let loc = polled(&e.cancel, c.len(), |i| {
                if c.codes()[i] == 0 {
                    0.
                } else {
                    lookup.get(c.text(i)).copied().unwrap_or(0) as f64
                }
            })?;
            let mut result = vec![logical(
                e,
                c.rows(),
                c.cols(),
                loc.iter().map(|x| u8::from(*x != 0.)).collect(),
            )?];
            if count > 1 {
                result.push(numbers(c.rows(), c.cols(), loc)?);
            }
            Ok(Some(result))
        }
        _ => Err(format!("categorical is not supported by {name} yet")),
    }
}
fn dimension(e: &Engine, v: &Value) -> Result<usize> {
    let m = v.clone().matrix(&e.cancel)?;
    if m.len() != 1 || !matches!(m.data()[0], 1.0 | 2.0) {
        return Err("categorical: dimension must be 1 or 2 (N-D not supported yet)".into());
    }
    Ok(m.data()[0] as usize)
}
fn sort(e: &Engine, c: &CategoricalArray, args: &[Value], count: usize) -> Result<Vec<Value>> {
    work_budget(c.len(), 28)?;
    let mut dim = if c.rows() == 1 { 2 } else { 1 };
    let mut descending = false;
    let mut seen_dim = false;
    let mut seen_order = false;
    for v in args {
        if let Some(s) = text(e, v) {
            if seen_order || !matches!(s.as_str(), "ascend" | "descend") {
                return Err("sort: unsupported option".into());
            }
            descending = s == "descend";
            seen_order = true;
        } else {
            if seen_dim {
                return Err("sort: repeated dimension".into());
            }
            dim = dimension(e, v)?;
            seen_dim = true;
        }
    }
    let (lanes, len) = if dim == 1 {
        (c.cols(), c.rows())
    } else {
        (c.rows(), c.cols())
    };
    let mut codes = vec![0; c.len()];
    let mut indices = vec![0.; c.len()];
    for lane in 0..lanes {
        e.check_cancel()?;
        let at = |k| {
            if dim == 1 {
                lane * c.rows() + k
            } else {
                k * c.rows() + lane
            }
        };
        let mut order: Vec<usize> = (0..len).collect();
        e.dtm_stable_sort(&mut order, |i, j| {
            let (a, b) = (c.codes()[at(i)], c.codes()[at(j)]);
            // Undefined elements go last ascending and first descending.
            match (a == 0, b == 0) {
                (true, false) if descending => std::cmp::Ordering::Less,
                (false, true) if descending => std::cmp::Ordering::Greater,
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                _ => {
                    if descending {
                        b.cmp(&a)
                    } else {
                        a.cmp(&b)
                    }
                }
            }
        })?;
        e.check_cancel()?;
        for (k, &i) in order.iter().enumerate() {
            codes[at(k)] = c.codes()[at(i)];
            indices[at(k)] = (i + 1) as f64;
        }
    }
    let mut out = vec![value(c.with_codes(c.rows(), c.cols(), codes)?)];
    if count > 1 {
        out.push(numbers(c.rows(), c.cols(), indices)?);
    }
    Ok(out)
}
fn unique(e: &Engine, c: &CategoricalArray, args: &[Value], count: usize) -> Result<Vec<Value>> {
    work_budget(c.len(), 96)?;
    let mut stable = false;
    if args.len() > 1 {
        return Err("unique: unsupported options".into());
    }
    if let Some(v) = args.first() {
        match text(e, v).as_deref() {
            Some("stable") => stable = true,
            Some("sorted") => {}
            _ => return Err("unique: unsupported option".into()),
        }
    }
    let mut first = HashMap::new();
    let mut codes = Vec::new();
    for (i, &code) in c.codes().iter().enumerate() {
        if i % 65536 == 0 {
            e.check_cancel()?;
        }
        if let std::collections::hash_map::Entry::Vacant(entry) = first.entry(code) {
            entry.insert(i);
            codes.push(code);
        }
    }
    if !stable {
        codes.sort_unstable_by_key(|&x| if x == 0 { u32::MAX } else { x });
    }
    let n = codes.len();
    let shape = if c.rows() == 1 { (1, n) } else { (n, 1) };
    let mut out = vec![value(c.with_codes(shape.0, shape.1, codes.clone())?)];
    if count > 1 {
        out.push(numbers(
            n,
            1,
            codes.iter().map(|k| (first[k] + 1) as f64).collect(),
        )?);
    }
    if count > 2 {
        let map: HashMap<u32, usize> = codes.iter().enumerate().map(|(i, &c)| (c, i + 1)).collect();
        out.push(numbers(
            c.len(),
            1,
            polled_map(&e.cancel, c.codes(), |k| map[&k] as f64)?,
        )?);
    }
    Ok(out)
}
