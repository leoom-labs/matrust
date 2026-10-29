//! Immutable native FIR request. Coefficients are built on the existing worker.
use matrust::{
    Result,
    dsp::{Band, FirOptions, WindowKind},
    simulation::Block,
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

pub const WINDOWS: [(WindowKind, &str, &str); 9] = [
    (WindowKind::Hamming, "Hamming", "hamming"),
    (WindowKind::Hann, "Hann", "hann"),
    (WindowKind::Blackman, "Blackman", "blackman"),
    (WindowKind::Bartlett, "Bartlett", "bartlett"),
    (WindowKind::Triangular, "Triangular", "triang"),
    (WindowKind::Rectangular, "Rectangular", "rectwin"),
    (
        WindowKind::BlackmanHarris,
        "Blackman-Harris",
        "blackmanharris",
    ),
    (WindowKind::Nuttall, "Nuttall", "nuttallwin"),
    (WindowKind::FlatTop, "Flat top", "flattopwin"),
];

#[derive(Clone, Copy, Debug)]
pub struct Design {
    pub order: usize,
    pub band: Band,
    pub sample_rate: f64,
    pub cutoff: f64,
    pub upper: f64,
    pub window: WindowKind,
    pub scale: bool,
}
impl Design {
    pub fn window_info(self) -> (&'static str, &'static str) {
        let (_, label, function) = WINDOWS
            .iter()
            .find(|(kind, _, _)| *kind == self.window)
            .unwrap();
        (label, function)
    }
    pub fn effective_order(self) -> Result<usize> {
        if !(1..=matrust::dsp::MAX_ORDER).contains(&self.order) {
            return Err("FIR order must be 1..4096".into());
        }
        let order = self.order
            + usize::from(
                matches!(self.band, Band::Highpass | Band::Bandstop)
                    && !self.order.is_multiple_of(2),
            );
        if order > matrust::dsp::MAX_ORDER {
            return Err("Promoted FIR order exceeds 4096".into());
        }
        Ok(order)
    }
    /// Constant-size admission only; the shared kernel remains authoritative.
    pub fn validate(self) -> Result<()> {
        let order = self.effective_order()?;
        if !self.sample_rate.is_finite() || self.sample_rate <= 0. {
            return Err("Sample rate must be positive and finite".into());
        }
        let (cuts, count) = self.cutoffs();
        if cuts[..count]
            .iter()
            .any(|x| !x.is_finite() || *x <= 0. || *x >= 1.)
            || (count == 2 && cuts[0] >= cuts[1])
        {
            return Err("Edges must increase and lie strictly between zero and Nyquist".into());
        }
        if self.scale
            && order == 1
            && matches!(
                self.window,
                WindowKind::Hann | WindowKind::Blackman | WindowKind::Bartlett
            )
        {
            return Err(
                "This two-tap window is all zero; increase the order or turn normalization off"
                    .into(),
            );
        }
        Ok(())
    }
    fn cutoffs(self) -> ([f64; 2], usize) {
        let normalized = |f: f64| {
            let doubled = f * 2.;
            if doubled.is_finite() {
                doubled / self.sample_rate
            } else {
                2. * (f / self.sample_rate)
            }
        };
        (
            [normalized(self.cutoff), normalized(self.upper)],
            if matches!(self.band, Band::Bandpass | Band::Bandstop) {
                2
            } else {
                1
            },
        )
    }
    pub fn summary(self) -> Result<String> {
        self.validate()?;
        Ok(format!(
            "{} - {} - {} taps{}",
            self.window_info().0,
            if self.scale { "scaled" } else { "unscaled" },
            self.effective_order()? + 1,
            if self.effective_order()? != self.order {
                " (order promoted)"
            } else {
                ""
            }
        ))
    }
    pub fn script(self) -> Result<String> {
        self.validate()?;
        let (cuts, count) = self.cutoffs();
        let cut = if count == 2 {
            format!("[{} {}]", cuts[0], cuts[1])
        } else {
            cuts[0].to_string()
        };
        // Retain the old Hamming/scaled call exactly; its default kernel stays unchanged.
        let options = if self.window == WindowKind::Hamming {
            if self.scale {
                String::new()
            } else {
                ", 'noscale'".into()
            }
        } else {
            format!(
                ", {}({}), '{}'",
                self.window_info().1,
                self.effective_order()? + 1,
                if self.scale { "scale" } else { "noscale" }
            )
        };
        Ok(format!(
            "b = fir1({}, {}, '{}'{});",
            self.order,
            cut,
            self.band.name(),
            options
        ))
    }
    pub fn exported_script(self) -> Result<String> {
        let script = self.script()?;
        Ok(format!(
            "% Sample rate: {} Hz; {}\n{script}\n",
            self.sample_rate,
            self.summary()?
        ))
    }
    pub fn block(self, cancel: &AtomicBool) -> Result<Block> {
        if cancel.load(Ordering::Relaxed) {
            return Err("Filter design cancelled".into());
        }
        self.validate()?;
        let window = if self.window == WindowKind::Hamming {
            None
        } else {
            Some(matrust::dsp::window(
                self.window,
                self.effective_order()? + 1,
                false,
                cancel,
            )?)
        };
        let (cuts, count) = self.cutoffs();
        let b = matrust::dsp::fir1_with_options(
            self.order,
            &cuts[..count],
            self.band,
            FirOptions {
                window: window.as_ref().map(|w| w.data()),
                scale: self.scale,
            },
            cancel,
        )?;
        if cancel.load(Ordering::Relaxed) {
            return Err("Filter design cancelled".into());
        }
        Ok(Block::Filter { b, a: vec![1.] })
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
    let check = || {
        if cancel.load(Ordering::Relaxed) {
            Err("Filter operation cancelled".to_owned())
        } else {
            Ok(())
        }
    };
    check()?;
    let script = request.design.exported_script()?;
    before_publish();
    check()?;
    matrust::files::atomic_write(&request.path, script.as_bytes())?;
    // Like IIR, entering atomic_write starts a noncancellable write/sync/rename
    // section. Report a completed publication honestly even after a late Stop.
    Ok(request.path)
}

impl crate::Workspace {
    pub(super) fn submit_fir_save(&mut self, request: Save) {
        if self.busy {
            return;
        }
        if let Err(error) = self.toolkits.begin_fir_save(request.design) {
            self.status = error.clone();
            self.toolkits.message = error;
            return;
        }
        self.submit(crate::worker::Job::SaveFirDesign(Box::new(request)));
        if self.busy {
            self.status = self.toolkits.message.clone();
        } else {
            self.toolkits.finish_design_save(Err(self.status.clone()));
        }
    }

    pub(super) fn submit_fir_block(&mut self, request: Design) {
        if self.busy {
            return;
        }
        match self.toolkits.begin_fir_block(request) {
            Ok(()) => {
                self.submit(crate::worker::Job::FirBlock(request));
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
}

#[cfg(test)]
mod submission_tests {
    use super::*;
    #[test]
    fn fir_designer_failed_worker_submission_clears_pending_state_and_allows_retry() {
        let mut app = crate::Workspace::with_context(eframe::egui::Context::default(), true);
        let original = app.toolkits.model.clone();
        let request = Design {
            order: 64,
            band: Band::Lowpass,
            sample_rate: 1000.,
            cutoff: 50.,
            upper: 150.,
            window: WindowKind::Hamming,
            scale: true,
        };
        app.worker.shutdown(); // Disconnect the real worker request channel.
        app.submit_fir_block(request);
        assert!(!app.busy);
        assert!(app.toolkits.model.same_storage(&original));
        assert!(!app.toolkits.message.starts_with("Designing"));
        app.toolkits
            .begin_fir_block(request)
            .expect("failed send must clear pending state");
        assert!(!app.toolkits.finish_filter_block(Err("test cleanup".into())));
    }
}

#[cfg(test)]
mod save_tests {
    use super::*;
    fn design() -> Design {
        Design {
            order: 7,
            band: Band::Highpass,
            sample_rate: 1000.,
            cutoff: 50.,
            upper: 150.,
            window: WindowKind::Hann,
            scale: false,
        }
    }
    #[test]
    fn fir_save_cancellation_and_invalid_design_preserve_existing_destination() {
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
        invalid.order = 0;
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
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    #[test]
    fn fir_save_atomic_failure_and_workspace_relative_binding_are_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("directory");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("keep"), b"original").unwrap();
        assert!(
            save(
                Save {
                    design: design(),
                    path: destination.clone()
                },
                &AtomicBool::new(false)
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read(destination.join("keep")).unwrap(),
            b"original"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        let request = crate::worker::Job::SaveFirDesign(Box::new(Save {
            design: design(),
            path: PathBuf::from("relative.m"),
        }))
        .in_folder(dir.path())
        .unwrap();
        let crate::worker::Job::SaveFirDesign(request) = request else {
            panic!("FIR save expected")
        };
        assert_eq!(request.path, dir.path().join("relative.m"));
    }
    #[test]
    fn fir_save_send_failure_cleans_pending_state_and_late_stop_reports_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("design.m");
        std::fs::write(&path, b"original").unwrap();
        let mut app = crate::Workspace::with_context(eframe::egui::Context::default(), true);
        let original = app.toolkits.model.clone();
        app.worker.shutdown();
        app.submit_fir_save(Save {
            design: design(),
            path: path.clone(),
        });
        assert!(!app.busy);
        assert!(app.toolkits.model.same_storage(&original));
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        app.toolkits
            .begin_fir_save(design())
            .expect("failed send clears pending state");
        app.worker.cancel.store(false, Ordering::Relaxed);
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
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            design().exported_script().unwrap()
        );
        assert!(app.toolkits.model.same_storage(&original));
        app.toolkits
            .begin_fir_save(design())
            .expect("completion clears pending state");
        app.complete_design_save(Err("test cleanup".into()));
    }
}
