//! No-input Apps menu commands. The desktop worker collects requests during a job.
use crate::{Result, session::ToolTab};
use std::{cell::RefCell, marker::PhantomData, rc::Rc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Launch {
    App(ToolTab),
    RandomSamples,
    AppsHome,
}

impl Launch {
    pub(super) fn for_name(name: &str) -> Option<Self> {
        let tab = match name {
            "signalAnalyzer" => ToolTab::SignalAnalyzer,
            "fvtool" => ToolTab::FilterVisualizer,
            "windowDesigner" => ToolTab::WindowDesigner,
            "signalMultiresolutionAnalyzer" => ToolTab::MultiresAnalyzer,
            "waveletSignalDenoiser" => ToolTab::WaveletDenoiser,
            "waveletTimeFrequencyAnalyzer" => ToolTab::WaveletTfAnalyzer,
            "pidTuner" => ToolTab::PidTuner,
            "controlSystemDesigner" => ToolTab::ControlSystemDesigner,
            "linearSystemAnalyzer" => ToolTab::LinearSystemAnalyzer,
            "modelReducer" => ToolTab::ModelReducer,
            "systemIdentification" => ToolTab::SystemIdentification,
            "curveFitter" | "cftool" => ToolTab::CurveFitter,
            "splineTool" => ToolTab::SplineTool,
            "distributionFitter" => ToolTab::DistributionFitter,
            "disttool" | "randtool" => ToolTab::DistributionExplorer,
            "classificationLearner" => ToolTab::ClassificationLearner,
            "regressionLearner" => ToolTab::RegressionLearner,
            "imageViewer" | "imtool" => ToolTab::ImageViewer,
            "imageSegmenter" => ToolTab::ImageSegmenter,
            "colorThresholder" => ToolTab::ColorThresholder,
            "imageRegionAnalyzer" => ToolTab::ImageRegionAnalyzer,
            "imageBatchProcessor" => ToolTab::ImageBatchProcessor,
            "dataCleaner" => ToolTab::DataCleaner,
            "uiimport" => ToolTab::ImportTool,
            _ => return None,
        };
        Some(if name == "randtool" {
            Self::RandomSamples
        } else {
            Self::App(tab)
        })
    }

    pub(super) fn open(self, name: &str, inputs: usize, outputs: usize) -> Result<()> {
        if inputs != 0 {
            return Err(format!(
                "{name} with arguments is not supported; call {name} with no inputs"
            ));
        }
        if outputs != 0 {
            return Err(format!("{name}: expected no outputs"));
        }
        self.record()
            .then_some(())
            .ok_or_else(|| format!("{name} requires the desktop"))
    }

    fn window(self) -> Option<ToolTab> {
        match self {
            Self::App(tab) => Some(tab),
            Self::RandomSamples => Some(ToolTab::DistributionExplorer),
            Self::AppsHome => None,
        }
    }

    /// Payload-bearing presentation hooks keep their own latest payload. Record
    /// their position alongside ordinary commands so the desktop can replay both.
    pub fn record(self) -> bool {
        PENDING.with_borrow_mut(|pending| {
            let Some(requests) = pending else {
                return false;
            };
            // At most one pending request per window, in order of last use.
            requests.retain(|old| old.window() != self.window());
            requests.push(self);
            true
        })
    }
}

thread_local! { static PENDING: RefCell<Option<Vec<Launch>>> = const { RefCell::new(None) }; }

/// Scoped to the desktop worker thread, like the other app presentation hooks.
pub struct Presentation {
    previous: Option<Vec<Launch>>,
    thread: PhantomData<Rc<()>>,
}

pub fn present() -> Presentation {
    Presentation {
        previous: PENDING.replace(Some(Vec::new())),
        thread: PhantomData,
    }
}

impl Presentation {
    pub fn take(&self) -> Vec<Launch> {
        PENDING.with_borrow_mut(|pending| std::mem::take(pending.as_mut().unwrap()))
    }
}

impl Drop for Presentation {
    fn drop(&mut self) {
        PENDING.set(self.previous.take());
    }
}
