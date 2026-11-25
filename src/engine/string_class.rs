//! String-class dispatch. Legacy character/cellstr kernels remain independent.
use super::{Engine, Execution, Value};
use crate::{
    Array, CellArray, CharacterRow, LogicalMatrix, Matrix, Result, StringArray,
    syntax::{Expr, Token},
};
use std::sync::Arc;

// Validate with the existing formatter before counting conversions and star inputs.
// Keep compose-specific shape handling in this module, outside the legacy kernels.
fn str_compose_input_count(format: &str, cancel: &std::sync::atomic::AtomicBool) -> Result<usize> {
    super::text_format::format_items("compose", format, &[], cancel)?;
    let unescaped = super::text_format::unescape(format);
    let mut chars = unescaped.chars().peekable();
    let mut count = 0;
    while let Some(ch) = chars.next() {
        if ch != '%' {
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            continue;
        }
        let mut inputs = 1;
        for ch in chars.by_ref() {
            if ch == '*' {
                inputs += 1;
            }
            if "diufFeEgGxXocs".contains(ch) {
                count += inputs;
                break;
            }
        }
        // The formatter treats a trailing incomplete conversion as literal text.
    }
    Ok(count)
}

pub(super) const STRING_AWARE: &[&str] = &[
    "title",
    "subtitle",
    "xlabel",
    "ylabel",
    "zlabel",
    "sgtitle",
    "text",
    "set",
    "get",
    "tf",
    "zpk",
    "ss",
    "c2d",
    // Argument validation functions accept string arrays and scalar strings.
    "mustBePositive",
    "mustBeNonnegative",
    "mustBeNegative",
    "mustBeNonpositive",
    "mustBeNonzero",
    "mustBeFinite",
    "mustBeNonNan",
    "mustBeInteger",
    "mustBeReal",
    "mustBeNumeric",
    "mustBeNumericOrLogical",
    "mustBeNonempty",
    "mustBeMember",
    "mustBeInRange",
    "mustBeGreaterThan",
    "mustBeGreaterThanOrEqual",
    "mustBeLessThan",
    "mustBeLessThanOrEqual",
    "mustBeText",
    "mustBeTextScalar",
    "mustBeNonzeroLengthText",
    "mustBeFile",
    "mustBeFolder",
    "mustBeVector",
    "mustBeRow",
    "mustBeColumn",
    "mustBeScalarOrEmpty",
    "mustBeA",
    "mustBeUnderlyingType",
    // isfile/isfolder answer per element of a string array.
    "isfile",
    "isfolder",
    // gscatter takes a string-array grouping variable.
    "gscatter",
    // econometrics tests take string options (Model=["AR";"TS"])
    "autocorr",
    "parcorr",
    "crosscorr",
    "lbqtest",
    "archtest",
    "adftest",
    "kpsstest",
    "pptest",
    "garch",
    "egarch",
    "gjr",
    "estimate",
    "infer",
    "forecast",
    "simulate",
    "summarize",
    "profile",
    "profsave",
    "timetable",
    "table2timetable",
    "timetable2table",
    "istimetable",
    "retime",
    "synchronize",
    "readlines",
    "writelines",
    "calendarDuration",
    "calyears",
    "calmonths",
    "calweeks",
    "caldays",
    "xticklabels",
    "yticklabels",
    "zticklabels",
    "colororder",
    "iscalendarduration",
    "between",
    "makima",
    "griddedInterpolant",
    "scatteredInterpolant",
    "makedist",
    "fitdist",
    "paramci",
    "designfilt",
    "fitcknn",
    "fitctree",
    "fitcsvm",
    "fitcdiscr",
    "fitclinear",
    "fitcecoc",
    "fitcensemble",
    "templateSVM",
    "templateTree",
    "templateLinear",
    "templateDiscriminant",
    "cvpartition",
    "training",
    "test",
    "repartition",
    "loss",
    "margin",
    "edge",
    "oobLoss",
    "predictorImportance",
    "fitcnb",
    "fitrtree",
    "fitrsvm",
    "fitrgp",
    "fitrensemble",
    "fitrlinear",
    "prune",
    "crossval",
    "predict",
    "resubLoss",
    "kfoldLoss",
    "kfoldPredict",
    "fitlm",
    "fitglm",
    "stepwiselm",
    "fit",
    "fittype",
    "fitoptions",
    "cfit",
    "sfit",
    "confint",
    "predint",
    "coeffnames",
    "coeffvalues",
    "numcoeffs",
    "numargs",
    "dependnames",
    "indepnames",
    "probnames",
    "probvalues",
    "islinear",
    "category",
    "differentiate",
    "integrate",
    "dendrogram",
    "addvars",
    "removevars",
    "renamevars",
    "movevars",
    "innerjoin",
    "outerjoin",
    "findgroups",
    "splitapply",
    "groupsummary",
    "varfun",
    "rowfun",
    "rmmissing",
    "fillmissing",
    "standardizeMissing",
    "isoutlier",
    "filloutliers",
    "rmoutliers",
    "nichols",
    "sigma",
    "allmargin",
    "ngrid",
    "uigetfile",
    "uiputfile",
    "uigetdir",
    "inputdlg",
    "questdlg",
    "msgbox",
    "errordlg",
    "warndlg",
    "helpdlg",
    "waitbar",
    "uiwait",
    "ttest",
    "ttest2",
    "anova1",
    "anova2",
    "kstest",
    "chi2gof",
    "ranksum",
    "signrank",
    "trimmean",
    "randsample",
    "datasample",
    "bootstrp",
    "string",
    "strings",
    "missing",
    "ismissing",
    "isstring",
    "size",
    "numel",
    "length",
    "isempty",
    "isscalar",
    "isrow",
    "iscolumn",
    "isvector",
    "ismatrix",
    "ndims",
    "class",
    "isa",
    "isequal",
    "isequaln",
    "iscell",
    "ischar",
    "isreal",
    "strlength",
    "char",
    "cellstr",
    "double",
    "str2double",
    "upper",
    "lower",
    "strip",
    "strtrim",
    "contains",
    "count",
    "startsWith",
    "endsWith",
    "replace",
    "strrep",
    "split",
    "join",
    "strjoin",
    "strcat",
    "sprintf",
    "fprintf",
    "compose",
    "extractBefore",
    "extractAfter",
    "extractBetween",
    "insertBefore",
    "insertAfter",
    "erase",
    "reverse",
    "pad",
    "disp",
    "display",
    "regexprep",
    "regexp",
    "regexpi",
    "cellfun",
    "arrayfun",
    "struct",
    "cell",
    "save",
    "jsonencode",
    "xlswrite",
    "webread",
    "websave",
    "weboptions",
    "h5read",
    "h5create",
    "h5write",
    "ncread",
    "confusionmat",
    "rng",
];
pub(super) fn is_string(v: &Value) -> bool {
    matches!(v, Value::QuotedString(_) | Value::Container(Array::Str(_)))
}
pub(super) fn view(v: &Value) -> Result<StringArray> {
    match v {
        Value::QuotedString(s) => StringArray::scalar(s.as_str()),
        Value::Container(Array::Str(s)) => Ok(s.clone()),
        _ => Err("Expected a string array".into()),
    }
}
/// All value-level producers canonicalize a present scalar; stored values always use Array::Str.
pub(super) fn value(s: StringArray) -> Value {
    if s.len() == 1
        && let Some(text) = &s.data()[0]
    {
        Value::QuotedString(text.to_string())
    } else {
        Value::Container(Array::Str(s))
    }
}
pub(super) fn guard(name: &str, args: &[Value]) -> Result<()> {
    if args.iter().any(super::categorical_fns::is_cat) {
        // Categorical calls take text operands (C == "a", ismember(C, "a")).
        return Ok(());
    }
    // These array consumers used to reject QuotedString in Value::array().
    // Guard their data operands too; text options and legacy text builtins
    // keep their established dispatch (for example sort(A,"descend")).
    let scalar_data_count = match name {
        "full" | "repelem" | "permute" | "ipermute" | "shiftdim" | "flipdim" | "unique"
        | "mat2cell" | "validateattributes" | "isdiag" | "istril" | "istriu" | "isbanded"
        | "issymmetric" | "ishermitian" | "bandwidth" | "uniquetol" | "setstr" | "hex2num"
        | "filter2" | "onCleanup" | "subsref" => 1,
        "ismember" | "union" | "intersect" | "setdiff" | "setxor" | "ismembertol" => 2,
        "blkdiag" | "isequalwithequalnans" | "substruct" => args.len(),
        _ => 0,
    };
    let assignment_string = name == "assignin" && args.get(2).is_some_and(is_string);
    if args
        .iter()
        .any(|v| matches!(v, Value::Container(Array::Str(_))))
        && !STRING_AWARE.contains(&name)
        || args.iter().take(scalar_data_count).any(is_string)
        || assignment_string
    {
        return Err(format!(
            "{name} does not support string arrays yet; convert with char(...) or cellstr(...)"
        ));
    }
    Ok(())
}
pub(super) fn arguments(
    engine: &mut Engine,
    name: &str,
    expressions: &[Expr],
    out: &mut Execution,
) -> Result<Vec<Value>> {
    let args = super::nd::arguments(engine, name, expressions, out)?;
    if args.iter().any(is_string) && engine.resolve_function(name)?.is_none() {
        guard(name, &args)?;
    }
    Ok(args)
}
fn arity(name: &str, args: &[Value], min: usize, max: usize) -> Result<()> {
    if (min..=max).contains(&args.len()) {
        Ok(())
    } else {
        Err(format!("{name}: expected {min}..{max} inputs"))
    }
}
pub(super) fn number(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v < 0. { "-Inf" } else { "Inf" }.into();
    }
    if v == 0. {
        return "0".into();
    }
    if v.abs() < 1e-4 || v.abs() >= 1e16 {
        let raw = format!("{v:e}");
        let (mantissa, exp) = raw.split_once('e').expect("scientific exponent");
        let exp: i32 = exp.parse().expect("integer exponent");
        format!(
            "{mantissa}e{}{:#02}",
            if exp < 0 { '-' } else { '+' },
            exp.abs()
        )
    } else {
        v.to_string()
    }
}
fn str_position(v: &Value) -> bool {
    matches!(v, Value::Matrix(_) | Value::Container(Array::Typed(_)))
}
fn number_single(v: f32) -> String {
    if !v.is_finite() || v == 0. {
        return number(f64::from(v));
    }
    if v.abs() < 1e-4 || v.abs() >= 1e16 {
        let raw = format!("{v:e}");
        let (mantissa, exp) = raw.split_once('e').expect("scientific exponent");
        let exp: i32 = exp.parse().expect("integer exponent");
        format!(
            "{mantissa}e{}{:02}",
            if exp < 0 { '-' } else { '+' },
            exp.abs()
        )
    } else {
        v.to_string()
    }
}
fn broadcast(a: &StringArray, b: &StringArray) -> Result<(usize, usize)> {
    let axis = |x, y| {
        if x == y {
            Ok(x)
        } else if x == 1 {
            Ok(y)
        } else if y == 1 {
            Ok(x)
        } else {
            Err("String array sizes are incompatible".to_owned())
        }
    };
    let shape = (axis(a.rows(), b.rows())?, axis(a.cols(), b.cols())?);
    StringArray::checked_len(shape.0, shape.1)?;
    Ok(shape)
}
fn at(s: &StringArray, r: usize, c: usize) -> &Option<Arc<str>> {
    &s.data()[(c % s.cols()) * s.rows() + r % s.rows()]
}
impl Engine {
    pub(super) fn str_transpose(&self, input: &Value) -> Result<Value> {
        let s = view(input)?;
        let mut data = Vec::with_capacity(s.len());
        for r in 0..s.rows() {
            for c in 0..s.cols() {
                self.check_cancel()?;
                data.push(s.data()[c * s.rows() + r].clone());
            }
        }
        Ok(value(StringArray::new(s.cols(), s.rows(), data)?))
    }
    fn str_restore_text_type(
        &self,
        input: &Value,
        result: Value,
        cell_output: bool,
    ) -> Result<Value> {
        let s = view(&result)?;
        if cell_output || matches!(input, Value::Container(Array::Cell(_))) {
            let items = s
                .data()
                .iter()
                .map(|s| {
                    self.check_cancel()?;
                    let text = s
                        .as_deref()
                        .ok_or("Character output cannot represent missing strings")?;
                    CharacterRow::from_text(text, &self.cancel)
                        .map(|c| Arc::new(Array::Character(c)))
                })
                .collect::<Result<Vec<_>>>()?;
            return Ok(Value::Container(
                CellArray::from_shared(s.rows(), s.cols(), items, &self.cancel)?.into(),
            ));
        }
        if !input.is_character() {
            return Err("Text input must be char, cellstr or string".into());
        }
        let width = s
            .data()
            .iter()
            .flatten()
            .map(|s| s.encode_utf16().count())
            .max()
            .unwrap_or(0);
        let mut units = vec![32; CharacterRow::checked_len(s.len(), width)?];
        for (r, text) in s.data().iter().enumerate() {
            self.check_cancel()?;
            for (c, unit) in text
                .as_deref()
                .ok_or("Character output cannot represent missing strings")?
                .encode_utf16()
                .enumerate()
            {
                units[c * s.len() + r] = unit;
            }
        }
        Ok(Value::Container(
            CharacterRow::from_column_major(s.len(), width, units, &self.cancel)?.into(),
        ))
    }
    fn str_ismissing(&self, input: &Value) -> Result<Value> {
        let a = input.clone().array(&self.cancel)?;
        let mut flags = Vec::with_capacity(a.len());
        for i in 0..a.len() {
            self.check_cancel()?;
            flags.push(u8::from(match &a {
                Array::Str(s) => s.data()[i].is_none(),
                Array::Numeric(m) => m.data()[i].is_nan(),
                Array::Complex(m) => m.data()[i].re.is_nan() || m.data()[i].im.is_nan(),
                Array::Typed(t) => t.real(i).is_nan(),
                Array::Cell(c) => match c.get_linear(i)?.as_ref() {
                    Array::Character(s) if s.rows() <= 1 => s.is_empty(),
                    _ => return Err("ismissing: cell input must contain character rows".into()),
                },
                _ => return Err("ismissing: input must be numeric, cellstr or string".into()),
            }));
        }
        if !matches!(
            a,
            Array::Str(_)
                | Array::Numeric(_)
                | Array::Complex(_)
                | Array::Typed(_)
                | Array::Cell(_)
        ) {
            return Err("ismissing: input must be numeric, cellstr or string".into());
        }
        Ok(Value::Logical(
            LogicalMatrix::from_column_vec(a.rows(), a.cols(), flags, &self.cancel)?
                .with_dims(&a.dims())?,
        ))
    }
    pub(super) fn str_convert(&self, v: &Value) -> Result<StringArray> {
        self.check_cancel()?;
        if is_string(v) {
            return view(v);
        }
        if !v.is_character() && !matches!(v, Value::Text(_)) {
            let dims = v.dims();
            if dims.len() != 2 {
                return Err("string: only two dimensions are supported".into());
            }
            StringArray::checked_len(dims[0], dims[1])?;
        }
        let (rows, cols, items) =
            match v {
                Value::Text(s) => return StringArray::scalar(s.as_str()),
                Value::Container(Array::Character(a)) => {
                    if a.dims().len() != 2 {
                        return Err("string: only two dimensions are supported".into());
                    }
                    if a.is_empty() {
                        return StringArray::scalar("");
                    }
                    StringArray::checked_len(a.rows(), 1)?;
                    if a.len() > crate::string_array::MAX_STRING_BYTES {
                        return Err("String array text exceeds 2 MiB".into());
                    }
                    let mut items = Vec::with_capacity(a.rows());
                    for r in 0..a.rows() {
                        self.check_cancel()?;
                        let units = (0..a.cols())
                            .map(|c| a.units()[c * a.rows() + r])
                            .collect::<Vec<_>>();
                        items.push(Some(Arc::from(
                            String::from_utf16(&units).map_err(|_| "string: invalid UTF-16")?,
                        )));
                    }
                    (a.rows(), 1, items)
                }
                Value::Container(Array::Cell(a)) => {
                    let mut items = Vec::with_capacity(a.len());
                    let mut total_bytes = 0;
                    for i in 0..a.len() {
                        self.check_cancel()?;
                        let item = a.get_linear(i)?;
                        match item.as_ref() {
                            Array::Character(c) if c.rows() <= 1 => {
                                let text = c.to_utf8(&self.cancel)?;
                                str_charge_text(&mut total_bytes, text.len())?;
                                items.push(Some(Arc::from(text)));
                            }
                            Array::Str(s) if s.len() == 1 => {
                                if let Some(text) = &s.data()[0] {
                                    str_charge_text(&mut total_bytes, text.len())?;
                                }
                                items.push(s.data()[0].clone());
                            }
                            _ => return Err(
                                "string: cell elements must be character rows or scalar strings"
                                    .into(),
                            ),
                        }
                    }
                    (a.rows(), a.cols(), items)
                }
                Value::Matrix(a) if !a.is_nd() => (
                    a.rows(),
                    a.cols(),
                    a.data()
                        .iter()
                        .map(|&v| {
                            self.check_cancel()?;
                            Ok(Some(Arc::from(number(v))))
                        })
                        .collect::<Result<Vec<_>>>()?,
                ),
                Value::Complex(a) if !a.is_nd() => {
                    let mut items = Vec::with_capacity(a.len());
                    for z in a.data() {
                        self.check_cancel()?;
                        let text = if z.im == 0. {
                            number(z.re)
                        } else {
                            format!(
                                "{}{}{}i",
                                number(z.re),
                                if z.im.is_sign_negative() { '-' } else { '+' },
                                number(z.im.abs())
                            )
                        };
                        items.push(Some(Arc::from(text)));
                    }
                    (a.rows(), a.cols(), items)
                }
                Value::Logical(a) if !a.is_nd() => (
                    a.rows(),
                    a.cols(),
                    a.data()
                        .iter()
                        .map(|&v| {
                            self.check_cancel()?;
                            Ok(Some(Arc::from(if v == 0 { "false" } else { "true" })))
                        })
                        .collect::<Result<Vec<_>>>()?,
                ),
                Value::Container(Array::Typed(a)) if !a.is_nd() => {
                    let mut items = Vec::with_capacity(a.len());
                    for i in 0..a.len() {
                        self.check_cancel()?;
                        items.push(Some(Arc::from(if a.is_integer() {
                            a.int(i).to_string()
                        } else {
                            number_single(a.real(i) as f32)
                        })));
                    }
                    (a.rows(), a.cols(), items)
                }
                _ => return Err("string: unsupported input type or dimensions".into()),
            };
        StringArray::new(rows, cols, items)
    }
    fn str_text(&self, v: &Value) -> Result<String> {
        if is_string(v) {
            let s = view(v)?;
            if s.len() == 1 {
                return s.data()[0]
                    .as_ref()
                    .map(|s| s.to_string())
                    .ok_or("Text option cannot be missing".into());
            }
            return Err("Text option must be scalar".into());
        }
        self.text_value(v.clone())
    }
    pub(super) fn str_binary(&self, op: &Token, a: &Value, b: &Value) -> Result<Value> {
        if super::categorical_fns::is_cat(b) {
            return super::categorical_fns::binary(self, op, a, b);
        }
        if !matches!(
            op,
            Token::Plus
                | Token::Equal
                | Token::NotEqual
                | Token::Less
                | Token::LessEqual
                | Token::Greater
                | Token::GreaterEqual
        ) {
            return Err("Unsupported operator for string arrays".into());
        }
        let (a, b) = (self.str_convert(a)?, self.str_convert(b)?);
        let (rows, cols) = broadcast(&a, &b)?;
        let mut text = Vec::new();
        let mut flags = Vec::new();
        let mut bytes = 0usize;
        for c in 0..cols {
            for r in 0..rows {
                self.check_cancel()?;
                let (x, y) = (at(&a, r, c), at(&b, r, c));
                if op == &Token::Plus {
                    text.push(match (x, y) {
                        (Some(x), Some(y)) => {
                            bytes = bytes
                                .checked_add(x.len())
                                .and_then(|n| n.checked_add(y.len()))
                                .ok_or("String text overflow")?;
                            if bytes > crate::string_array::MAX_STRING_BYTES {
                                return Err("String array text exceeds 2 MiB".into());
                            }
                            Some(Arc::from(format!("{x}{y}")))
                        }
                        _ => None,
                    });
                } else {
                    let flag = match (x, y) {
                        (Some(x), Some(y)) => match op {
                            Token::Equal => x == y,
                            Token::NotEqual => x != y,
                            Token::Less => x < y,
                            Token::LessEqual => x <= y,
                            Token::Greater => x > y,
                            Token::GreaterEqual => x >= y,
                            _ => return Err("Unsupported operator for string arrays".into()),
                        },
                        _ => op == &Token::NotEqual,
                    };
                    flags.push(u8::from(flag));
                }
            }
        }
        if op == &Token::Plus {
            Ok(value(StringArray::new(rows, cols, text)?))
        } else {
            Ok(Value::Logical(LogicalMatrix::from_column_vec(
                rows,
                cols,
                flags,
                &self.cancel,
            )?))
        }
    }
    pub(super) fn str_concatenate(&self, rows: Vec<Vec<Value>>) -> Result<Value> {
        let mut blocks = Vec::new();
        let (mut height, mut width) = (0usize, None);
        for row in rows {
            let mut parts = Vec::new();
            let (mut h, mut w) = (None, 0usize);
            for v in row {
                if matches!(&v, Value::Matrix(m) if (m.rows(), m.cols()) == (0, 0)) {
                    continue;
                }
                let s = self.str_convert(&v)?;
                if (s.rows(), s.cols()) == (0, 0) {
                    continue;
                }
                if h.is_some_and(|h| h != s.rows()) {
                    return Err("String concatenation row dimensions disagree".into());
                }
                h = Some(s.rows());
                w += s.cols();
                parts.push(s);
            }
            if let Some(h) = h {
                if width.is_some_and(|n| n != w) {
                    return Err("String concatenation column dimensions disagree".into());
                }
                width = Some(w);
                height += h;
                blocks.push((h, parts));
            }
        }
        let width = width.unwrap_or(0);
        let mut data = vec![None; StringArray::checked_len(height, width)?];
        let mut ro = 0;
        for (h, parts) in blocks {
            let mut co = 0;
            for part in parts {
                for c in 0..part.cols() {
                    for r in 0..part.rows() {
                        self.check_cancel()?;
                        data[(co + c) * height + ro + r] = part.data()[c * part.rows() + r].clone();
                    }
                }
                co += part.cols();
            }
            ro += h;
        }
        Ok(value(StringArray::new(height, width, data)?))
    }
    pub(super) fn str_call(
        &mut self,
        name: &str,
        args: &[Value],
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let new = matches!(
            name,
            "string"
                | "strings"
                | "missing"
                | "ismissing"
                | "isstring"
                | "strlength"
                | "strip"
                | "contains"
                | "count"
                | "startsWith"
                | "endsWith"
                | "replace"
                | "split"
                | "join"
                | "compose"
                | "extractBefore"
                | "extractAfter"
                | "extractBetween"
                | "insertBefore"
                | "insertAfter"
                | "erase"
                | "reverse"
                | "pad"
                | "newline"
        );
        let has_string = args.iter().any(is_string);
        if !new && !has_string {
            return Ok(None);
        }
        // A string used as an option must keep the existing builtin route.
        if !new
            && !args.first().is_some_and(is_string)
            && !matches!(
                name,
                "strcat" | "isequal" | "isequaln" | "strrep" | "regexprep"
            )
        {
            return Ok(None);
        }
        let result = match name {
            "string" => {
                arity(name, args, 0, 1)?;
                value(if args.is_empty() {
                    StringArray::scalar("")?
                } else {
                    self.str_convert(&args[0])?
                })
            }
            "strings" => {
                let dims = if args.is_empty() {
                    vec![1, 1]
                } else {
                    super::shape_arguments::shape_dims(args, None)?
                };
                if dims.len() != 2 {
                    return Err("strings supports two dimensions only".into());
                }
                value(StringArray::new(
                    dims[0],
                    dims[1],
                    vec![Some(Arc::from("")); StringArray::checked_len(dims[0], dims[1])?],
                )?)
            }
            "missing" => {
                arity(name, args, 0, 0)?;
                value(StringArray::new(1, 1, vec![None])?)
            }
            "newline" => {
                arity(name, args, 0, 0)?;
                Value::Text("\n".into())
            }
            "isstring" => {
                arity(name, args, 1, 1)?;
                Value::Logical(LogicalMatrix::scalar(is_string(&args[0])))
            }
            "class" => {
                arity(name, args, 1, 1)?;
                Value::Text("string".into())
            }
            "isa" => {
                arity(name, args, 2, 2)?;
                Value::Logical(LogicalMatrix::scalar(self.str_text(&args[1])? == "string"))
            }
            "ischar" | "iscell" => {
                arity(name, args, 1, 1)?;
                Value::Logical(LogicalMatrix::scalar(false))
            }
            "isequal" | "isequaln" => {
                arity(name, args, 2, usize::MAX)?;
                if !args.iter().all(is_string) {
                    return Ok(Some(Value::Logical(LogicalMatrix::scalar(false))));
                }
                let a = view(&args[0])?;
                let mut equal = true;
                for b in &args[1..] {
                    if !is_string(b) {
                        equal = false;
                        continue;
                    }
                    let b = view(b)?;
                    equal &= (a.rows(), a.cols()) == (b.rows(), b.cols())
                        && a.data()
                            .iter()
                            .zip(b.data())
                            .all(|(a, b)| (name == "isequaln" || a.is_some()) && a == b);
                }
                Value::Logical(LogicalMatrix::scalar(equal))
            }
            "size" | "numel" | "length" | "isempty" | "isscalar" | "isrow" | "iscolumn"
            | "isvector" | "ismatrix" | "ndims" => {
                arity(name, args, 1, if name == "size" { 2 } else { 1 })?;
                let s = view(&args[0])?;
                let (r, c) = (s.rows(), s.cols());
                match name {
                    "size" if args.len() == 1 => {
                        Value::Matrix(Matrix::from_column_vec(1, 2, vec![r as f64, c as f64])?)
                    }
                    "size" => {
                        let d = args[1].clone().matrix(&self.cancel)?.as_scalar()?;
                        if d < 1. || !d.is_finite() || d.fract() != 0. {
                            return Err("Invalid size dimension".into());
                        }
                        Value::Matrix(Matrix::scalar(if d == 1. {
                            r as f64
                        } else if d == 2. {
                            c as f64
                        } else {
                            1.
                        }))
                    }
                    "numel" => Value::Matrix(Matrix::scalar(s.len() as f64)),
                    "length" => Value::Matrix(Matrix::scalar(
                        if s.is_empty() { 0 } else { r.max(c) } as f64,
                    )),
                    "ndims" => Value::Matrix(Matrix::scalar(2.)),
                    _ => Value::Logical(LogicalMatrix::scalar(match name {
                        "isempty" => s.is_empty(),
                        "isscalar" => s.len() == 1,
                        "isrow" => r == 1,
                        "iscolumn" => c == 1,
                        "isvector" => r == 1 || c == 1,
                        _ => true,
                    })),
                }
            }
            "ismissing" => {
                arity(name, args, 1, 1)?;
                self.str_ismissing(&args[0])?
            }
            "strlength" => {
                arity(name, args, 1, 1)?;
                let s = self.str_convert(&args[0])?;
                Value::Matrix(Matrix::from_column_vec(
                    s.rows(),
                    s.cols(),
                    s.data()
                        .iter()
                        .map(|s| {
                            s.as_ref()
                                .map_or(f64::NAN, |s| s.encode_utf16().count() as f64)
                        })
                        .collect(),
                )?)
            }
            "char" | "cellstr" => {
                arity(name, args, 1, 1)?;
                let s = view(&args[0])?;
                let texts = s
                    .data()
                    .iter()
                    .map(|s| s.as_deref().unwrap_or("<missing>"))
                    .collect::<Vec<_>>();
                if name == "cellstr" {
                    let items = texts
                        .iter()
                        .map(|s| {
                            CharacterRow::from_text(s, &self.cancel)
                                .map(|c| Arc::new(Array::Character(c)))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    Value::Container(
                        CellArray::from_shared(s.rows(), s.cols(), items, &self.cancel)?.into(),
                    )
                } else {
                    let width = texts
                        .iter()
                        .map(|s| s.encode_utf16().count())
                        .max()
                        .unwrap_or(0);
                    CharacterRow::checked_len(s.len(), width)?;
                    let mut units = vec![32; s.len() * width];
                    for (r, text) in texts.iter().enumerate() {
                        self.check_cancel()?;
                        for (c, u) in text.encode_utf16().enumerate() {
                            units[c * s.len() + r] = u;
                        }
                    }
                    Value::Container(
                        CharacterRow::from_column_major(s.len(), width, units, &self.cancel)?
                            .into(),
                    )
                }
            }
            "double" | "str2double" => {
                arity(name, args, 1, 1)?;
                let s = view(&args[0])?;
                let mut data = Vec::with_capacity(s.len());
                let mut complex = false;
                for text in s.data() {
                    self.check_cancel()?;
                    let z = if let Some(text) = text {
                        if let Ok(real) = text.trim().parse::<f64>() {
                            crate::Complex64::new(real, 0.)
                        } else {
                            match self
                                .strings2_outputs(
                                    "str2double",
                                    &[Value::Text(text.to_string())],
                                    1,
                                )?
                                .remove(0)
                            {
                                Value::Matrix(m) => crate::Complex64::new(m.data()[0], 0.),
                                Value::Complex(m) => m.data()[0],
                                _ => unreachable!("str2double returns a numeric scalar"),
                            }
                        }
                    } else {
                        crate::Complex64::new(f64::NAN, 0.)
                    };
                    complex |= z.im != 0.;
                    data.push(z);
                }
                if complex {
                    Value::Complex(crate::ComplexMatrix::from_column_vec(
                        s.rows(),
                        s.cols(),
                        data,
                        &self.cancel,
                    )?)
                } else {
                    Value::Matrix(Matrix::from_column_vec(
                        s.rows(),
                        s.cols(),
                        data.into_iter().map(|z| z.re).collect(),
                    )?)
                }
            }
            "mat2str" | "int2str" if matches!(args.first(), Some(Value::QuotedString(_))) => {
                return self.text_format_call(name, args.to_vec(), out);
            }
            "num2str" | "mat2str" | "int2str" => {
                return Err(format!(
                    "{name} does not accept string inputs; use char(...)"
                ));
            }
            // display labels its output like an echo; the host layer owns it.
            "disp" => {
                arity(name, args, 1, 1)?;
                if matches!(&args[0], Value::QuotedString(s) if s.is_empty()) {
                    return Ok(Some(Value::Void));
                }
                if let Some(text) =
                    crate::display_format::disp(&Array::Str(view(&args[0])?), &self.display)?
                {
                    self.emit(out, text)?;
                }
                Value::Void
            }
            "split" => self.str_split(args)?,
            "join" | "strjoin" => self.str_join(name, args)?,
            "compose" | "sprintf" => self.str_compose(name, args, out)?,
            "regexp" | "regexpi" => return Ok(Some(self.str_regexp(name, args, 1)?.remove(0))),
            "strcat" => {
                arity(name, args, 1, usize::MAX)?;
                let matrix_rows = args.iter().find_map(|arg| match arg {
                    Value::Container(Array::Character(chars)) if !chars.is_text_row() => {
                        Some(chars.rows())
                    }
                    _ => None,
                });
                let operand = |arg: &Value| -> Result<StringArray> {
                    let text = match arg {
                        Value::Container(Array::Character(chars))
                            if !chars.is_nd() && !chars.is_text_row() && chars.is_empty() =>
                        {
                            let rows = chars.rows();
                            StringArray::checked_len(rows, 1)?;
                            StringArray::new(rows, 1, vec![Some(Arc::from("")); rows])?
                        }
                        _ => self.str_convert(arg)?,
                    };
                    if let Some(rows) = matrix_rows
                        && text.len() != 1
                        && (text.rows(), text.cols()) != (rows, 1)
                    {
                        return Err(
                            "strcat: nonscalar inputs must have matching text dimensions".into(),
                        );
                    }
                    if matrix_rows.is_some() && arg.is_character() {
                        let items = text
                            .data()
                            .iter()
                            .map(|s| {
                                s.as_ref().map(|s| {
                                    Arc::from(s.trim_end_matches(super::strings::is_strcat_space))
                                })
                            })
                            .collect();
                        return StringArray::new(text.rows(), text.cols(), items);
                    }
                    Ok(text)
                };
                let mut s = operand(&args[0])?;
                for b in &args[1..] {
                    if matrix_rows.is_some() {
                        s = view(&self.str_binary(
                            &Token::Plus,
                            &value(s),
                            &value(operand(b)?),
                        )?)?;
                    } else {
                        s = view(&self.str_binary(&Token::Plus, &value(s), b)?)?;
                    }
                }
                value(s)
            }
            "upper" | "lower" | "strtrim" | "strip" | "contains" | "count" | "startsWith"
            | "endsWith" | "replace" | "strrep" | "extractBefore" | "extractAfter"
            | "extractBetween" | "insertBefore" | "insertAfter" | "erase" | "reverse" | "pad"
            | "regexprep" => self.str_map(name, args, out)?,
            _ => return Ok(None),
        };
        let result = if args.first().is_some_and(|v| !is_string(v))
            && matches!(
                name,
                "strip"
                    | "replace"
                    | "strrep"
                    | "regexprep"
                    | "split"
                    | "join"
                    | "compose"
                    | "extractBefore"
                    | "extractAfter"
                    | "extractBetween"
                    | "insertBefore"
                    | "insertAfter"
                    | "erase"
                    | "reverse"
                    | "pad"
            ) {
            self.str_restore_text_type(&args[0], result, matches!(name, "split" | "compose"))?
        } else {
            result
        };
        Ok(Some(result))
    }
}

impl Engine {
    // Pattern lists are applied to every subject, never broadcast with it.
    fn str_patterns(&mut self, name: &str, args: &[Value], out: &mut Execution) -> Result<Value> {
        let subject = self.str_convert(&args[0])?;
        let patterns = self.str_convert(&args[1])?;
        let boolean = matches!(name, "contains" | "count" | "startsWith" | "endsWith");
        let replacements = if boolean || name == "erase" {
            None
        } else {
            let s = self.str_convert(&args[2])?;
            if s.len() != 1 && s.len() != patterns.len() {
                return Err(format!(
                    "{name}: old and new patterns must have equal counts or a scalar replacement"
                ));
            }
            Some(s)
        };
        let ignore_case = if boolean {
            match &args[2..] {
                [] => false,
                [key, flag] if self.str_text(key)?.eq_ignore_ascii_case("IgnoreCase") => {
                    flag.clone().logical(&self.cancel)?.as_scalar()?
                }
                _ => return Err(format!("{name}: expected 'IgnoreCase', logical scalar")),
            }
        } else {
            false
        };
        let mut flags = Vec::new();
        let mut counts = Vec::new();
        let mut data = Vec::new();
        let mut bytes = 0;
        for item in subject.data() {
            self.check_cancel()?;
            let Some(s) = item else {
                counts.push(0.);
                if boolean {
                    flags.push(0);
                } else {
                    data.push(None);
                }
                continue;
            };
            if boolean {
                let lowered;
                let text = if ignore_case {
                    lowered = s.to_lowercase();
                    &lowered
                } else {
                    s.as_ref()
                };
                if name == "count" {
                    counts.push(self.str_count(text, patterns.data(), ignore_case)?);
                    continue;
                }
                let mut matched = false;
                for p in patterns.data().iter().flatten() {
                    self.check_cancel()?;
                    let lowered;
                    let p = if ignore_case {
                        lowered = p.to_lowercase();
                        &lowered
                    } else {
                        p.as_ref()
                    };
                    matched = match name {
                        "contains" => text.contains(p),
                        "startsWith" => text.starts_with(p),
                        _ => text.ends_with(p),
                    };
                    if matched {
                        break;
                    }
                }
                flags.push(u8::from(matched));
                continue;
            }
            let mut text = s.to_string();
            let mut missing = false;
            for (i, p) in patterns.data().iter().enumerate() {
                self.check_cancel()?;
                let replacement = replacements.as_ref().map_or(Some(""), |s| {
                    s.data()[if s.len() == 1 { 0 } else { i }].as_deref()
                });
                let (Some(p), Some(replacement)) = (p, replacement) else {
                    missing = true;
                    break;
                };
                text = if matches!(name, "replace" | "erase") {
                    checked_replace(&text, p, replacement)?
                } else {
                    let mut legacy = vec![
                        Value::Text(text),
                        Value::Text(p.to_string()),
                        Value::Text(replacement.to_owned()),
                    ];
                    for option in &args[3..] {
                        legacy.push(if is_string(option) {
                            Value::Text(self.str_text(option)?)
                        } else {
                            option.clone()
                        });
                    }
                    let result = if name == "strrep" {
                        self.text_format_call(name, legacy, out)?.expect("strrep")
                    } else {
                        self.strings2_outputs(name, &legacy, 1)?.remove(0)
                    };
                    self.text_value(result)?
                };
            }
            if missing {
                data.push(None);
            } else {
                str_charge_text(&mut bytes, text.len())?;
                data.push(Some(Arc::from(text)));
            }
        }
        if name == "count" {
            Ok(Value::Matrix(Matrix::from_column_vec(
                subject.rows(),
                subject.cols(),
                counts,
            )?))
        } else if boolean {
            Ok(Value::Logical(LogicalMatrix::from_column_vec(
                subject.rows(),
                subject.cols(),
                flags,
                &self.cancel,
            )?))
        } else {
            Ok(value(StringArray::new(
                subject.rows(),
                subject.cols(),
                data,
            )?))
        }
    }
    /// `matlab.lang.makeValidName(S, 'ReplacementStyle', style, 'Prefix', p)`.
    pub(super) fn make_valid_name_call(&mut self, args: &[Value]) -> Result<Value> {
        use super::readers::{Replacement, make_valid_name};
        const NAME: &str = "matlab.lang.makeValidName";
        if args.is_empty() || args.len().is_multiple_of(2) {
            return Err(format!(
                "{NAME}: expected {NAME}(S) or {NAME}(S, Name, Value, ...)"
            ));
        }
        let mut style = Replacement::Underscore;
        let mut prefix = "x".to_string();
        for pair in args[1..].chunks(2) {
            let key = self.str_text(&pair[0])?;
            let option = self.str_text(&pair[1])?;
            match key.to_ascii_lowercase().as_str() {
                "replacementstyle" => {
                    style = match option.to_ascii_lowercase().as_str() {
                        "underscore" => Replacement::Underscore,
                        "delete" => Replacement::Delete,
                        "hex" => Replacement::Hex,
                        _ => {
                            return Err(format!(
                                "{NAME}: ReplacementStyle must be 'underscore', 'delete' or 'hex'"
                            ));
                        }
                    }
                }
                "prefix" => {
                    if !crate::struct_array::valid_field_name(&option) {
                        return Err(format!("{NAME}: Prefix must be a valid variable name"));
                    }
                    prefix = option;
                }
                _ => return Err(format!("{NAME}: unknown option '{key}'")),
            }
        }
        let input = &args[0];
        let cell = matches!(input, Value::Container(Array::Cell(_)));
        if !(is_string(input) || cell || input.is_character()) {
            return Err(format!(
                "{NAME}: S must be a character vector, cellstr or string array"
            ));
        }
        let s = self.str_convert(input)?;
        let mut data = Vec::with_capacity(s.len());
        for text in s.data() {
            self.check_cancel()?;
            data.push(
                text.as_ref()
                    .map(|t| Arc::from(make_valid_name(t, style, &prefix))),
            );
        }
        let result = value(StringArray::new(s.rows(), s.cols(), data)?);
        if is_string(input) {
            Ok(result)
        } else {
            self.str_restore_text_type(input, result, false)
        }
    }
    /// count: occurrences scanned left to right; each character is matched at
    /// most once, trying the patterns in order at every position.
    fn str_count(
        &self,
        text: &str,
        patterns: &[Option<Arc<str>>],
        ignore_case: bool,
    ) -> Result<f64> {
        let patterns: Vec<String> = patterns
            .iter()
            .flatten()
            .filter(|p| !p.is_empty())
            .map(|p| {
                if ignore_case {
                    p.to_lowercase()
                } else {
                    p.to_string()
                }
            })
            .collect();
        let (mut at, mut n) = (0, 0.);
        while at < text.len() {
            self.check_cancel()?;
            let rest = &text[at..];
            match patterns.iter().find(|p| rest.starts_with(p.as_str())) {
                Some(p) => {
                    n += 1.;
                    at += p.len();
                }
                None => at += rest.chars().next().map_or(1, char::len_utf8),
            }
        }
        Ok(n)
    }
    fn str_map(&mut self, name: &str, args: &[Value], out: &mut Execution) -> Result<Value> {
        let (min, max) = match name {
            "upper" | "lower" | "strtrim" | "reverse" => (1, 1),
            "strip" => (1, 3),
            "pad" => (2, 4),
            "contains" | "count" | "startsWith" | "endsWith" => (2, 4),
            "replace" | "strrep" | "insertBefore" | "insertAfter" | "extractBetween" => (3, 3),
            "regexprep" => (3, usize::MAX),
            _ => (2, 2),
        };
        arity(name, args, min, max)?;
        if matches!(
            name,
            "contains"
                | "count"
                | "startsWith"
                | "endsWith"
                | "replace"
                | "strrep"
                | "regexprep"
                | "erase"
        ) {
            return self.str_patterns(name, args, out);
        }
        // Validate options once, including on empty arrays.
        let mut side = if name == "strip" && args.len() > 1 {
            self.str_text(&args[1])?
        } else if name == "pad" && args.len() > 2 {
            self.str_text(&args[2])?
        } else if name == "strip" {
            "both".into()
        } else {
            "right".into()
        };
        let mut trim_chars = if name == "strip" && args.len() > 2 {
            Some(self.str_text(&args[2])?)
        } else {
            None
        };
        let mut pad_char = if name == "pad" && args.len() > 3 {
            self.str_text(&args[3])?
        } else {
            " ".into()
        };
        if matches!(name, "strip" | "pad") && !matches!(side.as_str(), "left" | "right" | "both") {
            if side.chars().count() != 1 || args.len() != if name == "strip" { 2 } else { 3 } {
                return Err(format!("{name}: side must be left, right or both"));
            }
            if name == "strip" {
                trim_chars = Some(side);
                side = "both".into();
            } else {
                pad_char = side;
                side = "right".into();
            }
        }
        if name == "pad" && pad_char.encode_utf16().count() != 1 {
            return Err("pad: pad character must be a single character".into());
        }
        let width = if name == "pad" {
            let n = args[1].clone().matrix(&self.cancel)?.as_scalar()?;
            if !n.is_finite()
                || n < 0.
                || n.fract() != 0.
                || n > crate::string_array::MAX_STRING_BYTES as f64
            {
                return Err("pad: invalid width".into());
            }
            n as usize
        } else {
            0
        };
        let subject = self.str_convert(&args[0])?;
        let mut shape = (subject.rows(), subject.cols());
        // Extraction/insertion boundaries support scalar expansion along either axis.
        let pattern_count = match name {
            "insertBefore" | "insertAfter" | "extractBetween" => 2,
            "extractBefore" | "extractAfter" => 1,
            _ => 0,
        };
        if args.len() <= pattern_count {
            return Err(format!("{name}: too few inputs"));
        }
        let mut operands = vec![subject];
        for arg in args.iter().skip(1).take(pattern_count) {
            let s = self.str_convert(arg)?;
            let dummy = StringArray::new(
                shape.0,
                shape.1,
                vec![None; StringArray::checked_len(shape.0, shape.1)?],
            )?;
            shape = broadcast(&dummy, &s)?;
            operands.push(s);
        }
        if name == "extractBetween" && !str_position(&args[1]) && !str_position(&args[2]) {
            return self.str_extract_between_text(&operands, shape);
        }
        let mut result = Vec::with_capacity(shape.0 * shape.1);
        let mut bytes = 0usize;
        for c in 0..shape.1 {
            for r in 0..shape.0 {
                self.check_cancel()?;
                let texts = operands
                    .iter()
                    .map(|s| at(s, r, c).as_deref())
                    .collect::<Vec<_>>();
                if texts.iter().any(|s| s.is_none()) {
                    result.push(None);
                    continue;
                }
                let texts = texts.into_iter().map(Option::unwrap).collect::<Vec<_>>();
                let s = texts[0];
                let text = match name {
                    "upper" => s.to_uppercase(),
                    "lower" => s.to_lowercase(),
                    "strtrim" => s
                        .trim_matches(super::strings::is_trimmed_whitespace)
                        .to_owned(),
                    "reverse" => s.chars().rev().collect(),
                    "strip" => {
                        let trim = |c: char| {
                            trim_chars.as_ref().map_or_else(
                                || super::strings::is_trimmed_whitespace(c),
                                |s| s.contains(c),
                            )
                        };
                        match side.as_str() {
                            "left" => s.trim_start_matches(trim),
                            "right" => s.trim_end_matches(trim),
                            "both" => s.trim_matches(trim),
                            _ => return Err("strip: side must be left, right or both".into()),
                        }
                        .to_owned()
                    }
                    "pad" => {
                        let pad = &pad_char;
                        let n = width.saturating_sub(s.encode_utf16().count());
                        let (left, right) = match side.as_str() {
                            "left" => (n, 0),
                            "right" => (0, n),
                            "both" => (n / 2, n - n / 2),
                            _ => return Err("pad: side must be left, right or both".into()),
                        };
                        if n.saturating_mul(pad.len()).saturating_add(s.len())
                            > crate::string_array::MAX_STRING_BYTES
                        {
                            return Err("pad: result exceeds 2 MiB".into());
                        }
                        format!("{}{s}{}", pad.repeat(left), pad.repeat(right))
                    }
                    "extractBefore" | "extractAfter" | "extractBetween" | "insertBefore"
                    | "insertAfter" => {
                        // A surrogate pair occupies two positions. UTF-8 string storage
                        // cannot represent a substring ending inside that pair.
                        let mut bounds = Vec::with_capacity(s.encode_utf16().count() + 1);
                        for (i, ch) in s.char_indices() {
                            bounds.push(Some(i));
                            if ch.len_utf16() == 2 {
                                bounds.push(None);
                            }
                        }
                        bounds.push(Some(s.len()));
                        let boundary = |index: usize| -> Result<usize> {
                            bounds[index]
                                .ok_or("String position splits a UTF-16 surrogate pair".into())
                        };
                        let locate = |arg: usize, after: bool| -> Result<Option<usize>> {
                            if str_position(&args[arg]) {
                                let pos = texts[arg]
                                    .parse::<f64>()
                                    .map_err(|_| "Invalid string position")?;
                                let max = if name == "extractBetween" {
                                    bounds.len() - 1
                                } else {
                                    bounds.len()
                                };
                                if !pos.is_finite()
                                    || pos.fract() != 0.
                                    || pos < 1.
                                    || pos > max as f64
                                {
                                    return Err("String position is out of bounds".into());
                                }
                                let index = pos as usize - 1 + usize::from(after);
                                return Ok(Some(boundary(index.min(bounds.len() - 1))?));
                            }
                            Ok(s.find(texts[arg])
                                .map(|i| i + if after { texts[arg].len() } else { 0 }))
                        };
                        match name {
                            "extractBefore" => s[..locate(1, false)?.unwrap_or(0)].to_owned(),
                            "extractAfter" => {
                                let p = if str_position(&args[1]) {
                                    let pos = texts[1]
                                        .parse::<usize>()
                                        .map_err(|_| "Invalid string position")?;
                                    if pos >= bounds.len() {
                                        return Err("String position is out of bounds".into());
                                    }
                                    boundary(pos)?
                                } else {
                                    locate(1, true)?.unwrap_or(s.len())
                                };
                                s[p..].to_owned()
                            }
                            "extractBetween" => {
                                let start = if str_position(&args[1]) {
                                    locate(1, false)?
                                } else {
                                    locate(1, true)?
                                };
                                let stop = if str_position(&args[2]) {
                                    locate(2, true)?
                                } else {
                                    start.and_then(|a| s[a..].find(texts[2]).map(|b| a + b))
                                };
                                match (start, stop) {
                                    (Some(a), Some(b)) if a <= b => s[a..b].to_owned(),
                                    _ => String::new(),
                                }
                            }
                            _ => {
                                if !str_position(&args[1]) {
                                    let count = s.match_indices(texts[1]).count();
                                    if count
                                        .checked_mul(texts[2].len())
                                        .and_then(|n| n.checked_add(s.len()))
                                        .is_none_or(|n| n > crate::string_array::MAX_STRING_BYTES)
                                    {
                                        return Err("String result exceeds 2 MiB".into());
                                    }
                                    let mut text = String::new();
                                    let mut copied = 0;
                                    for (i, pattern) in s.match_indices(texts[1]) {
                                        self.check_cancel()?;
                                        let p = i + if name == "insertAfter" {
                                            pattern.len()
                                        } else {
                                            0
                                        };
                                        text.push_str(&s[copied..p]);
                                        text.push_str(texts[2]);
                                        copied = p;
                                    }
                                    text.push_str(&s[copied..]);
                                    str_charge_text(&mut bytes, text.len())?;
                                    result.push(Some(Arc::from(text)));
                                    continue;
                                }
                                let Some(p) = locate(1, name == "insertAfter")? else {
                                    str_charge_text(&mut bytes, s.len())?;
                                    result.push(Some(Arc::from(s)));
                                    continue;
                                };
                                if s.len().saturating_add(texts[2].len())
                                    > crate::string_array::MAX_STRING_BYTES
                                {
                                    return Err("String result exceeds 2 MiB".into());
                                }
                                format!("{}{}{}", &s[..p], texts[2], &s[p..])
                            }
                        }
                    }
                    _ => unreachable!(),
                };
                bytes = bytes
                    .checked_add(text.len())
                    .ok_or("String output overflow")?;
                if bytes > crate::string_array::MAX_STRING_BYTES {
                    return Err("String array text exceeds 2 MiB".into());
                }
                result.push(Some(Arc::from(text)));
            }
        }
        Ok(value(StringArray::new(shape.0, shape.1, result)?))
    }
    fn str_split(&self, args: &[Value]) -> Result<Value> {
        arity("split", args, 1, 4)?;
        let s = self.str_convert(&args[0])?;
        let (delimiter, options) = match &args[1..] {
            [] => (None, &args[1..]),
            [key, _]
                if self
                    .str_text(key)?
                    .eq_ignore_ascii_case("CollapseDelimiters") =>
            {
                (None, &args[1..])
            }
            [delimiter, rest @ ..] => (Some(self.str_text(delimiter)?), rest),
        };
        let collapse = match options {
            [] => true,
            [key, flag]
                if self
                    .str_text(key)?
                    .eq_ignore_ascii_case("CollapseDelimiters") =>
            {
                flag.clone().logical(&self.cancel)?.as_scalar()?
            }
            _ => return Err("split: expected 'CollapseDelimiters', logical scalar".into()),
        };
        if s.is_empty() {
            return Ok(value(s));
        }
        let mut count = None;
        let mut parts = Vec::with_capacity(s.len());
        for item in s.data() {
            self.check_cancel()?;
            let Some(text) = item else {
                parts.push(None);
                continue;
            };
            let text = if delimiter.is_none() && collapse {
                text.trim()
            } else {
                text.as_ref()
            };
            let mut pieces = Vec::new();
            if delimiter.as_deref() == Some("") && !text.is_empty() {
                for (i, ch) in text.char_indices() {
                    self.check_cancel()?;
                    StringArray::checked_len(pieces.len() + 1, s.len())?;
                    pieces.push(Some(Arc::from(&text[i..i + ch.len_utf8()])));
                }
            } else {
                let matches: Box<dyn Iterator<Item = (usize, usize)>> = match &delimiter {
                    Some(d) if !d.is_empty() => {
                        Box::new(text.match_indices(d).map(|(i, d)| (i, d.len())))
                    }
                    _ => Box::new(
                        text.char_indices()
                            .filter(|(_, ch)| ch.is_whitespace())
                            .map(|(i, ch)| (i, ch.len_utf8())),
                    ),
                };
                let mut copied = 0;
                let mut previous = false;
                for (i, len) in matches {
                    self.check_cancel()?;
                    if !(collapse && previous && i == copied) {
                        StringArray::checked_len(pieces.len() + 1, s.len())?;
                        pieces.push(Some(Arc::from(&text[copied..i])));
                    }
                    copied = i + len;
                    previous = true;
                }
                StringArray::checked_len(pieces.len() + 1, s.len())?;
                pieces.push(Some(Arc::from(&text[copied..])));
            }
            let n = pieces.len();
            StringArray::checked_len(n, s.len())?;
            if count.is_some_and(|old| old != n) {
                return Err(
                    "split: every element must split into the same number of pieces".into(),
                );
            }
            count = Some(n);
            parts.push(Some(pieces));
        }
        let count = count.unwrap_or(usize::from(!s.is_empty()));
        if s.cols() > 1 && count > 1 {
            return Err(
                "split: a row or matrix result requires N-D string storage, which is not supported yet".into(),
            );
        }
        let (rows, cols) = if s.len() == 1 {
            (count, 1)
        } else if count == 1 {
            (s.rows(), s.cols())
        } else {
            (s.rows(), count)
        };
        let mut data = vec![None; StringArray::checked_len(rows, cols)?];
        for (i, p) in parts.into_iter().enumerate() {
            let Some(p) = p else {
                continue;
            };
            for (j, v) in p.into_iter().enumerate() {
                let pos = if s.len() == 1 { j } else { j * rows + i };
                data[pos] = v;
            }
        }
        Ok(value(StringArray::new(rows, cols, data)?))
    }
    fn str_join(&self, name: &str, args: &[Value]) -> Result<Value> {
        arity(name, args, 1, 2)?;
        let s = self.str_convert(&args[0])?;
        let mut delimiter = if args.len() == 2 {
            self.str_text(&args[1])?
        } else {
            " ".into()
        };
        if name == "strjoin" {
            delimiter = super::text_format::unescape(&delimiter);
        }
        let horizontal = name == "strjoin" || s.cols() != 1;
        let (rows, cols) = if name == "strjoin" {
            (1, 1)
        } else if horizontal {
            (s.rows(), 1)
        } else {
            (1, s.cols())
        };
        let mut data = Vec::with_capacity(rows * cols);
        let mut total_bytes = 0;
        for group in 0..rows * cols {
            self.check_cancel()?;
            let indices = if name == "strjoin" {
                (0..s.len()).collect::<Vec<_>>()
            } else if horizontal {
                (0..s.cols()).map(|c| c * s.rows() + group).collect()
            } else {
                (0..s.rows()).collect()
            };
            let mut text = String::new();
            let mut missing = false;
            for (i, k) in indices.into_iter().enumerate() {
                self.check_cancel()?;
                let Some(part) = &s.data()[k] else {
                    missing = true;
                    break;
                };
                let extra = part.len() + if i > 0 { delimiter.len() } else { 0 };
                if text.len().saturating_add(extra) > crate::string_array::MAX_STRING_BYTES {
                    return Err("join: output exceeds 2 MiB".into());
                }
                if i > 0 {
                    text.push_str(&delimiter);
                }
                text.push_str(part);
            }
            if !missing {
                str_charge_text(&mut total_bytes, text.len())?;
            }
            data.push(if missing { None } else { Some(Arc::from(text)) });
        }
        Ok(value(StringArray::new(rows, cols, data)?))
    }
    fn str_extract_between_text(
        &self,
        operands: &[StringArray],
        shape: (usize, usize),
    ) -> Result<Value> {
        let len = StringArray::checked_len(shape.0, shape.1)?;
        let mut parts = Vec::with_capacity(len);
        let mut count = None;
        let mut bytes = 0;
        for c in 0..shape.1 {
            for r in 0..shape.0 {
                self.check_cancel()?;
                let (Some(text), Some(start), Some(stop)) = (
                    at(&operands[0], r, c),
                    at(&operands[1], r, c),
                    at(&operands[2], r, c),
                ) else {
                    parts.push(None);
                    continue;
                };
                let mut pieces = Vec::new();
                let mut copied = 0;
                for (i, matched) in text.match_indices(start.as_ref()) {
                    self.check_cancel()?;
                    if i < copied {
                        continue;
                    }
                    let begin = i + matched.len();
                    let Some(end) = text[begin..].find(stop.as_ref()).map(|j| begin + j) else {
                        break;
                    };
                    StringArray::checked_len(pieces.len() + 1, len)?;
                    str_charge_text(&mut bytes, end - begin)?;
                    pieces.push(Some(Arc::from(&text[begin..end])));
                    copied = end + stop.len();
                }
                if count.is_some_and(|n| n != pieces.len()) {
                    return Err(
                        "extractBetween: every element must have the same number of matches".into(),
                    );
                }
                count = Some(pieces.len());
                parts.push(Some(pieces));
            }
        }
        let count = count.unwrap_or(usize::from(len != 0));
        if shape.1 > 1 && count > 1 {
            return Err("extractBetween: a row or matrix result requires N-D string storage, which is not supported yet".into());
        }
        let (rows, cols) = if len == 1 {
            (count, 1)
        } else if count == 1 || len == 0 {
            shape
        } else {
            (shape.0, count)
        };
        let mut data = vec![None; StringArray::checked_len(rows, cols)?];
        for (i, pieces) in parts.into_iter().enumerate() {
            if let Some(pieces) = pieces {
                for (j, piece) in pieces.into_iter().enumerate() {
                    data[if len == 1 { j } else { j * rows + i }] = piece;
                }
            }
        }
        Ok(value(StringArray::new(rows, cols, data)?))
    }
    fn str_compose(&mut self, name: &str, args: &[Value], out: &mut Execution) -> Result<Value> {
        arity(name, args, 1, usize::MAX)?;
        let fmt = self.str_convert(&args[0])?;
        if name == "sprintf" {
            if fmt.len() != 1 {
                return Err("sprintf: format must be scalar".into());
            }
            let Some(format) = &fmt.data()[0] else {
                return Ok(value(fmt));
            };
            let mut legacy = vec![Value::Text(format.to_string())];
            for arg in &args[1..] {
                legacy.push(if let Value::Container(Array::Typed(a)) = arg {
                    Value::Matrix(a.to_matrix()?)
                } else {
                    arg.clone()
                });
            }
            let v = self
                .text_format_call("sprintf", legacy, out)?
                .expect("sprintf");
            return Ok(value(StringArray::scalar(self.text_value(v)?)?));
        }
        // With one data array, a format consumes successive columns of each row.
        // A single conversion therefore retains the existing elementwise shape.
        let mut row_inputs = 1;
        if args.len() == 2 && !args[1].is_character() {
            let mut expected = None;
            for format in fmt.data().iter().flatten() {
                let inputs = str_compose_input_count(format, &self.cancel)?.max(1);
                if expected.is_some_and(|n| n != inputs) {
                    return Err(
                        "compose: formats must consume the same number of row entries".into(),
                    );
                }
                expected = Some(inputs);
            }
            row_inputs = expected.unwrap_or(1);
        }
        let mut shape = (fmt.rows(), fmt.cols());
        for arg in &args[1..] {
            let mut dims = arg.dims().to_vec();
            if dims.len() != 2 {
                return Err("compose: only 2-D arguments are supported".into());
            }
            if !dims[1].is_multiple_of(row_inputs) {
                return Err(
                    "compose: data columns must be a multiple of the format input count".into(),
                );
            }
            dims[1] /= row_inputs;
            let dummy = StringArray::new(
                shape.0,
                shape.1,
                vec![None; StringArray::checked_len(shape.0, shape.1)?],
            )?;
            let other = if arg.is_character() {
                StringArray::scalar(self.str_text(arg)?)?
            } else {
                StringArray::new(
                    dims[0],
                    dims[1],
                    vec![None; StringArray::checked_len(dims[0], dims[1])?],
                )?
            };
            shape = broadcast(&dummy, &other)?;
        }
        let mut data = Vec::with_capacity(shape.0 * shape.1);
        let mut total_bytes = 0;
        for c in 0..shape.1 {
            for r in 0..shape.0 {
                self.check_cancel()?;
                let Some(format) = at(&fmt, r, c) else {
                    data.push(None);
                    continue;
                };
                let mut legacy = vec![Value::Text(format.to_string())];
                let mut missing = false;
                for arg in &args[1..] {
                    for offset in 0..row_inputs {
                        let column = c * row_inputs + offset;
                        if is_string(arg) {
                            let s = view(arg)?;
                            match at(&s, r, column) {
                                Some(s) => legacy.push(Value::Text(s.to_string())),
                                None => {
                                    missing = true;
                                    break;
                                }
                            }
                        } else if arg.is_character() {
                            legacy.push(Value::Text(self.str_text(arg)?));
                        } else {
                            let a = arg.clone().array(&self.cancel)?;
                            let scalar = a.scalar(r % a.rows(), column % a.cols())?;
                            legacy.push(match scalar {
                                crate::Scalar::Real(v) => Value::Matrix(Matrix::scalar(v)),
                                crate::Scalar::Integer(v) => {
                                    Value::Matrix(Matrix::scalar(v as f64))
                                }
                                crate::Scalar::Logical(v) => {
                                    Value::Logical(LogicalMatrix::scalar(v))
                                }
                                _ => return Err("compose: unsupported numeric class".into()),
                            });
                        }
                    }
                }
                if missing {
                    data.push(None);
                    continue;
                }
                let v = self
                    .text_format_call("sprintf", legacy, out)?
                    .expect("sprintf");
                let text = self.text_value(v)?;
                str_charge_text(&mut total_bytes, text.len())?;
                data.push(Some(Arc::from(text)));
            }
        }
        Ok(value(StringArray::new(shape.0, shape.1, data)?))
    }
    pub(super) fn str_regexp(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
    ) -> Result<Vec<Value>> {
        arity(name, args, 2, usize::MAX)?;
        if !(1..=7).contains(&count) {
            return Err("regexp supports one to seven outputs".into());
        }
        let s = view(&args[0])?;
        let pattern = self.str_convert(&args[1])?;
        let (rows, cols) = broadcast(&s, &pattern)?;
        let mut options = Vec::with_capacity(args.len().saturating_sub(2));
        for arg in &args[2..] {
            options.push(if is_string(arg) {
                Value::Text(self.str_text(arg)?)
            } else {
                arg.clone()
            });
        }
        let default_order = [
            "start",
            "end",
            "tokenextents",
            "match",
            "tokens",
            "names",
            "split",
        ];
        let mut order = Vec::new();
        for option in &options {
            let text = self.str_text(option)?.to_ascii_lowercase();
            if default_order.contains(&text.as_str()) {
                order.push(text);
            }
        }
        for name in default_order {
            if !order.iter().any(|s| s == name) {
                order.push(name.to_owned());
            }
        }
        // Validate even an empty array's pattern/options through the old kernel.
        if rows == 0 || cols == 0 {
            let mut legacy = vec![
                Value::Text(String::new()),
                Value::Text(
                    pattern
                        .data()
                        .first()
                        .and_then(Option::as_deref)
                        .unwrap_or("")
                        .into(),
                ),
            ];
            legacy.extend(options.iter().cloned());
            self.strings2_outputs(name, &legacy, count)?;
        }
        let mut outputs = vec![Vec::with_capacity(rows * cols); count];
        let mut bytes = 0usize;
        for c in 0..cols {
            for r in 0..rows {
                self.check_cancel()?;
                let item = at(&s, r, c);
                let pat = at(&pattern, r, c);
                let absent = item.is_none() || pat.is_none();
                let mut legacy = vec![
                    Value::Text(item.as_deref().unwrap_or("").into()),
                    Value::Text(pat.as_deref().unwrap_or("").into()),
                ];
                legacy.extend(options.iter().cloned());
                let result = self.strings2_outputs(name, &legacy, count)?;
                for ((items, v), kind) in outputs.iter_mut().zip(result).zip(&order) {
                    let a = v.array(&self.cancel)?;
                    let a = if matches!(kind.as_str(), "match" | "tokens" | "names" | "split") {
                        str_regex_result(a, absent, &self.cancel)?
                    } else {
                        a
                    };
                    bytes = bytes
                        .checked_add(a.retained_bytes())
                        .ok_or("regexp output overflow")?;
                    if bytes > 256 * 1024 * 1024 {
                        return Err("regexp output exceeds 256 MiB".into());
                    }
                    items.push(Arc::new(a));
                }
            }
        }
        outputs
            .into_iter()
            .map(|mut items| {
                if rows * cols == 1 {
                    Ok(items.remove(0).as_ref().clone().into())
                } else {
                    Ok(Value::Container(
                        CellArray::from_shared(rows, cols, items, &self.cancel)?.into(),
                    ))
                }
            })
            .collect()
    }
}
pub(super) fn str_charge_text(total: &mut usize, bytes: usize) -> Result<()> {
    *total = total.checked_add(bytes).ok_or("String output overflow")?;
    if *total > crate::string_array::MAX_STRING_BYTES {
        return Err("String array text exceeds 2 MiB".into());
    }
    Ok(())
}
fn checked_replace(s: &str, old: &str, new: &str) -> Result<String> {
    let count = s.matches(old).count();
    if count
        .checked_mul(new.len())
        .and_then(|n| n.checked_add(s.len() - count * old.len()))
        .is_none_or(|n| n > crate::string_array::MAX_STRING_BYTES)
    {
        return Err("String replacement exceeds 2 MiB".into());
    }
    Ok(s.replace(old, new))
}
fn str_regex_result(
    a: Array,
    missing: bool,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Array> {
    crate::dsp::check_cancel(cancel)?;
    match a {
        Array::Character(c) => Ok(Array::Str(if missing {
            StringArray::new(1, 1, vec![None])?
        } else {
            StringArray::scalar(c.to_utf8(cancel)?)?
        })),
        Array::Cell(c)
            if (0..c.len())
                .all(|i| matches!(c.get_linear(i).map(AsRef::as_ref), Ok(Array::Character(_)))) =>
        {
            if missing {
                return Ok(Array::Str(StringArray::new(1, 1, vec![None])?));
            }
            let mut data = Vec::with_capacity(c.len());
            for i in 0..c.len() {
                let Array::Character(text) = c.get_linear(i)?.as_ref() else {
                    unreachable!()
                };
                data.push(Some(Arc::from(text.to_utf8(cancel)?)));
            }
            Ok(Array::Str(StringArray::new(c.rows(), c.cols(), data)?))
        }
        Array::Cell(c) => {
            let mut data = Vec::with_capacity(c.len());
            for i in 0..c.len() {
                data.push(Arc::new(str_regex_result(
                    c.get_linear(i)?.as_ref().clone(),
                    missing,
                    cancel,
                )?));
            }
            Ok(Array::Cell(CellArray::from_shared(
                c.rows(),
                c.cols(),
                data,
                cancel,
            )?))
        }
        Array::Struct(s) => {
            let mut data = Vec::with_capacity(s.len() * s.field_count());
            for element in 0..s.len() {
                for field in 0..s.field_count() {
                    data.push(Arc::new(str_regex_result(
                        s.get(field, element)?.as_ref().clone(),
                        missing,
                        cancel,
                    )?));
                }
            }
            Ok(Array::Struct(crate::StructArray::from_slots(
                s.rows(),
                s.cols(),
                s.field_names().to_vec(),
                data,
                cancel,
            )?))
        }
        Array::Numeric(_) if missing => {
            Ok(Array::Numeric(Matrix::from_column_vec(1, 0, Vec::new())?))
        }
        a => Ok(a),
    }
}

impl Engine {
    pub(super) fn str_index(
        &mut self,
        base: Value,
        args: &[Expr],
        content: bool,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let s = view(&base)?;
        let (shape, positions) = self.container_positions(&Array::Str(s.clone()), args, out)?;
        if content {
            let mut bytes = 0usize;
            for &i in &positions {
                self.check_cancel()?;
                let units = s.data()[i]
                    .as_deref()
                    .unwrap_or("<missing>")
                    .encode_utf16()
                    .count();
                bytes = bytes
                    .checked_add(units * 2)
                    .ok_or("String content output overflow")?;
                if bytes > 256 * 1024 * 1024 {
                    return Err("String content output exceeds 256 MiB".into());
                }
            }
            self.charge(bytes.div_ceil(8))?;
            positions
                .into_iter()
                .map(|i| {
                    self.check_cancel()?;
                    Ok(Value::Container(
                        CharacterRow::from_text(
                            s.data()[i].as_deref().unwrap_or("<missing>"),
                            &self.cancel,
                        )?
                        .into(),
                    ))
                })
                .collect()
        } else {
            Ok(vec![value(StringArray::new(
                shape.0,
                shape.1,
                positions.into_iter().map(|i| s.data()[i].clone()).collect(),
            )?)])
        }
    }
    pub(super) fn str_assign(
        &mut self,
        old: Option<StringArray>,
        args: &[Expr],
        replacement: Value,
        deletion: bool,
        out: &mut Execution,
    ) -> Result<StringArray> {
        if args.is_empty() || args.len() > 2 {
            return Err("String assignment supports one or two subscripts".into());
        }
        let has_end = old.is_some();
        let old = match old {
            Some(s) => s,
            None => StringArray::new(0, 0, vec![])?,
        };
        let (r, c) = (old.rows(), old.cols());
        let first = self
            .cel_subscript(
                &args[0],
                has_end.then_some(if args.len() == 1 { old.len() } else { r }),
                crate::string_array::MAX_STRINGS,
                out,
            )?
            .0;
        let second = if args.len() == 2 {
            Some(
                self.cel_subscript(
                    &args[1],
                    has_end.then_some(c),
                    crate::string_array::MAX_STRINGS,
                    out,
                )?
                .0,
            )
        } else {
            None
        };
        let extent =
            |i: &super::indexing::Indices| (0..i.len()).map(|k| i.at(k) + 1).max().unwrap_or(0);
        let (er, ec) = (extent(&first), second.as_ref().map_or(0, extent));
        if deletion {
            if er > if args.len() == 1 { old.len() } else { r } || (second.is_some() && ec > c) {
                return Err("String deletion index is out of bounds".into());
            }
            if args.len() == 2 && !args.iter().any(|a| self.colon_subscript(a)) {
                return Err("A null assignment can have only one non-colon index".into());
            }
            let remove_rows = args.len() == 2 && !self.colon_subscript(&args[0]);
            let n = if args.len() == 1 {
                old.len()
            } else if remove_rows {
                r
            } else {
                c
            };
            let sub = if args.len() == 1 || remove_rows {
                &first
            } else {
                second.as_ref().expect("second index")
            };
            let mut removed = vec![false; n];
            for k in 0..sub.len() {
                removed[sub.at(k)] = true;
            }
            let kept = (0..n).filter(|&i| !removed[i]).collect::<Vec<_>>();
            let shape = if args.len() == 1 {
                if self.colon_subscript(&args[0]) {
                    (0, 0)
                } else if c == 1 && r != 1 {
                    (kept.len(), 1)
                } else {
                    (1, kept.len())
                }
            } else if remove_rows {
                (kept.len(), c)
            } else {
                (r, kept.len())
            };
            let mut data = Vec::with_capacity(shape.0 * shape.1);
            for i in 0..shape.0 * shape.1 {
                self.check_cancel()?;
                let source = if args.len() == 1 {
                    kept[i]
                } else if remove_rows {
                    kept[i % shape.0] + i / shape.0 * r
                } else {
                    i % r + kept[i / r] * r
                };
                data.push(old.data()[source].clone());
            }
            return StringArray::new(shape.0, shape.1, data);
        }
        let count = first
            .len()
            .checked_mul(second.as_ref().map_or(1, super::indexing::Indices::len))
            .ok_or("String assignment size overflow")?;
        StringArray::checked_len(count, 1)?;
        let replacement = self.str_convert(&replacement)?;
        if replacement.len() != 1 && replacement.len() != count {
            return Err("String assignment dimension mismatch".into());
        }
        if count == 0 {
            return Ok(old);
        }
        let shape = if second.is_some() {
            (r.max(er), c.max(ec))
        } else if er <= old.len() {
            (r, c)
        } else if r <= 1 {
            (1, er)
        } else if c == 1 {
            (er, 1)
        } else {
            return Err("Cannot grow a matrix through one ambiguous linear subscript".into());
        };
        let mut data = vec![None; StringArray::checked_len(shape.0, shape.1)?];
        for col in 0..c {
            for row in 0..r {
                self.check_cancel()?;
                data[col * shape.0 + row] = old.data()[col * r + row].clone();
            }
        }
        for k in 0..count {
            self.check_cancel()?;
            let i = if let Some(second) = &second {
                second.at(k / first.len()) * shape.0 + first.at(k % first.len())
            } else {
                first.at(k)
            };
            data[i] = replacement.data()[if replacement.len() == 1 { 0 } else { k }].clone();
        }
        StringArray::new(shape.0, shape.1, data)
    }
}
