//! Native IIR admission is cheap; the existing worker computes and saves SOS.
use matrust::{Result, dsp::Band, simulation::Block};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Butterworth,
    Chebyshev1,
}
impl Family {
    pub fn label(self) -> &'static str {
        match self {
            Self::Butterworth => "Butterworth",
            Self::Chebyshev1 => "Chebyshev I",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Design {
    pub family: Family,
    pub ripple_db: f64,
    pub order: usize,
    pub band: Band,
    pub sample_rate: f64,
    pub cutoff: f64,
    pub upper: f64,
}
impl Design {
    pub fn cutoff(self) -> f64 {
        normalize(self.cutoff, self.sample_rate)
    }
    pub fn cutoffs(self) -> ([f64; 2], usize) {
        (
            [self.cutoff(), normalize(self.upper, self.sample_rate)],
            if matches!(self.band, Band::Bandpass | Band::Bandstop) {
                2
            } else {
                1
            },
        )
    }
    pub fn validate(self) -> Result<()> {
        if !self.sample_rate.is_finite() || self.sample_rate <= 0. {
            return Err("Sample rate must be positive and finite".into());
        }
        if self.family == Family::Chebyshev1
            && (!self.ripple_db.is_finite() || self.ripple_db <= 0.)
        {
            return Err("Chebyshev I passband ripple must be positive and finite (dB)".into());
        }
        if !(1..=20).contains(&self.order) {
            return Err(format!("{} order must be 1..20", self.family.label()));
        }
        let cut = self.cutoff();
        if !cut.is_finite() || cut <= 0. || cut >= 1. {
            return Err(format!(
                "{} cutoff must be strictly between zero and Nyquist",
                self.family.label()
            ));
        }
        let (cuts, count) = self.cutoffs();
        if count == 2 && (!cuts[1].is_finite() || cuts[1] <= cuts[0] || cuts[1] >= 1.) {
            return Err(format!(
                "{} edges must increase and lie strictly between zero and Nyquist",
                self.family.label()
            ));
        }
        Ok(())
    }
    pub fn script(self) -> Result<String> {
        self.validate()?;
        let (cuts, count) = self.cutoffs();
        let edges = if count == 2 {
            format!("[{} {}]", cuts[0], cuts[1])
        } else {
            cuts[0].to_string()
        };
        if self.family == Family::Chebyshev1 {
            return Ok(format!(
                "sos = cheby1_sos({}, {:e}, {}, '{}');",
                self.order,
                self.ripple_db,
                edges,
                self.band.name()
            ));
        }
        Ok(format!(
            "sos = butter_sos({}, {}, '{}');",
            self.order,
            edges,
            self.band.name()
        ))
    }
    pub fn summary(self) -> Result<String> {
        self.validate()?;
        let name = match self.family {
            Family::Butterworth => "Butterworth".to_owned(),
            Family::Chebyshev1 => format!("Chebyshev I ({:e} dB ripple)", self.ripple_db),
        };
        if matches!(self.band, Band::Bandpass | Band::Bandstop) {
            Ok(format!(
                "{name} {} - prototype {} / result order {} - {} SOS sections",
                self.band.name(),
                self.order,
                2 * self.order,
                self.order
            ))
        } else {
            Ok(format!(
                "{name} {} - order {} - {} SOS sections",
                self.band.name(),
                self.order,
                self.order.div_ceil(2)
            ))
        }
    }
    pub(super) fn coefficients(self, cancel: &AtomicBool) -> Result<matrust::Matrix> {
        check(cancel)?;
        self.validate()?;
        let (cuts, count) = self.cutoffs();
        let sos = match self.family {
            Family::Butterworth => matrust::dsp::butter_sos_with_cutoffs(
                self.order,
                &cuts[..count],
                self.band,
                cancel,
            )?,
            Family::Chebyshev1 => matrust::dsp::cheby1(
                self.order,
                self.ripple_db,
                &cuts[..count],
                self.band,
                cancel,
            )?
            .sos(cancel)?,
        };
        check(cancel)?;
        Ok(sos)
    }
    pub fn block(self, cancel: &AtomicBool) -> Result<Block> {
        let sos = self.coefficients(cancel)?;
        let sections = (0..sos.rows())
            .map(|r| std::array::from_fn(|c| sos.data()[c * sos.rows() + r]))
            .collect();
        check(cancel)?;
        Ok(Block::SosFilter { sections })
    }
    pub fn portable_script(self, cancel: &AtomicBool) -> Result<String> {
        let sos = self.coefficients(cancel)?;
        let mut rows = Vec::with_capacity(sos.rows());
        for r in 0..sos.rows() {
            check(cancel)?;
            rows.push(
                (0..6)
                    .map(|c| format!("{:.17e}", sos.data()[c * sos.rows() + r]))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        Ok(format!(
            "% {} SOS, sample rate {} Hz. Compatible with MATLAB sosfilt.\nsos = [{}];\n",
            match self.family {
                Family::Butterworth => "Butterworth".to_owned(),
                Family::Chebyshev1 => format!("Chebyshev I, ripple {} dB", self.ripple_db),
            },
            self.sample_rate,
            rows.join(";\n")
        ))
    }
}
fn normalize(frequency: f64, sample_rate: f64) -> f64 {
    let doubled = frequency * 2.;
    if doubled.is_finite() {
        doubled / sample_rate
    } else {
        2. * (frequency / sample_rate)
    }
}
fn check(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err("Filter operation cancelled".into())
    } else {
        Ok(())
    }
}
pub struct Save {
    pub design: Design,
    pub path: PathBuf,
}
pub fn save(request: Save, cancel: &AtomicBool) -> Result<PathBuf> {
    save_before_publish(request, cancel, || {})
}
fn save_before_publish(
    request: Save,
    cancel: &AtomicBool,
    before_publish: impl FnOnce(),
) -> Result<PathBuf> {
    let script = request.design.portable_script(cancel)?;
    before_publish();
    check(cancel)?;
    matrust::files::atomic_write(&request.path, script.as_bytes())?;
    // Atomic publication may finish while Stop arrives. Report that saved result;
    // do not claim the previous file survived a cancellation after publication.
    Ok(request.path)
}

impl crate::Workspace {
    pub(super) fn complete_design_save(&mut self, result: Result<PathBuf>) {
        self.toolkits.finish_design_save(result);
        self.status = self.toolkits.message.clone();
    }
    pub(super) fn submit_iir_block(&mut self, request: Design) {
        if self.busy {
            return;
        }
        match self.toolkits.begin_iir_block(request) {
            Ok(()) => {
                self.submit(crate::worker::Job::IirBlock(request));
                if self.busy {
                    self.status = self.toolkits.message.clone();
                } else {
                    self.toolkits.finish_filter_block(Err(self.status.clone()));
                }
            }
            Err(error) => {
                self.status = error.clone();
                self.toolkits.message = error;
            }
        }
    }
    pub(super) fn submit_iir_save(&mut self, request: Save) {
        if self.busy {
            return;
        }
        if let Err(error) = self.toolkits.begin_iir_save(request.design) {
            self.status = error.clone();
            self.toolkits.message = error;
            return;
        }
        self.submit(crate::worker::Job::SaveIirDesign(Box::new(request)));
        if self.busy {
            self.status = self.toolkits.message.clone();
        } else {
            self.toolkits.finish_design_save(Err(self.status.clone()));
        }
    }
}

#[cfg(test)]
mod io_tests {
    use super::*;
    fn design() -> Design {
        Design {
            family: Family::Butterworth,
            ripple_db: 1.,
            order: 6,
            band: Band::Lowpass,
            sample_rate: 1000.,
            cutoff: 50.,
            upper: 150.,
        }
    }
    #[test]
    fn iir_save_cancel_before_design_and_before_publication_preserves_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("design.m");
        std::fs::write(&path, b"original").unwrap();
        let cancel = AtomicBool::new(true);
        assert!(
            save(
                Save {
                    design: design(),
                    path: path.clone()
                },
                &cancel
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        cancel.store(false, Ordering::Relaxed);
        assert!(
            save_before_publish(
                Save {
                    design: design(),
                    path: path.clone()
                },
                &cancel,
                || cancel.store(true, Ordering::Relaxed)
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        let mut invalid = design();
        invalid.order = 21;
        assert!(
            save(
                Save {
                    design: invalid,
                    path: path.clone()
                },
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no temporary output remains"
        );
    }
    #[test]
    fn iir_save_atomic_failure_keeps_destination_and_request_paths_bind_to_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("is-a-directory");
        std::fs::create_dir(&destination).unwrap();
        let keep = destination.join("keep");
        std::fs::write(&keep, b"original").unwrap();
        assert!(
            save(
                Save {
                    design: design(),
                    path: destination
                },
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert_eq!(std::fs::read(keep).unwrap(), b"original");
        let job = crate::worker::Job::SaveIirDesign(Box::new(Save {
            design: design(),
            path: PathBuf::from("relative.m"),
        }))
        .in_folder(dir.path())
        .unwrap();
        let crate::worker::Job::SaveIirDesign(request) = job else {
            panic!("save request");
        };
        assert_eq!(request.path, dir.path().join("relative.m"));
    }
    #[test]
    fn iir_worker_submit_failure_clears_block_and_save_pending_state() {
        let mut app = crate::Workspace::with_context(eframe::egui::Context::default(), true);
        let original = app.toolkits.model.clone();
        app.worker.shutdown();
        app.submit_iir_block(design());
        assert!(!app.busy && app.toolkits.model.same_storage(&original));
        app.toolkits
            .begin_iir_block(design())
            .expect("block send failure clears pending state");
        app.toolkits.finish_filter_block(Err("test cleanup".into()));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("design.m");
        std::fs::write(&path, b"original").unwrap();
        app.submit_iir_save(Save {
            design: design(),
            path: path.clone(),
        });
        assert!(!app.busy && app.toolkits.model.same_storage(&original));
        assert_eq!(std::fs::read(path).unwrap(), b"original");
        app.toolkits
            .begin_iir_save(design())
            .expect("save send failure clears pending state");
        app.complete_design_save(Err("test cleanup".into()));
    }
    #[test]
    fn iir_save_late_stop_reports_completed_atomic_publication_honestly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("design.m");
        let mut app = crate::Workspace::with_context(eframe::egui::Context::default(), true);
        app.toolkits.begin_iir_save(design()).unwrap();
        let result = save(
            Save {
                design: design(),
                path: path.clone(),
            },
            &app.worker.cancel,
        );
        assert!(result.is_ok());
        app.worker.cancel.store(true, Ordering::Relaxed);
        app.complete_design_save(result);
        assert!(app.status.starts_with("Saved"));
        assert!(
            std::fs::read_to_string(path)
                .unwrap()
                .starts_with("% Butterworth SOS")
        );
    }
}
