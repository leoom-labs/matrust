//! Pure-Rust data model, validation, samples and `createpde` script generator
//! for the PDE Modeler app. Independent of egui and desktop UI.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// Maximum number of shapes in one model (matches `decsg`).
pub const MAX_SHAPES: usize = 512;
/// Maximum set formula length in characters (matches `decsg`).
pub const MAX_FORMULA_LEN: usize = 4096;
/// Default `Hmax` for the plate-with-hole samples (keeps quadratic DOFs well
/// under the 4096 time-dependent solver limit).
pub const PLATE_DEFAULT_HMAX: &str = "0.25";

const KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "classdef",
    "continue",
    "else",
    "elseif",
    "end",
    "for",
    "function",
    "global",
    "if",
    "otherwise",
    "parfor",
    "persistent",
    "return",
    "spmd",
    "switch",
    "try",
    "while",
];

/// A 2-D CSG primitive shape drawn in PDE Modeler.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rectangle {
        name: String,
        x0: f64,
        y0: f64,
        w: f64,
        h: f64,
    },
    Circle {
        name: String,
        cx: f64,
        cy: f64,
        r: f64,
    },
    Ellipse {
        name: String,
        cx: f64,
        cy: f64,
        a: f64,
        b: f64,
        angle: f64,
    },
    Polygon {
        name: String,
        pts: Vec<[f64; 2]>,
    },
}

impl Shape {
    pub fn name(&self) -> &str {
        match self {
            Self::Rectangle { name, .. }
            | Self::Circle { name, .. }
            | Self::Ellipse { name, .. }
            | Self::Polygon { name, .. } => name,
        }
    }

    pub fn name_mut(&mut self) -> &mut String {
        match self {
            Self::Rectangle { name, .. }
            | Self::Circle { name, .. }
            | Self::Ellipse { name, .. }
            | Self::Polygon { name, .. } => name,
        }
    }

    pub fn kind_prefix(&self) -> &'static str {
        match self {
            Self::Rectangle { .. } => "R",
            Self::Circle { .. } => "C",
            Self::Ellipse { .. } => "E",
            Self::Polygon { .. } => "P",
        }
    }

    /// One-line human-readable summary for the read-only shape list.
    pub fn summary(&self) -> String {
        match self {
            Self::Rectangle { name, x0, y0, w, h } => {
                format!(
                    "{name}: Rectangle ({}, {}) {} x {}",
                    fmt_num(*x0),
                    fmt_num(*y0),
                    fmt_num(*w),
                    fmt_num(*h)
                )
            }
            Self::Circle { name, cx, cy, r } => {
                format!(
                    "{name}: Circle ({}, {}) r = {}",
                    fmt_num(*cx),
                    fmt_num(*cy),
                    fmt_num(*r)
                )
            }
            Self::Ellipse {
                name,
                cx,
                cy,
                a,
                b,
                angle,
            } => {
                format!(
                    "{name}: Ellipse ({}, {}) a = {}, b = {}, angle = {}",
                    fmt_num(*cx),
                    fmt_num(*cy),
                    fmt_num(*a),
                    fmt_num(*b),
                    fmt_num(*angle)
                )
            }
            Self::Polygon { name, pts } => {
                format!("{name}: Polygon ({} vertices)", pts.len())
            }
        }
    }

    /// Column entries for the `decsg` geometry description matrix `gd`.
    fn gd_column(&self) -> Result<Vec<f64>, String> {
        match self {
            Self::Rectangle { name, x0, y0, w, h } => {
                if !(*x0).is_finite()
                    || !(*y0).is_finite()
                    || !(*w).is_finite()
                    || !(*h).is_finite()
                    || *w <= 0.0
                    || *h <= 0.0
                {
                    return Err(format!(
                        "Shape '{name}': rectangle width and height must be positive finite numbers."
                    ));
                }
                let x1 = x0 + w;
                let y1 = y0 + h;
                Ok(vec![3.0, 4.0, *x0, x1, x1, *x0, *y0, *y0, y1, y1])
            }
            Self::Circle { name, cx, cy, r } => {
                if !(*cx).is_finite() || !(*cy).is_finite() || !(*r).is_finite() || *r <= 0.0 {
                    return Err(format!(
                        "Shape '{name}': circle radius must be a positive finite number."
                    ));
                }
                Ok(vec![1.0, *cx, *cy, *r])
            }
            Self::Ellipse {
                name,
                cx,
                cy,
                a,
                b,
                angle,
            } => {
                if !(*cx).is_finite()
                    || !(*cy).is_finite()
                    || !(*a).is_finite()
                    || !(*b).is_finite()
                    || !(*angle).is_finite()
                    || *a <= 0.0
                    || *b <= 0.0
                {
                    return Err(format!(
                        "Shape '{name}': ellipse semi-axes must be positive finite numbers."
                    ));
                }
                Ok(vec![4.0, *cx, *cy, *a, *b, *angle])
            }
            Self::Polygon { name, pts } => {
                if pts.len() < 3 {
                    return Err(format!(
                        "Shape '{name}': polygon must have at least 3 vertices."
                    ));
                }
                if pts.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
                    return Err(format!(
                        "Shape '{name}': polygon vertices must be finite numbers."
                    ));
                }
                let mut col = Vec::with_capacity(2 + 2 * pts.len());
                col.push(2.0);
                col.push(pts.len() as f64);
                for p in pts {
                    col.push(p[0]);
                }
                for p in pts {
                    col.push(p[1]);
                }
                Ok(col)
            }
        }
    }
}

/// PDE equation class.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PdeType {
    #[default]
    Elliptic,
    Parabolic,
    Hyperbolic,
    Eigen,
}

impl PdeType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Elliptic => "Elliptic",
            Self::Parabolic => "Parabolic",
            Self::Hyperbolic => "Hyperbolic",
            Self::Eigen => "Eigenvalue",
        }
    }
}

/// Finite-element geometric order passed to `generateMesh`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GeometricOrder {
    Linear,
    #[default]
    Quadratic,
}

impl GeometricOrder {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Quadratic => "quadratic",
        }
    }
}

/// PDE coefficients stored as text (numbers or MATLAB expressions).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Coefficients {
    pub m: String,
    pub d: String,
    pub c: String,
    pub a: String,
    pub f: String,
}

impl Default for Coefficients {
    fn default() -> Self {
        Self {
            m: "0".into(),
            d: "0".into(),
            c: "1".into(),
            a: "0".into(),
            f: "1".into(),
        }
    }
}

/// Time grid for parabolic and hyperbolic solves (`linspace(t0, t1, steps)`).
#[derive(Clone, Debug, PartialEq)]
pub struct TimeRange {
    pub t0: f64,
    pub t1: f64,
    pub steps: usize,
}

impl Default for TimeRange {
    fn default() -> Self {
        Self {
            t0: 0.0,
            t1: 1.0,
            steps: 21,
        }
    }
}

/// Mesh generation settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MeshSettings {
    /// Empty string means auto (let `generateMesh` pick `Hmax`).
    pub hmax: String,
    pub order: GeometricOrder,
}

/// Boundary condition on a geometry edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundaryCondition {
    Dirichlet { u: String },
    Neumann { g: String, q: String },
}

impl BoundaryCondition {
    pub fn summary(&self) -> String {
        match self {
            Self::Dirichlet { u } => format!("Dirichlet (u = {u})"),
            Self::Neumann { g, q } => format!("Neumann (g = {g}, q = {q})"),
        }
    }

    /// Each value must be one MATLAB expression (see `check_expression`).
    pub fn validate_expressions(&self, edge: usize) -> Result<(), String> {
        match self {
            Self::Dirichlet { u } => check_expression(&format!("u on edge E{edge}"), u),
            Self::Neumann { g, q } => {
                check_expression(&format!("g on edge E{edge}"), g)?;
                check_expression(&format!("q on edge E{edge}"), q)
            }
        }
    }
}

/// Complete state of a 2-D PDE Modeler session.
#[derive(Clone, Debug, PartialEq)]
pub struct PdeAppModel {
    pub shapes: Vec<Shape>,
    pub set_formula: String,
    pub pde_type: PdeType,
    pub coeffs: Coefficients,
    pub time_range: TimeRange,
    pub eigen_range: String,
    pub mesh: MeshSettings,
    /// Known edge IDs of the decomposed geometry (e.g. `1..=8`).
    pub edge_ids: Vec<usize>,
    pub boundary_conditions: BTreeMap<usize, BoundaryCondition>,
    pub u0: String,
    pub ut0: String,
}

impl Default for PdeAppModel {
    fn default() -> Self {
        Self {
            shapes: Vec::new(),
            set_formula: String::new(),
            pde_type: PdeType::Elliptic,
            coeffs: Coefficients::default(),
            time_range: TimeRange::default(),
            eigen_range: "[-Inf 100]".into(),
            mesh: MeshSettings::default(),
            edge_ids: Vec::new(),
            boundary_conditions: BTreeMap::new(),
            u0: "0".into(),
            ut0: "0".into(),
        }
    }
}

impl PdeAppModel {
    /// Returns true if `name` is a valid MATLAB identifier (not a keyword).
    pub fn is_valid_identifier(name: &str) -> bool {
        let mut chars = name.chars();
        let Some(first) = chars.next() else {
            return false;
        };
        if !first.is_ascii_alphabetic() || name.len() > 63 {
            return false;
        }
        chars.all(|c| c.is_ascii_alphanumeric() || c == '_') && !KEYWORDS.contains(&name)
    }

    /// Next free default name with the given prefix (`R1`, `C1`, `E1`, `P1`, ...).
    pub fn next_default_name(&self, prefix: &str) -> String {
        let mut n = 1usize;
        loop {
            let candidate = format!("{prefix}{n}");
            if !self.shapes.iter().any(|s| s.name() == candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    /// Default set formula: union of all shape names (`R1+C1+...`).
    pub fn default_set_formula(&self) -> String {
        self.shapes
            .iter()
            .map(Shape::name)
            .collect::<Vec<_>>()
            .join("+")
    }

    fn push_shape(&mut self, shape: Shape) -> String {
        let prev_default = self.default_set_formula();
        let name = shape.name().to_owned();
        self.shapes.push(shape);
        if self.set_formula.trim().is_empty() || self.set_formula.trim() == prev_default {
            self.set_formula = self.default_set_formula();
        }
        name
    }

    pub fn add_rectangle(&mut self, x0: f64, y0: f64, w: f64, h: f64) -> String {
        let name = self.next_default_name("R");
        self.push_shape(Shape::Rectangle { name, x0, y0, w, h })
    }

    pub fn add_circle(&mut self, cx: f64, cy: f64, r: f64) -> String {
        let name = self.next_default_name("C");
        self.push_shape(Shape::Circle { name, cx, cy, r })
    }

    pub fn add_ellipse(&mut self, cx: f64, cy: f64, a: f64, b: f64, angle: f64) -> String {
        let name = self.next_default_name("E");
        self.push_shape(Shape::Ellipse {
            name,
            cx,
            cy,
            a,
            b,
            angle,
        })
    }

    pub fn add_polygon(&mut self, pts: Vec<[f64; 2]>) -> String {
        let name = self.next_default_name("P");
        self.push_shape(Shape::Polygon { name, pts })
    }

    /// Rename shape at `index`, enforcing valid identifier syntax and uniqueness.
    pub fn rename_shape(&mut self, index: usize, new_name: &str) -> Result<(), String> {
        let trimmed = new_name.trim();
        if index >= self.shapes.len() {
            return Err(format!("Shape index {} is out of range.", index + 1));
        }
        if !Self::is_valid_identifier(trimmed) {
            return Err(format!(
                "Invalid shape name '{trimmed}': must be a valid identifier."
            ));
        }
        if self
            .shapes
            .iter()
            .enumerate()
            .any(|(i, s)| i != index && s.name() == trimmed)
        {
            return Err(format!(
                "Duplicate shape name '{trimmed}': shape names must be unique."
            ));
        }
        *self.shapes[index].name_mut() = trimmed.to_owned();
        Ok(())
    }

    /// Validate all shape names (valid identifiers, unique, non-empty).
    pub fn validate_shape_names(&self) -> Result<(), String> {
        let mut seen = BTreeSet::new();
        for shape in &self.shapes {
            let name = shape.name();
            if !Self::is_valid_identifier(name) {
                return Err(format!(
                    "Invalid shape name '{name}': must be a valid identifier."
                ));
            }
            if !seen.insert(name) {
                return Err(format!(
                    "Duplicate shape name '{name}': shape names must be unique."
                ));
            }
        }
        Ok(())
    }

    /// Validate the set formula against the current shape names.
    /// Returns one clear error line naming the bad token on failure.
    pub fn validate_set_formula(&self) -> Result<(), String> {
        let formula = self.set_formula.trim();
        if formula.is_empty() {
            return Err("Invalid set formula: formula is empty.".into());
        }
        if formula.chars().count() > MAX_FORMULA_LEN {
            return Err(format!(
                "Invalid set formula: exceeds {MAX_FORMULA_LEN} characters."
            ));
        }
        let names: Vec<&str> = self.shapes.iter().map(Shape::name).collect();
        let mut parser = FormulaValidator {
            chars: formula.chars().collect(),
            pos: 0,
            depth: 0,
            names: &names,
        };
        parser.sum()?;
        parser.skip();
        if parser.pos < parser.chars.len() {
            let tok = parser.read_bad_token();
            return Err(format!("Invalid set formula: unexpected token '{tok}'."));
        }
        Ok(())
    }

    /// Validate the model and return the first user-facing problem.
    /// Bad coefficient expressions are left to the engine error.
    pub fn validate(&self) -> Result<(), String> {
        if self.shapes.is_empty() {
            return Err("No shape drawn. Add a shape or load a sample.".into());
        }
        if self.shapes.len() > MAX_SHAPES {
            return Err(format!("Too many shapes (maximum {MAX_SHAPES})."));
        }
        self.validate_shape_names()?;
        for shape in &self.shapes {
            shape.gd_column()?;
        }
        self.validate_set_formula()?;
        let hmax = self.mesh.hmax.trim();
        if !hmax.is_empty() {
            let value: f64 = hmax
                .parse()
                .map_err(|_| format!("Invalid Hmax '{hmax}': must be a positive number."))?;
            if !value.is_finite() || value <= 0.0 {
                return Err(format!("Invalid Hmax '{hmax}': must be a positive number."));
            }
        }
        if matches!(self.pde_type, PdeType::Parabolic | PdeType::Hyperbolic)
            && (!self.time_range.t0.is_finite()
                || !self.time_range.t1.is_finite()
                || self.time_range.t1 <= self.time_range.t0
                || self.time_range.steps < 2)
        {
            return Err(
                "Invalid time range: end time must exceed start time with at least 2 steps.".into(),
            );
        }
        self.validate_expressions()?;
        if !self.edge_ids.is_empty() {
            let missing: Vec<String> = self
                .edge_ids
                .iter()
                .filter(|id| !self.boundary_conditions.contains_key(id))
                .map(usize::to_string)
                .collect();
            if !missing.is_empty() {
                let plural = if missing.len() == 1 { "" } else { "s" };
                return Err(format!(
                    "Missing boundary condition on edge{plural}: {}.",
                    missing.join(", ")
                ));
            }
        } else if self.boundary_conditions.is_empty() {
            return Err(
                "Missing boundary condition: assign a condition to every geometry edge.".into(),
            );
        }
        Ok(())
    }

    /// Check every coefficient, boundary-condition, initial-condition and
    /// eigen-range field the script will use: each must be one MATLAB
    /// expression, so no field text can add statements to the script.
    pub fn validate_expressions(&self) -> Result<(), String> {
        self.validate_coefficient_expressions()?;
        for (edge, bc) in &self.boundary_conditions {
            bc.validate_expressions(*edge)?;
        }
        Ok(())
    }

    /// The PDE-panel part of `validate_expressions` (coefficients, u0, ut0,
    /// eigen range).
    pub fn validate_coefficient_expressions(&self) -> Result<(), String> {
        let c = &self.coeffs;
        let mut fields: Vec<(&str, &str)> = match self.pde_type {
            PdeType::Elliptic => vec![("c", &c.c), ("a", &c.a), ("f", &c.f)],
            PdeType::Parabolic => {
                vec![
                    ("d", &c.d),
                    ("c", &c.c),
                    ("a", &c.a),
                    ("f", &c.f),
                    ("u0", &self.u0),
                ]
            }
            PdeType::Hyperbolic => vec![
                ("m", &c.m),
                ("c", &c.c),
                ("a", &c.a),
                ("f", &c.f),
                ("u0", &self.u0),
                ("ut0", &self.ut0),
            ],
            PdeType::Eigen => vec![
                ("d", &c.d),
                ("c", &c.c),
                ("a", &c.a),
                ("Range", &self.eigen_range),
            ],
        };
        // Hidden coefficients still reach `specifyCoefficients`.
        for (name, text) in [
            ("m", &c.m),
            ("d", &c.d),
            ("c", &c.c),
            ("a", &c.a),
            ("f", &c.f),
        ] {
            if !fields.iter().any(|(n, _)| *n == name) {
                fields.push((name, text));
            }
        }
        for (name, text) in fields {
            check_expression(name, text)?;
        }
        Ok(())
    }

    /// Generate the geometry-only script (`model = createpde()`, `pdem_gd`,
    /// `pdem_sf`, `pdem_ns`, `geometryFromEdges(model, decsg(...))`). The
    /// `pdem_` prefix keeps Export from overwriting a user's `gd`/`sf`/`ns`.
    pub fn geometry_script(&self) -> Result<String, String> {
        if self.shapes.is_empty() {
            return Err("No shape drawn. Add a shape or load a sample.".into());
        }
        if self.shapes.len() > MAX_SHAPES {
            return Err(format!("Too many shapes (maximum {MAX_SHAPES})."));
        }
        self.validate_shape_names()?;
        self.validate_set_formula()?;

        let mut cols = Vec::with_capacity(self.shapes.len());
        let mut max_rows = 0usize;
        for shape in &self.shapes {
            let col = shape.gd_column()?;
            max_rows = max_rows.max(col.len());
            cols.push(col);
        }
        for col in &mut cols {
            col.resize(max_rows, 0.0);
        }

        let mut out = String::new();
        out.push_str("% Create 2-D PDE model and CSG geometry\n");
        out.push_str("model = createpde();\n");
        out.push_str("pdem_gd = [\n");
        for row in 0..max_rows {
            out.push_str("    ");
            for (c, col) in cols.iter().enumerate() {
                if c > 0 {
                    out.push_str(", ");
                }
                out.push_str(&fmt_num(col[row]));
            }
            if row + 1 < max_rows {
                out.push(';');
            }
            out.push('\n');
        }
        out.push_str("];\n");

        let sf = self.set_formula.trim().replace('\'', "''");
        let _ = writeln!(out, "pdem_sf = '{sf}';");

        let max_name_len = self
            .shapes
            .iter()
            .map(|s| s.name().len())
            .max()
            .unwrap_or(1);
        out.push_str("pdem_ns = [");
        for (i, shape) in self.shapes.iter().enumerate() {
            if i > 0 {
                out.push_str("; ");
            }
            let padded = format!("{:<width$}", shape.name(), width = max_name_len);
            let _ = write!(out, "'{padded}'");
        }
        out.push_str("]';\n");
        out.push_str("geometryFromEdges(model, decsg(pdem_gd, pdem_sf, pdem_ns));\n");
        Ok(out)
    }

    /// Generate the complete, commented, runnable `createpde` workflow script.
    pub fn script(&self) -> Result<String, String> {
        self.validate()?;
        let mut out = self.geometry_script()?;

        // Mesh
        out.push_str("\n% Generate finite-element mesh\n");
        let hmax = self.mesh.hmax.trim();
        let order = self.mesh.order.as_str();
        if hmax.is_empty() {
            let _ = writeln!(out, "generateMesh(model, 'GeometricOrder', '{order}');");
        } else {
            let _ = writeln!(
                out,
                "generateMesh(model, 'Hmax', {hmax}, 'GeometricOrder', '{order}');"
            );
        }

        // Coefficients
        out.push_str("\n% Specify PDE coefficients\n");
        let m = coeff_or_zero(&self.coeffs.m);
        let d = coeff_or_zero(&self.coeffs.d);
        let c = coeff_or_zero(&self.coeffs.c);
        let a = coeff_or_zero(&self.coeffs.a);
        let f = coeff_or_zero(&self.coeffs.f);
        let _ = writeln!(
            out,
            "specifyCoefficients(model, 'm', {m}, 'd', {d}, 'c', {c}, 'a', {a}, 'f', {f});"
        );

        // Boundary conditions
        out.push_str("\n% Apply boundary conditions\n");
        for (&edge_id, bc) in &self.boundary_conditions {
            match bc {
                BoundaryCondition::Dirichlet { u } => {
                    let u_expr = coeff_or_zero(u);
                    let _ = writeln!(
                        out,
                        "applyBoundaryCondition(model, 'dirichlet', 'Edge', {edge_id}, 'u', {u_expr});"
                    );
                }
                BoundaryCondition::Neumann { g, q } => {
                    let g_expr = coeff_or_zero(g);
                    let q_expr = coeff_or_zero(q);
                    let _ = writeln!(
                        out,
                        "applyBoundaryCondition(model, 'neumann', 'Edge', {edge_id}, 'g', {g_expr}, 'q', {q_expr});"
                    );
                }
            }
        }

        // Initial conditions (parabolic / hyperbolic)
        match self.pde_type {
            PdeType::Parabolic => {
                out.push_str("\n% Set initial conditions\n");
                let u0 = coeff_or_zero(&self.u0);
                let _ = writeln!(out, "setInitialConditions(model, {u0});");
            }
            PdeType::Hyperbolic => {
                out.push_str("\n% Set initial conditions\n");
                let u0 = coeff_or_zero(&self.u0);
                let ut0 = coeff_or_zero(&self.ut0);
                let _ = writeln!(out, "setInitialConditions(model, {u0}, {ut0});");
            }
            PdeType::Elliptic | PdeType::Eigen => {}
        }

        // Solve
        out.push_str("\n% Solve the PDE\n");
        match self.pde_type {
            PdeType::Elliptic => {
                out.push_str("results = solvepde(model);\n");
            }
            PdeType::Parabolic | PdeType::Hyperbolic => {
                let _ = writeln!(
                    out,
                    "pdem_tlist = linspace({}, {}, {});",
                    fmt_num(self.time_range.t0),
                    fmt_num(self.time_range.t1),
                    self.time_range.steps
                );
                out.push_str("results = solvepde(model, pdem_tlist);\n");
            }
            PdeType::Eigen => {
                let range = if self.eigen_range.trim().is_empty() {
                    "[-Inf 100]"
                } else {
                    self.eigen_range.trim()
                };
                let _ = writeln!(out, "results = solvepdeeig(model, {range});");
            }
        }

        Ok(out)
    }

    /// Sample 1: Square `[-1, 1]^2` minus circle radius 0.4 at origin, elliptic `-Δu = 1`,
    /// `u = 0` on outer edges and zero flux (`g = 0, q = 0`) on the hole edges.
    pub fn plate_with_hole() -> Self {
        let mut model = Self {
            shapes: vec![
                Shape::Rectangle {
                    name: "R1".into(),
                    x0: -1.0,
                    y0: -1.0,
                    w: 2.0,
                    h: 2.0,
                },
                Shape::Circle {
                    name: "C1".into(),
                    cx: 0.0,
                    cy: 0.0,
                    r: 0.4,
                },
            ],
            set_formula: "R1-C1".into(),
            pde_type: PdeType::Elliptic,
            coeffs: Coefficients {
                m: "0".into(),
                d: "0".into(),
                c: "1".into(),
                a: "0".into(),
                f: "1".into(),
            },
            time_range: TimeRange::default(),
            eigen_range: "[-Inf 100]".into(),
            mesh: MeshSettings {
                hmax: PLATE_DEFAULT_HMAX.into(),
                order: GeometricOrder::Quadratic,
            },
            edge_ids: (1..=8).collect(),
            boundary_conditions: BTreeMap::new(),
            u0: "0".into(),
            ut0: "0".into(),
        };
        // decsg edge IDs for R1-C1 (verified against decsg output):
        // Edges 1..=4 are the 4 outer rectangle edges; edges 5..=8 are the 4 circular hole arcs.
        for edge in 1..=4 {
            model
                .boundary_conditions
                .insert(edge, BoundaryCondition::Dirichlet { u: "0".into() });
        }
        for edge in 5..=8 {
            model.boundary_conditions.insert(
                edge,
                BoundaryCondition::Neumann {
                    g: "0".into(),
                    q: "0".into(),
                },
            );
        }
        model
    }

    /// Sample 2: Same plate with a hole, parabolic heat equation `d=1, c=1, f=1`,
    /// `u0 = 0`, `tlist = linspace(0, 1, 21)`.
    pub fn plate_heat() -> Self {
        let mut model = Self::plate_with_hole();
        model.pde_type = PdeType::Parabolic;
        model.coeffs = Coefficients {
            m: "0".into(),
            d: "1".into(),
            c: "1".into(),
            a: "0".into(),
            f: "1".into(),
        };
        model.u0 = "0".into();
        model.time_range = TimeRange {
            t0: 0.0,
            t1: 1.0,
            steps: 21,
        };
        model
    }
}

/// Refuse field text that is not exactly one MATLAB expression (empty means 0).
/// The text is parsed with the engine parser both bare and in parentheses, so
/// statement separators, top-level commas, comments, newlines and unbalanced
/// brackets cannot change the script around the field.
pub fn check_expression(field: &str, text: &str) -> Result<(), String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(());
    }
    let refuse = || {
        Err(format!(
            "Invalid {field}: enter one MATLAB expression (no ';', ',' or new lines)."
        ))
    };
    if t.contains(['\n', '\r']) {
        return refuse();
    }
    for wrapped in [format!("pdem_expr = {t};"), format!("pdem_expr = ({t});")] {
        let Ok(statements) = crate::syntax::parse(&wrapped) else {
            return refuse();
        };
        let ok = matches!(
            statements.as_slice(),
            [statement] if matches!(
                &statement.kind,
                crate::syntax::StatementKind::Evaluate(ev)
                    if ev.targets == [Some("pdem_expr".to_owned())]
                        && ev.indexed_target.is_none()
                        && ev.quiet
            )
        );
        if !ok {
            return refuse();
        }
    }
    Ok(())
}

fn coeff_or_zero(text: &str) -> &str {
    let t = text.trim();
    if t.is_empty() { "0" } else { t }
}

fn fmt_num(v: f64) -> String {
    if v == 0.0 { "0".into() } else { format!("{v}") }
}

struct FormulaValidator<'a> {
    chars: Vec<char>,
    pos: usize,
    depth: usize,
    names: &'a [&'a str],
}

impl FormulaValidator<'_> {
    fn skip(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.skip();
        self.chars.get(self.pos).copied()
    }

    fn read_bad_token(&mut self) -> String {
        self.skip();
        if self.pos >= self.chars.len() {
            return String::new();
        }
        let start = self.pos;
        if self.chars[self.pos].is_alphanumeric() || self.chars[self.pos] == '_' {
            while self.pos < self.chars.len()
                && (self.chars[self.pos].is_alphanumeric() || self.chars[self.pos] == '_')
            {
                self.pos += 1;
            }
            self.chars[start..self.pos].iter().collect()
        } else {
            let c = self.chars[self.pos];
            self.pos += 1;
            c.to_string()
        }
    }

    fn sum(&mut self) -> Result<(), String> {
        self.product()?;
        while self.peek() == Some('+') {
            self.pos += 1;
            self.product()?;
        }
        Ok(())
    }

    fn product(&mut self) -> Result<(), String> {
        self.difference()?;
        while self.peek() == Some('*') {
            self.pos += 1;
            self.difference()?;
        }
        Ok(())
    }

    fn difference(&mut self) -> Result<(), String> {
        self.atom()?;
        while self.peek() == Some('-') {
            self.pos += 1;
            self.atom()?;
        }
        Ok(())
    }

    fn atom(&mut self) -> Result<(), String> {
        match self.peek() {
            Some('(') => {
                self.depth += 1;
                if self.depth > 200 {
                    return Err("Invalid set formula: parentheses nested too deeply.".into());
                }
                self.pos += 1;
                self.sum()?;
                if self.peek() != Some(')') {
                    return Err("Invalid set formula: missing ')'.".into());
                }
                self.pos += 1;
                self.depth -= 1;
                Ok(())
            }
            Some(c) if c.is_alphanumeric() || c == '_' => {
                let start = self.pos;
                while self.pos < self.chars.len()
                    && (self.chars[self.pos].is_alphanumeric() || self.chars[self.pos] == '_')
                {
                    self.pos += 1;
                }
                let name: String = self.chars[start..self.pos].iter().collect();
                if self.names.contains(&name.as_str()) {
                    Ok(())
                } else {
                    Err(format!("Invalid set formula: unknown shape '{name}'."))
                }
            }
            Some(_) => {
                let tok = self.read_bad_token();
                Err(format!("Invalid set formula: unexpected token '{tok}'."))
            }
            None => Err("Invalid set formula: unexpected end of formula.".into()),
        }
    }
}
