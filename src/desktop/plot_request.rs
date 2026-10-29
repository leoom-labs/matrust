//! Shared-array requests for native workspace plotting. No UI dependency.
use matrust::{
    Array, Engine, Result,
    plot::{
        AxisScale, Figure, MAX_POINTS, PlotData, PlotKind, series_from_matrices,
        series_from_plot_data,
    },
};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Request {
    pub x: Option<(String, Array)>,
    pub y: (String, Array),
    pub kind: PlotKind,
    pub log_x: bool,
    pub log_y: bool,
}

impl Request {
    pub fn validate(&self) -> Result<()> {
        if self.kind == PlotKind::Patch {
            return Err("Patch needs explicit face matrices".into());
        }
        for (name, data) in std::iter::once(&self.y).chain(self.x.iter()) {
            if !matrust::files::valid_name(name) {
                return Err("Select a valid workspace variable".into());
            }
            data.require_numeric_array()?;
            if data.is_complex() && self.kind != PlotKind::Line {
                return Err(format!(
                    "{name} is complex; select Line or plot real, imag or abs explicitly"
                ));
            }
            if data.is_empty() {
                return Err(format!(
                    "{name} is empty; select a variable containing data"
                ));
            }
            if data.len() > MAX_POINTS {
                return Err(format!("{name} exceeds the 1000000-point plot limit"));
            }
        }
        Ok(())
    }
    pub fn build(&self, cancel: &AtomicBool) -> Result<Figure> {
        self.validate()?;
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let y = (!self.y.1.is_complex())
            .then(|| self.y.1.to_numeric(cancel))
            .transpose()?;
        let x = self
            .x
            .as_ref()
            .filter(|(_, a)| !a.is_complex())
            .map(|(_, a)| a.to_numeric(cancel))
            .transpose()?;
        let series =
            if self.y.1.is_complex() || self.x.as_ref().is_some_and(|(_, a)| a.is_complex()) {
                let y = plot_data(&self.y.1, y.as_ref())?;
                let x = self
                    .x
                    .as_ref()
                    .map(|(_, a)| plot_data(a, x.as_ref()))
                    .transpose()?;
                series_from_plot_data(x, y, cancel)?
            } else {
                series_from_matrices(
                    x.as_ref(),
                    y.as_ref().ok_or("Missing plot coordinates")?,
                    self.kind,
                )?
            };
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let mut figure = Figure::default();
        figure.add(series)?;
        (figure.xlabel, figure.ylabel) = self.labels();
        figure.xscale = if self.log_x {
            AxisScale::Log
        } else {
            AxisScale::Linear
        };
        figure.yscale = if self.log_y {
            AxisScale::Log
        } else {
            AxisScale::Linear
        };
        figure.validate()?;
        Ok(figure)
    }
    pub fn create(&self, engine: &mut Engine) -> Result<u32> {
        let ids = engine.figure_ids();
        if ids.len() >= 16 {
            return Err("At most 16 figures can be open; close an unused figure first".into());
        }
        let figure = self.build(&engine.cancellation())?;
        if engine.cancellation().load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
        let id = (1..=17).find(|id| !ids.contains(id)).unwrap();
        // All fallible construction happens before selecting or publishing.
        engine.select_figure(id)?;
        engine.figure = Some(figure);
        Ok(id)
    }
    pub fn script(&self) -> Result<String> {
        self.validate()?;
        let function = match self.kind {
            PlotKind::Line => "plot",
            PlotKind::Scatter => "scatter",
            PlotKind::Bar => "bar",
            PlotKind::Stairs => "stairs",
            PlotKind::Stem => "stem",
            PlotKind::Patch => return Err("Patch needs explicit face matrices".into()),
        };
        let expression = |(name, array): &(String, Array)| {
            if array.is_logical() {
                format!("double({name})")
            } else {
                name.clone()
            }
        };
        let y = expression(&self.y);
        let args = self
            .x
            .as_ref()
            .map_or_else(|| y.clone(), |x| format!("{}, {y}", expression(x)));
        let (xlabel, ylabel) = self.labels();
        let mut script = format!(
            "% Plot workspace variables in a new figure.\nfigure;\n{function}({args});\nxlabel('{xlabel}');\nylabel('{}');\n",
            ylabel
        );
        if self.log_x {
            script.push_str("xscale('log');\n");
        }
        if self.log_y {
            script.push_str("yscale('log');\n");
        }
        Ok(script)
    }
    fn labels(&self) -> (String, String) {
        if self.x.is_none() && self.y.1.is_complex() {
            return (format!("real({})", self.y.0), format!("imag({})", self.y.0));
        }
        let label = |(name, array): &(String, Array)| {
            if array.is_complex() {
                format!("real({name})")
            } else {
                name.clone()
            }
        };
        (
            self.x.as_ref().map_or("Sample".into(), label),
            label(&self.y),
        )
    }
}

fn plot_data<'a>(array: &'a Array, real: Option<&'a matrust::Matrix>) -> Result<PlotData<'a>> {
    array
        .as_complex()
        .map(PlotData::Complex)
        .or_else(|| real.map(PlotData::Real))
        .ok_or_else(|| "Missing plot coordinates".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_plot_request_matches_scripts_and_shares_source_arrays() {
        for source in [
            "x=1:4;y=[1 2;3 4;5 6;7 8];",
            "x=[1;2;3;4];y=[4 3 2 1];",
            "x=1:4;y=logical([0 1 0 1]);",
        ] {
            for kind in [
                PlotKind::Line,
                PlotKind::Scatter,
                PlotKind::Bar,
                PlotKind::Stairs,
                PlotKind::Stem,
            ] {
                for implicit in [true, false] {
                    let mut engine = Engine::default();
                    engine.execute(source).unwrap();
                    let vars = engine.workspace();
                    let request = Request {
                        y: ("y".into(), vars["y"].clone()),
                        x: (!implicit).then(|| ("x".into(), vars["x"].clone())),
                        kind,
                        log_x: true,
                        log_y: true,
                    };
                    if let Some(y) = vars["y"].as_numeric() {
                        assert_eq!(
                            y.data().as_ptr(),
                            request.y.1.as_numeric().unwrap().data().as_ptr()
                        );
                    }
                    let expected = request.build(&AtomicBool::new(false)).unwrap();
                    engine.execute(&request.script().unwrap()).unwrap();
                    assert_eq!(
                        format!("{expected:?}"),
                        format!("{:?}", engine.figure.as_ref().unwrap())
                    );
                    assert_eq!(vars["y"].len(), request.y.1.len());
                }
            }
        }
    }
    #[test]
    fn workspace_plot_request_failures_leave_figures_and_selection_unchanged() {
        let mut engine = Engine::default();
        engine
            .execute("x=1:3;y=1:4;plot(y);title('Keep');")
            .unwrap();
        let vars = engine.workspace();
        let original = engine.figure.clone().unwrap();
        let mut request = Request {
            x: Some(("x".into(), vars["x"].clone())),
            y: ("y".into(), vars["y"].clone()),
            kind: PlotKind::Line,
            log_x: false,
            log_y: false,
        };
        assert!(request.create(&mut engine).is_err());
        assert_eq!(engine.figure_ids(), vec![1]);
        assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
        request.x = None;
        engine.cancellation().store(true, Ordering::Relaxed);
        assert!(
            request
                .create(&mut engine)
                .unwrap_err()
                .contains("Cancelled")
        );
        assert_eq!(engine.current_figure, 1);
        engine.cancellation().store(false, Ordering::Relaxed);
        assert_eq!(request.create(&mut engine).unwrap(), 2);
        assert!(original.same_snapshot(engine.figures().find(|(id, _)| *id == 1).unwrap().1));
        for id in 3..=16 {
            engine.select_figure(id).unwrap();
        }
        assert!(
            request
                .create(&mut engine)
                .unwrap_err()
                .contains("16 figures")
        );
        assert_eq!(engine.current_figure, 16);
        request.y.0 = "x);close('all')".into();
        assert!(request.script().is_err());
    }
    #[test]
    fn workspace_plot_budgets_and_invalid_bars_reject_without_publication() {
        let mut engine = Engine::default();
        engine.execute("plot([2 4]);").unwrap();
        let original = engine.figure.clone().unwrap();
        for (rows, cols, kind) in [
            (0, 1, PlotKind::Line),
            (1, MAX_POINTS + 1, PlotKind::Line),
            (2, 65, PlotKind::Line),
            (1, 10001, PlotKind::Bar),
        ] {
            let request = Request {
                x: None,
                y: (
                    "y".into(),
                    matrust::Matrix::from_column_vec(rows, cols, vec![1.; rows * cols])
                        .unwrap()
                        .into(),
                ),
                kind,
                log_x: false,
                log_y: false,
            };
            assert!(request.create(&mut engine).is_err());
            assert_eq!(engine.figure_ids(), vec![1]);
            assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
        }
        let values = matrust::Matrix::from_column_slice(1, 3, &[1., 1., 2.]).unwrap();
        let request = Request {
            x: Some(("x".into(), values.clone().into())),
            y: ("y".into(), values.into()),
            kind: PlotKind::Bar,
            log_x: false,
            log_y: false,
        };
        assert!(
            request
                .create(&mut engine)
                .unwrap_err()
                .contains("strictly increasing")
        );
        assert!(original.same_snapshot(engine.figure.as_ref().unwrap()));
    }
}

#[cfg(test)]
mod complex_boundary_tests {
    use super::*;
    use matrust::{Complex64, ComplexMatrix};
    #[test]
    fn unsupported_complex_plot_kinds_reject_before_script_generation_or_figure_commit() {
        let cancel = AtomicBool::new(false);
        for imaginary in [0., 3.] {
            let input = ComplexMatrix::scalar(Complex64::new(2., imaginary), &cancel).unwrap();
            let request = Request {
                x: None,
                y: ("Z".into(), input.clone().into()),
                kind: PlotKind::Scatter,
                log_x: false,
                log_y: false,
            };
            assert!(request.validate().unwrap_err().contains("complex"));
            assert!(request.script().is_err());
            assert!(request.build(&cancel).is_err());
            let mut engine = Engine::default();
            engine.execute("plot([1 2],[3 4]);").unwrap();
            let figure = engine.figure.clone().unwrap();
            let ids = engine.figure_ids();
            let current = engine.current_figure;
            assert!(request.create(&mut engine).is_err());
            assert_eq!(engine.figure_ids(), ids);
            assert_eq!(engine.current_figure, current);
            assert!(engine.figure.as_ref().unwrap().same_snapshot(&figure));
            // Unsupported kinds reject complex X even when Y is real.
            let request = Request {
                x: Some(("Z".into(), input.into())),
                y: ("Y".into(), matrust::Matrix::scalar(1.).into()),
                kind: PlotKind::Scatter,
                log_x: false,
                log_y: false,
            };
            assert!(request.script().is_err());
        }
    }

    #[test]
    fn native_complex_line_requests_match_generated_scripts_and_preserve_sources() {
        for source in [
            "Z=complex([1 2 3],[4 5 6]);X=[7 8 9];",
            "Z=complex([1 2;3 4;5 6],0);X=complex([7 8 9],10);",
            "Z=logical([0 1 0]);X=complex([7 8 9],10);",
        ] {
            for implicit in [true, false] {
                let mut engine = Engine::default();
                engine.execute(source).unwrap();
                let vars = engine.workspace();
                let request = Request {
                    x: (!implicit).then(|| ("X".into(), vars["X"].clone())),
                    y: ("Z".into(), vars["Z"].clone()),
                    kind: PlotKind::Line,
                    log_x: true,
                    log_y: false,
                };
                let expected = request.build(&AtomicBool::new(false)).unwrap();
                let script = request.script().unwrap();
                engine.execute(&script).unwrap();
                let actual = engine.figure.as_ref().unwrap();
                assert_eq!(expected.series.len(), actual.series.len());
                for (a, b) in expected.series.iter().zip(&actual.series) {
                    assert_eq!(a.points, b.points);
                }
                assert_eq!(expected.xlabel, actual.xlabel);
                assert_eq!(expected.ylabel, actual.ylabel);
                assert_eq!(expected.xscale, actual.xscale);
                assert_eq!(expected.yscale, actual.yscale);
                if implicit && vars["Z"].is_complex() {
                    assert_eq!(expected.xlabel, "real(Z)");
                    assert_eq!(expected.ylabel, "imag(Z)");
                }
                for name in ["X", "Z"] {
                    assert!(engine.array(name).unwrap().same_snapshot(&vars[name]));
                }
            }
        }
    }

    #[test]
    fn native_complex_line_cancel_empty_and_shape_failure_do_not_publish() {
        let mut engine = Engine::default();
        engine
            .execute("Z=complex([1 2],[3 4]);X=[1 2 3];plot([0 1],[2 3]);")
            .unwrap();
        let old = engine.figure.clone().unwrap();
        let ids = engine.figure_ids();
        let current = engine.current_figure;
        let mut request = Request {
            x: Some(("X".into(), engine.array("X").unwrap())),
            y: ("Z".into(), engine.array("Z").unwrap()),
            kind: PlotKind::Line,
            log_x: false,
            log_y: false,
        };
        assert!(request.create(&mut engine).is_err());
        request.x = None;
        engine.cancellation().store(true, Ordering::Relaxed);
        assert!(request.create(&mut engine).is_err());
        engine.cancellation().store(false, Ordering::Relaxed);
        request.y.1 = ComplexMatrix::filled(0, 3, Complex64::new(0., 0.), &AtomicBool::new(false))
            .unwrap()
            .into();
        assert!(request.validate().unwrap_err().contains("empty"));
        assert!(request.create(&mut engine).is_err());
        assert_eq!(engine.figure_ids(), ids);
        assert_eq!(engine.current_figure, current);
        assert!(engine.figure.as_ref().unwrap().same_snapshot(&old));
    }
}
