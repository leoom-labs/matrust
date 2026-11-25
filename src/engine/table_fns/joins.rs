//! Key joins of two tables: join (each left row matches exactly one right
//! row, left order), innerjoin (matching rows) and outerjoin (full, left or
//! right; unmatched rows get missing values), both sorted by key values.
use super::super::{Engine, Execution, Value};
use super::groups::{K, KeyKind, key_kind, key_kinds_compatible, promote_int_keys, row_keys};
use super::{row_expr, split_options, table_of, text_scalar, tvalue};
use crate::syntax::Expr;
use crate::table_array::{TableArray, TableParts};
use crate::{Array, LogicalMatrix, Matrix, Result};
use std::collections::HashMap;

/// Combined key of each row, plus how many elements each variable appended.
/// A missing component stays in the vector (`K::Missing` sorts last).
fn raw_keys(
    t: &TableArray,
    vars: &[usize],
    who: &str,
    by_text: bool,
) -> Result<(Vec<Vec<K>>, Vec<usize>)> {
    let mut combined: Vec<Vec<K>> = vec![Vec::new(); t.height()];
    let mut widths = Vec::with_capacity(vars.len());
    for &v in vars {
        let parts = row_keys(t.column(v), t.height(), who, by_text)?;
        widths.push(parts.first().map_or(0, Vec::len));
        for (row, k) in combined.iter_mut().zip(parts) {
            row.extend(k);
        }
    }
    Ok((combined, widths))
}

/// Drop rows that contain a missing component from matching. Two missing keys
/// are not equal.
fn present_keys(rows: &[Vec<K>]) -> Vec<Option<Vec<K>>> {
    rows.iter()
        .map(|k| (!k.contains(&K::Missing)).then(|| k.clone()))
        .collect()
}

/// Integer columns paired with a float column compare as doubles.
fn align_numeric(
    rows: &mut [Vec<K>],
    widths: &[usize],
    kinds: &[(KeyKind, KeyKind)],
    left_side: bool,
) {
    let mut start = 0usize;
    for (i, &width) in widths.iter().enumerate() {
        let (mine, other) = if left_side {
            (kinds[i].0, kinds[i].1)
        } else {
            (kinds[i].1, kinds[i].0)
        };
        if mine == KeyKind::Integer && other == KeyKind::Float {
            promote_int_keys(rows, start, width);
        }
        start += width;
    }
}

/// Most rows a join may produce.
const MAX_PAIRS: usize = 50_000_000;

fn index_column(list: &[Option<usize>]) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(
        list.len(),
        1,
        list.iter()
            .map(|i| i.map_or(0., |i| (i + 1) as f64))
            .collect(),
    )?))
}

fn flag(engine: &Engine, v: &Value) -> Result<bool> {
    Ok(v.clone()
        .matrix(&engine.cancel)?
        .data()
        .first()
        .is_some_and(|x| *x != 0.))
}

impl Engine {
    pub(super) fn tbl_keyjoin(
        &mut self,
        who: &str,
        args: &[Value],
        names: &[String],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        let (data, options) = split_options(
            args,
            &[
                "Keys",
                "LeftKeys",
                "RightKeys",
                "LeftVariables",
                "RightVariables",
                "MergeKeys",
                "Type",
                "KeepOneCopy",
            ],
        );
        let [lv, rv] = data else {
            return Err(format!("{who}: expected two tables and options"));
        };
        let (Some(left), Some(right)) = (table_of(lv).cloned(), table_of(rv).cloned()) else {
            return Err(format!("{who}: both inputs must be tables"));
        };
        if who == "join" && count > 2 || count > 3 {
            return Err(format!("{who}: too many output arguments"));
        }
        let (mut lk, mut rk, mut lvars, mut rvars) = (None, None, None, None);
        let (mut merge, mut kind) = (false, "full".to_owned());
        let mut keep_one: Vec<String> = Vec::new();
        for (o, v) in &options {
            match o.as_str() {
                "Keys" => {
                    let l = self.tbl_vars_of(&left, v)?;
                    let names: Vec<String> = l.iter().map(|&i| left.names()[i].clone()).collect();
                    rk = Some(
                        names
                            .iter()
                            .map(|n| right.index_of(n).ok_or_else(|| super::unknown_variable(n)))
                            .collect::<Result<Vec<_>>>()?,
                    );
                    lk = Some(l);
                }
                "LeftKeys" => lk = Some(self.tbl_vars_of(&left, v)?),
                "RightKeys" => rk = Some(self.tbl_vars_of(&right, v)?),
                "LeftVariables" => lvars = Some(self.tbl_vars_of(&left, v)?),
                "RightVariables" => rvars = Some(self.tbl_vars_of(&right, v)?),
                "MergeKeys" => merge = flag(self, v)?,
                "Type" if who == "outerjoin" => {
                    kind = text_scalar(v)
                        .ok_or("outerjoin: Type must be text")?
                        .to_ascii_lowercase();
                    if !matches!(kind.as_str(), "full" | "left" | "right") {
                        return Err(format!("outerjoin: unknown Type '{kind}'"));
                    }
                }
                "KeepOneCopy" if who == "join" => {
                    keep_one = super::text_list(self, v, "KeepOneCopy")?
                        .ok_or("join: KeepOneCopy must be variable names")?;
                }
                _ => return Err(format!("{who}: the '{o}' option is not supported")),
            }
        }
        if merge && who != "outerjoin" {
            return Err(format!("{who}: MergeKeys is an outerjoin option"));
        }
        let (lk, rk) = match (lk, rk) {
            (Some(l), Some(r)) => (l, r),
            (None, None) => {
                let l: Vec<usize> = (0..left.width())
                    .filter(|&i| right.index_of(&left.names()[i]).is_some())
                    .collect();
                if l.is_empty() {
                    return Err(format!(
                        "{who}: the tables have no common variable to use as a key; give 'Keys'"
                    ));
                }
                let r = l
                    .iter()
                    .map(|&i| right.index_of(&left.names()[i]).expect("common"))
                    .collect();
                (l, r)
            }
            _ => return Err(format!("{who}: give both LeftKeys and RightKeys")),
        };
        if lk.len() != rk.len() {
            return Err(format!(
                "{who}: LeftKeys and RightKeys must name the same number of variables"
            ));
        }
        if lk.is_empty() {
            return Err(format!("{who}: Keys must name at least one variable"));
        }
        let kinds: Vec<(KeyKind, KeyKind)> = lk
            .iter()
            .zip(&rk)
            .map(|(&a, &b)| {
                Ok((
                    key_kind(left.column(a), who)?,
                    key_kind(right.column(b), who)?,
                ))
            })
            .collect::<Result<_>>()?;
        for &(a, b) in &kinds {
            if !key_kinds_compatible(a, b) {
                return Err(format!("{who}: incompatible key types"));
            }
        }
        let (mut lraw, lwidths) = raw_keys(&left, &lk, who, true)?;
        let (mut rraw, rwidths) = raw_keys(&right, &rk, who, true)?;
        align_numeric(&mut lraw, &lwidths, &kinds, true);
        align_numeric(&mut rraw, &rwidths, &kinds, false);
        let lkeys = present_keys(&lraw);
        let rkeys = present_keys(&rraw);
        // Output order follows category order when both sides' categorical
        // keys share one category list, else the key text. Sort keys keep
        // missing components so NaN is last only inside its column.
        let same_categories =
            lk.iter()
                .zip(&rk)
                .all(|(&a, &b)| match (left.column(a), right.column(b)) {
                    (Array::Categorical(x), Array::Categorical(y)) => {
                        x.categories() == y.categories()
                    }
                    (Array::Categorical(_), _) | (_, Array::Categorical(_)) => false,
                    _ => true,
                });
        let (lsort, rsort) = if same_categories {
            let (mut ls, lw) = raw_keys(&left, &lk, who, false)?;
            let (mut rs, rw) = raw_keys(&right, &rk, who, false)?;
            align_numeric(&mut ls, &lw, &kinds, true);
            align_numeric(&mut rs, &rw, &kinds, false);
            (ls, rs)
        } else {
            (lraw, rraw)
        };
        let mut by_key: HashMap<&Vec<K>, Vec<usize>> = HashMap::new();
        for (j, k) in rkeys.iter().enumerate() {
            if let Some(k) = k {
                by_key.entry(k).or_default().push(j);
            }
        }
        // Row pairs (left, right) of the result, in output order.
        let mut pairs: Vec<(Option<usize>, Option<usize>)> = Vec::new();
        if who == "join" {
            for (i, k) in lkeys.iter().enumerate() {
                let hits = k
                    .as_ref()
                    .and_then(|k| by_key.get(k))
                    .map_or(&[][..], Vec::as_slice);
                match hits {
                    [j] => pairs.push((Some(i), Some(*j))),
                    [] => {
                        return Err(format!(
                            "join: the key of left row {} has no match in the right table",
                            i + 1
                        ));
                    }
                    _ => {
                        return Err(format!(
                            "join: the key of left row {} matches several right rows; use innerjoin",
                            i + 1
                        ));
                    }
                }
            }
        } else {
            let mut total = 0usize;
            for k in lkeys.iter().flatten() {
                total += by_key.get(k).map_or(0, Vec::len);
            }
            if total > MAX_PAIRS {
                return Err(format!(
                    "{who}: the result would have {total} rows (at most {MAX_PAIRS}); the keys repeat too often"
                ));
            }
            let mut matched_right = vec![false; right.height()];
            for (i, k) in lkeys.iter().enumerate() {
                if i % super::CANCEL_STRIDE == 0 {
                    self.check_cancel()?;
                }
                let hits = k
                    .as_ref()
                    .and_then(|k| by_key.get(k))
                    .map_or(&[][..], Vec::as_slice);
                for &j in hits {
                    pairs.push((Some(i), Some(j)));
                    matched_right[j] = true;
                }
                if hits.is_empty() && who == "outerjoin" && kind != "right" {
                    pairs.push((Some(i), None));
                }
            }
            if who == "outerjoin" && kind != "left" {
                for j in (0..right.height()).filter(|&j| !matched_right[j]) {
                    pairs.push((None, Some(j)));
                }
            }
            // Lexicographic key order. K::Missing sorts last inside its column,
            // so a NaN in a later column does not sink the whole row.
            pairs.sort_by(|a, b| {
                let ka: &[K] = match a.0 {
                    Some(i) => &lsort[i],
                    None => match a.1 {
                        Some(j) => &rsort[j],
                        None => &[],
                    },
                };
                let kb: &[K] = match b.0 {
                    Some(i) => &lsort[i],
                    None => match b.1 {
                        Some(j) => &rsort[j],
                        None => &[],
                    },
                };
                ka.cmp(kb)
                    .then(a.0.unwrap_or(usize::MAX).cmp(&b.0.unwrap_or(usize::MAX)))
                    .then(a.1.cmp(&b.1))
            });
        }
        let lvars = lvars.unwrap_or_else(|| (0..left.width()).collect());
        let rvars = rvars.unwrap_or_else(|| {
            (0..right.width())
                .filter(|j| who == "outerjoin" && !merge || !rk.contains(j))
                .collect()
        });
        let rvars: Vec<usize> = if merge {
            rvars.into_iter().filter(|j| !rk.contains(j)).collect()
        } else {
            rvars
        };
        let rvars: Vec<usize> = rvars
            .into_iter()
            .filter(|&j| !keep_one.contains(&right.names()[j]))
            .collect();
        let li: Vec<Option<usize>> = pairs.iter().map(|p| p.0).collect();
        let ri: Vec<Option<usize>> = pairs.iter().map(|p| p.1).collect();
        let n = pairs.len();
        let mut lnames: Vec<String> = lvars.iter().map(|&v| left.names()[v].clone()).collect();
        let mut rnames: Vec<String> = rvars.iter().map(|&v| right.names()[v].clone()).collect();
        let suffix = |k: usize, fallback: &str| match names.get(k) {
            Some(n) if !n.is_empty() => n.clone(),
            _ => fallback.to_owned(),
        };
        let (ls, rs) = (suffix(0, "left"), suffix(1, "right"));
        for (k, &v) in lvars.iter().enumerate() {
            if merge && let Some(p) = lk.iter().position(|&x| x == v) {
                let rname = &right.names()[rk[p]];
                if *rname != lnames[k] {
                    lnames[k] = format!("{}_{rname}", lnames[k]);
                }
            }
        }
        let clash: Vec<String> = lnames
            .iter()
            .filter(|n| rnames.contains(n))
            .cloned()
            .collect();
        for n in lnames.iter_mut() {
            if clash.contains(n) {
                *n = format!("{n}_{ls}");
            }
        }
        for n in rnames.iter_mut() {
            if clash.contains(n) {
                *n = format!("{n}_{rs}");
            }
        }
        let mut parts = TableParts::new(Vec::new(), Vec::new(), n);
        for (k, &v) in lvars.iter().enumerate() {
            let mut col = self.join_take(&left, v, &li, who, out)?;
            if merge && let Some(p) = lk.iter().position(|&x| x == v) {
                let fill: Vec<usize> = (0..n).filter(|&r| li[r].is_none()).collect();
                if !fill.is_empty() {
                    let from: Vec<usize> =
                        fill.iter().map(|&r| ri[r].expect("right row")).collect();
                    let piece = self.tbl_take_rows(right.column(rk[p]), &from, out)?;
                    col =
                        self.tbl_write(col, vec![row_expr(&fill), Expr::All], Some(piece), out)?;
                }
            }
            parts.push_variable(lnames[k].clone(), col);
        }
        for (k, &v) in rvars.iter().enumerate() {
            let col = self.join_take(&right, v, &ri, who, out)?;
            parts.push_variable(rnames[k].clone(), col);
        }
        let mut values = vec![tvalue(TableArray::new(parts)?)];
        if who == "join" {
            if count > 1 {
                values.push(index_column(&ri)?);
            }
        } else {
            if count > 1 {
                values.push(index_column(&li)?);
            }
            if count > 2 {
                values.push(index_column(&ri)?);
            }
        }
        Ok(values)
    }

    /// Rows `rows` of variable `v`; None rows get the type's missing value.
    fn join_take(
        &mut self,
        t: &TableArray,
        v: usize,
        rows: &[Option<usize>],
        who: &str,
        out: &mut Execution,
    ) -> Result<Array> {
        let column = t.column(v);
        let gaps: Vec<usize> = (0..rows.len()).filter(|&r| rows[r].is_none()).collect();
        if !gaps.is_empty() && t.height() == 0 {
            return Err(format!("{who}: an empty table cannot be outer-joined yet"));
        }
        let picked: Vec<usize> = rows.iter().map(|r| r.unwrap_or(0)).collect();
        let col = self.tbl_take_rows(column, &picked, out)?;
        if gaps.is_empty() {
            return Ok(col);
        }
        let piece = match self.missing_piece(&col, out)? {
            Some(p) => p,
            None if matches!(col, Array::Logical(_)) => {
                Array::Logical(LogicalMatrix::scalar(false))
            }
            None => Array::Numeric(Matrix::scalar(0.)),
        };
        self.tbl_write(col, vec![row_expr(&gaps), Expr::All], Some(piece), out)
    }
}
