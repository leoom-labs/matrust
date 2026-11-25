//! Regression and clustering, independent of the desktop UI.
//!
//! Random starts consume the session rand stream in replicate order, then
//! center order. Sample uses a partial Fisher-Yates shuffle (one uniform per
//! center); plus uses one uniform for the first row and one per subsequent
//! weighted draw. Uniform centers consume coordinate-major, center-minor draws.
//! Explicit starts and PAM BUILD consume no random numbers.
//! Kmedoids cluster starts partially shuffle ceil(n/10).max(k) rows,
//! consuming one uniform per sampled row before a deterministic preliminary PAM.
use super::{Engine, Execution, Value};
use crate::{Array, Matrix, Result};
use nalgebra::DMatrix;

pub(super) mod classifiers;
mod cluster;
mod discriminant;
mod distance;
mod ecoc;
pub(in crate::engine) mod ensemble;
mod linear_classifier;
pub(in crate::engine) mod metrics;
pub(in crate::engine) mod ml;
pub(super) mod ml_regression;
pub(in crate::engine) mod partition;
mod pca;
pub(super) mod regression;

pub(super) const NAMES: &[&str] = &[
    "lassoPlot",
    "regress",
    "ridge",
    "lasso",
    "pca",
    "pcacov",
    "kmeans",
    "kmedoids",
    "pdist",
    "pdist2",
    "squareform",
    "linkage",
    "dendrogram",
    "cluster",
    "inconsistent",
    "knnsearch",
    "confusionmat",
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
];

fn text(v: &Value) -> Option<&str> {
    match v {
        Value::Text(s) | Value::QuotedString(s) => Some(s),
        _ => None,
    }
}

fn real(e: &Engine, name: &str, v: &Value) -> Result<Matrix> {
    if text(v).is_some() || v.is_character() || v.is_nd() {
        return Err(format!(
            "{name}: expected a real two-dimensional numeric array"
        ));
    }
    v.clone().matrix(&e.cancel)
}

fn scalar(e: &Engine, name: &str, v: &Value) -> Result<f64> {
    let m = real(e, name, v)?;
    if m.len() != 1 || !m.data()[0].is_finite() {
        return Err(format!("{name}: expected a finite scalar"));
    }
    Ok(m.data()[0])
}

fn integer(e: &Engine, name: &str, v: &Value) -> Result<usize> {
    let x = scalar(e, name, v)?;
    if x < 1. || x.fract() != 0. || x > crate::matrix::MAX_ELEMENTS as f64 {
        return Err(format!("{name}: expected a bounded positive integer"));
    }
    Ok(x as usize)
}

fn boolean(e: &Engine, name: &str, v: &Value) -> Result<bool> {
    if let Some(s) = text(v) {
        return match s.to_lowercase().as_str() {
            "on" | "true" => Ok(true),
            "off" | "false" => Ok(false),
            _ => Err(format!("{name}: expected on/off or a logical scalar")),
        };
    }
    match scalar(e, name, v)? {
        0. => Ok(false),
        1. => Ok(true),
        _ => Err(format!("{name}: expected a logical scalar")),
    }
}

fn options<'a>(
    name: &str,
    args: &'a [Value],
    allowed: &[&str],
) -> Result<Vec<(String, &'a Value)>> {
    if !args.len().is_multiple_of(2) {
        return Err(format!("{name}: options must be name/value pairs"));
    }
    args.as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let key = text(&pair[0])
                .ok_or_else(|| format!("{name}: expected an option name"))?
                .to_lowercase();
            if !allowed.contains(&key.as_str()) {
                return Err(format!("{name}: option '{key}' is not supported yet"));
            }
            Ok((key, &pair[1]))
        })
        .collect()
}

fn size(e: &Engine, name: &str, rows: usize, cols: usize) -> Result<usize> {
    let n = Matrix::checked_len(rows, cols)
        .map_err(|_| format!("{name}: output or workspace too large"))?;
    e.preflight_elements(n)
        .map_err(|_| format!("{name}: output or workspace too large"))?;
    Ok(n)
}

fn value(rows: usize, cols: usize, data: Vec<f64>) -> Result<Value> {
    Ok(Value::Matrix(Matrix::from_column_vec(rows, cols, data)?))
}

fn dense(m: &Matrix) -> DMatrix<f64> {
    DMatrix::from_column_slice(m.rows(), m.cols(), m.data())
}

fn matrix_value(m: DMatrix<f64>) -> Result<Value> {
    value(m.nrows(), m.ncols(), m.as_slice().to_vec())
}

fn row(m: &Matrix, r: usize) -> Vec<f64> {
    (0..m.cols()).map(|c| m.data()[r + c * m.rows()]).collect()
}

fn clean_rows(e: &Engine, name: &str, x: &Matrix, y: Option<&Matrix>) -> Result<Vec<usize>> {
    if x.data()
        .iter()
        .chain(y.into_iter().flat_map(|y| y.data()))
        .any(|v| v.is_infinite())
    {
        return Err(format!("{name}: infinite data are not supported"));
    }
    let mut keep = Vec::new();
    for r in 0..x.rows() {
        if r.is_multiple_of(256) {
            e.check_cancel()?;
        }
        if (0..x.cols()).all(|c| !x.data()[r + c * x.rows()].is_nan())
            && y.is_none_or(|y| !y.data()[r].is_nan())
        {
            keep.push(r);
        }
    }
    Ok(keep)
}

fn warning(e: &mut Engine, out: &mut Execution, message: &str) -> Result<()> {
    e.lang_utils_call("warning", &[Value::Text(message.into())], out)?;
    Ok(())
}

impl Engine {
    pub(super) fn stm_outputs(
        &mut self,
        name: &str,
        args: &[Value],
        count: usize,
        out: &mut Execution,
    ) -> Result<Vec<Value>> {
        self.check_cancel()?;
        if name == "dendrogram" && count > 3 {
            return Err("dendrogram returns at most three outputs".into());
        }
        if name == "lassoPlot" {
            return Err("lassoPlot: not supported yet".into());
        }
        // Preserve character and string group operands; only the option tail
        // is text-coerced. Positional metric strings are parsed by their module.
        let positional = match name {
            "regress" | "ridge" => args.len(),
            "lasso" | "kmeans" | "kmedoids" | "pdist2" | "knnsearch" | "confusionmat"
            | "fitcknn" | "fitctree" | "fitcsvm" | "fitcnb" | "fitcdiscr" | "fitclinear"
            | "fitcecoc" | "fitcensemble" | "fitrtree" | "fitrsvm" | "fitrgp" | "fitrensemble"
            | "fitrlinear" => 2,
            "templateSVM" | "templateTree" | "templateLinear" | "templateDiscriminant" => 0,
            _ => 1,
        }
        .min(args.len());
        let mut parsed = args[..positional].to_vec();
        if name == "confusionmat" {
            for (i, value) in args[positional..].iter().enumerate() {
                if i.is_multiple_of(2) {
                    parsed.push(Value::Text(self.text_value(value.clone())?));
                } else {
                    parsed.push(value.clone());
                }
            }
        } else {
            parsed.extend(self.text_arguments(args[positional..].to_vec())?);
        }
        let args = parsed;
        let mut result = match name {
            "regress" | "ridge" | "lasso" => regression::call(self, name, &args, out),
            "pca" | "pcacov" => pca::call(self, name, &args),
            "pdist" | "pdist2" | "squareform" | "knnsearch" => {
                distance::call(self, name, &args, out)
            }
            "fitcknn" | "fitctree" | "fitcsvm" | "fitcnb" => classifiers::fit(self, name, &args),
            "fitcdiscr" | "fitclinear" | "fitcecoc" | "fitcensemble" => ml::fit(self, name, &args),
            "fitrtree" | "fitrsvm" | "fitrgp" | "fitrensemble" | "fitrlinear" => {
                ml_regression::fit(self, name, &args)
            }
            "templateSVM" | "templateTree" | "templateLinear" | "templateDiscriminant" => {
                ml::template(self, name, &args).map(|v| vec![v])
            }
            "cvpartition" | "training" | "test" | "repartition" => {
                partition::call(self, name, &args).map(|v| vec![v])
            }
            "loss" | "margin" | "edge" | "oobLoss" | "predictorImportance" => {
                Err(format!("{name}: expected a model object"))
            }
            "crossval" | "predict" | "resubLoss" | "kfoldLoss" | "kfoldPredict" | "prune" => {
                Err(format!("{name}: expected a model object"))
            }
            _ => cluster::call(self, name, &args, out),
        }?;
        if count > result.len() {
            return Err(format!("{name}: too many outputs"));
        }
        result.truncate(count.max(1));
        Ok(result)
    }
}

// Dedicated caller for image segmentation's centre-displacement stop rule.
pub(super) fn image_segment_kmeans(
    e: &mut Engine,
    a: &[Value],
    threshold: f64,
    out: &mut Execution,
) -> Result<Vec<Value>> {
    cluster::image_segment_partition(e, "kmeans", a, Some(threshold), out)
}
