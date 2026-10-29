//! Capture-only controls. Ordinary desktop theme and timing remain unchanged.
use super::egui;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IirDesignerCase {
    Bandpass1,
    Bandpass20,
    Bandstop1,
    Bandstop20,
}
impl IirDesignerCase {
    pub(super) fn settings(self) -> (matrust::dsp::Band, usize) {
        use matrust::dsp::Band;
        match self {
            Self::Bandpass1 => (Band::Bandpass, 1),
            Self::Bandpass20 => (Band::Bandpass, 20),
            Self::Bandstop1 => (Band::Bandstop, 1),
            Self::Bandstop20 => (Band::Bandstop, 20),
        }
    }
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Bandpass1 => "bandpass-1",
            Self::Bandpass20 => "bandpass-20",
            Self::Bandstop1 => "bandstop-1",
            Self::Bandstop20 => "bandstop-20",
        }
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct Options {
    pub path: Option<PathBuf>,
    pub ready_frames: u8,
    pub theme: Option<egui::ThemePreference>,
    pub script_reload_demo: bool,
    pub model_new_demo: bool,
    pub figure_selector_demo: bool,
    pub iir_designer_case: Option<IirDesignerCase>,
}

impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut args = args.into_iter();
        let mut path = None;
        let mut frames = None;
        let mut theme = None;
        let mut script_reload_demo = false;
        let mut model_new_demo = false;
        let mut figure_selector_demo = false;
        let mut iir_designer_case = None;
        let mut iir_designer_demo = false;
        let mut isolated = false;
        let mut fir_designer_demo = false;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                // The existing file queue owns all later arguments as paths.
                "--open-files" => break,
                "--preview-image" | "--preview-page" | "--preview-pdf-page" => {
                    args.next();
                }
                "--isolated" => isolated = true,
                "--iir-designer-demo" => iir_designer_demo = true,
                "--designer-demo" => fir_designer_demo = true,
                "--iir-designer-case" => {
                    if iir_designer_case.is_some() {
                        return Err("Supply --iir-designer-case only once".into());
                    }
                    iir_designer_case = Some(match args.next().as_deref() {
                        Some("bandpass-1") => IirDesignerCase::Bandpass1,
                        Some("bandpass-20") => IirDesignerCase::Bandpass20,
                        Some("bandstop-1") => IirDesignerCase::Bandstop1,
                        Some("bandstop-20") => IirDesignerCase::Bandstop20,
                        _ => return Err("--iir-designer-case requires bandpass-1, bandpass-20, bandstop-1 or bandstop-20".into()),
                    });
                }
                "--screenshot" => {
                    if path.is_some() {
                        return Err("Supply --screenshot only once".into());
                    }
                    let value = args.next().ok_or("--screenshot requires a filename")?;
                    if value.is_empty() || value.starts_with("--") {
                        return Err("--screenshot requires a filename".into());
                    }
                    path = Some(PathBuf::from(value));
                }
                "--figure-selector-demo" => {
                    if figure_selector_demo {
                        return Err("Supply --figure-selector-demo only once".into());
                    }
                    figure_selector_demo = true;
                }
                "--model-new-demo" => {
                    if model_new_demo {
                        return Err("Supply --model-new-demo only once".into());
                    }
                    model_new_demo = true;
                }
                "--script-reload-demo" => {
                    if script_reload_demo {
                        return Err("Supply --script-reload-demo only once".into());
                    }
                    script_reload_demo = true;
                }
                "--screenshot-frames" => {
                    if frames.is_some() {
                        return Err("Supply --screenshot-frames only once".into());
                    }
                    frames = Some(match args.next().as_deref() {
                        Some("1") => 1,
                        Some("2") => 2,
                        Some("3") => 3,
                        _ => {
                            return Err(
                                "--screenshot-frames requires 1, 2 or 3 ready frames".into()
                            );
                        }
                    });
                }
                "--theme" => {
                    if theme.is_some() {
                        return Err("Supply --theme only once".into());
                    }
                    theme = Some(match args.next().as_deref() {
                        Some("light") => egui::ThemePreference::Light,
                        Some("dark") => egui::ThemePreference::Dark,
                        _ => return Err("--theme requires light or dark".into()),
                    });
                }
                _ => {} // Existing desktop/demo options remain owned by their consumers.
            }
        }
        if path.is_none() && (frames.is_some() || theme.is_some()) {
            return Err("--theme and --screenshot-frames require --screenshot FILE".into());
        }
        if script_reload_demo && path.is_none() {
            return Err("--script-reload-demo requires --screenshot FILE".into());
        }
        if model_new_demo && path.is_none() {
            return Err("--model-new-demo requires --screenshot FILE".into());
        }
        if model_new_demo && script_reload_demo {
            return Err("Choose only one model-new or script-reload capture action".into());
        }
        if figure_selector_demo && (path.is_none() || frames.is_some_and(|n| n != 3)) {
            return Err(
                "--figure-selector-demo requires --screenshot FILE and three ready frames".into(),
            );
        }
        if figure_selector_demo && (model_new_demo || script_reload_demo) {
            return Err(
                "Choose only one figure-selector, model-new or script-reload capture action".into(),
            );
        }
        if iir_designer_case.is_some() {
            if path.is_none() || !iir_designer_demo || !isolated {
                return Err("--iir-designer-case requires --screenshot FILE, --iir-designer-demo and --isolated".into());
            }
            if fir_designer_demo || script_reload_demo || model_new_demo || figure_selector_demo {
                return Err("Choose only the IIR Designer capture action".into());
            }
        }
        Ok(Self {
            path,
            ready_frames: frames.unwrap_or(3),
            theme,
            script_reload_demo,
            model_new_demo,
            figure_selector_demo,
            iir_designer_case,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn screenshot_controls_default_and_bounded_explicit_options() {
        let ordinary = parse(&["--designer-demo"]).unwrap();
        assert_eq!(
            ordinary,
            Options {
                path: None,
                ready_frames: 3,
                theme: None,
                script_reload_demo: false,
                model_new_demo: false,
                figure_selector_demo: false,
                iir_designer_case: None,
            }
        );
        for theme in ["light", "dark"] {
            for frames in ["1", "2", "3"] {
                let options = parse(&[
                    "--theme",
                    theme,
                    "--screenshot",
                    "a b.png",
                    "--screenshot-frames",
                    frames,
                ])
                .unwrap();
                assert_eq!(options.path, Some(PathBuf::from("a b.png")));
                assert_eq!(options.ready_frames.to_string(), frames);
                assert_eq!(
                    options.theme,
                    Some(if theme == "dark" {
                        egui::ThemePreference::Dark
                    } else {
                        egui::ThemePreference::Light
                    })
                );
            }
        }
        assert_eq!(
            parse(&["--screenshot", "capture.png"])
                .unwrap()
                .ready_frames,
            3
        );
    }

    #[test]
    fn figure_selector_capture_guard_duplicates_and_open_paths() {
        assert!(parse(&["--figure-selector-demo"]).is_err());
        assert!(
            parse(&["--screenshot", "x.png", "--figure-selector-demo"])
                .unwrap()
                .figure_selector_demo
        );
        for extra in [
            "--figure-selector-demo",
            "--model-new-demo",
            "--script-reload-demo",
        ] {
            assert!(parse(&["--screenshot", "x.png", "--figure-selector-demo", extra]).is_err());
        }
        assert!(
            parse(&[
                "--screenshot",
                "x.png",
                "--figure-selector-demo",
                "--screenshot-frames",
                "1"
            ])
            .is_err()
        );
        assert!(
            !parse(&[
                "--screenshot",
                "x.png",
                "--open-files",
                "--figure-selector-demo"
            ])
            .unwrap()
            .figure_selector_demo
        );
    }

    #[test]
    fn model_new_capture_requires_screenshot_and_one_action_and_respects_paths() {
        assert!(parse(&["--model-new-demo"]).is_err());
        assert!(
            parse(&["--screenshot", "x.png", "--model-new-demo"])
                .unwrap()
                .model_new_demo
        );
        assert!(
            parse(&[
                "--screenshot",
                "x.png",
                "--model-new-demo",
                "--model-new-demo"
            ])
            .is_err()
        );
        assert!(
            parse(&[
                "--screenshot",
                "x.png",
                "--model-new-demo",
                "--script-reload-demo"
            ])
            .is_err()
        );
        assert!(
            !parse(&["--screenshot", "x.png", "--open-files", "--model-new-demo"])
                .unwrap()
                .model_new_demo
        );
        for path_flag in ["--preview-image", "--preview-page", "--preview-pdf-page"] {
            assert!(
                !parse(&[path_flag, "--model-new-demo"])
                    .unwrap()
                    .model_new_demo
            );
        }
        assert!(
            !parse(&["--open-files", "--model-new-demo", "--screenshot", "x.png"])
                .unwrap()
                .model_new_demo
        );
    }

    #[test]
    fn script_reload_demo_requires_capture_and_respects_file_tail() {
        assert!(parse(&["--script-reload-demo"]).is_err());
        assert!(
            parse(&["--screenshot", "x.png", "--script-reload-demo"])
                .unwrap()
                .script_reload_demo
        );
        assert!(
            parse(&[
                "--screenshot",
                "x.png",
                "--script-reload-demo",
                "--script-reload-demo"
            ])
            .is_err()
        );
        assert!(
            !parse(&[
                "--screenshot",
                "x.png",
                "--open-files",
                "--script-reload-demo"
            ])
            .unwrap()
            .script_reload_demo
        );
        assert!(
            !parse(&["--preview-image", "--script-reload-demo"])
                .unwrap()
                .script_reload_demo
        );
    }

    #[test]
    fn screenshot_controls_reject_invalid_or_non_capture_configuration() {
        for args in [
            vec!["--theme", "dark"],
            vec!["--screenshot-frames", "1"],
            vec!["--screenshot"],
            vec!["--screenshot", ""],
            vec!["--screenshot", "--theme", "dark"],
            vec!["--screenshot", "x.png", "--theme", "system"],
            vec!["--screenshot", "x.png", "--theme"],
            vec![
                "--screenshot",
                "x.png",
                "--theme",
                "light",
                "--theme",
                "dark",
            ],
            vec!["--screenshot", "x.png", "--screenshot", "y.png"],
            vec![
                "--screenshot",
                "x.png",
                "--screenshot-frames",
                "1",
                "--screenshot-frames",
                "3",
            ],
        ] {
            assert!(parse(&args).is_err(), "{args:?}");
        }
        for value in ["0", "4", "255", "-1", "1.5", "NaN", "01", ""] {
            assert!(parse(&["--screenshot", "x.png", "--screenshot-frames", value]).is_err());
        }
    }

    #[test]
    fn screenshot_controls_do_not_consume_file_queue_or_preview_path_values() {
        let options = parse(&[
            "--screenshot",
            "x.png",
            "--open-files",
            "--theme",
            "dark",
            "--screenshot-frames",
            "0",
        ])
        .unwrap();
        assert_eq!(options.theme, None);
        assert_eq!(options.ready_frames, 3);
        assert_eq!(parse(&["--preview-image", "--theme"]).unwrap().theme, None);
        assert_eq!(
            parse(&["--open-files", "--screenshot", "x.png"])
                .unwrap()
                .path,
            None
        );
    }
}

#[cfg(test)]
mod iir_capture_tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::parse(args.iter().map(|s| (*s).to_owned()))
    }
    #[test]
    fn iir_capture_accepts_only_four_isolated_screenshot_cases() {
        for (name, band, order) in [
            ("bandpass-1", matrust::dsp::Band::Bandpass, 1),
            ("bandpass-20", matrust::dsp::Band::Bandpass, 20),
            ("bandstop-1", matrust::dsp::Band::Bandstop, 1),
            ("bandstop-20", matrust::dsp::Band::Bandstop, 20),
        ] {
            let options = parse(&[
                "--screenshot",
                "a b.png",
                "--isolated",
                "--iir-designer-demo",
                "--iir-designer-case",
                name,
            ])
            .unwrap();
            let case = options.iir_designer_case.unwrap();
            assert_eq!(case.settings(), (band, order));
            assert_eq!(case.name(), name);
        }
        for args in [
            vec!["--iir-designer-case", "bandpass-1"],
            vec![
                "--screenshot",
                "x.png",
                "--iir-designer-case",
                "bandpass-1",
                "--isolated",
            ],
            vec![
                "--screenshot",
                "x.png",
                "--iir-designer-case",
                "bandpass-1",
                "--iir-designer-demo",
            ],
        ] {
            assert!(parse(&args).is_err());
        }
        let required = [
            "--screenshot",
            "x.png",
            "--isolated",
            "--iir-designer-demo",
            "--iir-designer-case",
        ];
        for invalid in [
            "",
            "bandpass-0",
            "bandpass-21",
            "bandpass-2",
            "bandstop-01",
            "lowpass-1",
            "NaN",
            "--compact-demo",
        ] {
            let mut args = required.to_vec();
            args.push(invalid);
            assert!(parse(&args).is_err());
        }
        assert!(parse(&required).is_err());
        for conflict in [
            "--designer-demo",
            "--model-new-demo",
            "--script-reload-demo",
            "--figure-selector-demo",
        ] {
            let mut args = required.to_vec();
            args.extend(["bandpass-1", conflict]);
            assert!(parse(&args).is_err());
        }
        let mut duplicate = required.to_vec();
        duplicate.extend(["bandstop-1", "--iir-designer-case", "bandpass-20"]);
        assert!(parse(&duplicate).is_err());
    }
    #[test]
    fn iir_capture_preserves_ordinary_demo_and_path_tail_ownership() {
        assert!(
            parse(&["--iir-designer-demo"])
                .unwrap()
                .iir_designer_case
                .is_none()
        );
        assert!(
            parse(&[
                "--screenshot",
                "x.png",
                "--open-files",
                "--iir-designer-case",
                "bandpass-1"
            ])
            .unwrap()
            .iir_designer_case
            .is_none()
        );
        assert!(
            parse(&["--preview-image", "--iir-designer-case"])
                .unwrap()
                .iir_designer_case
                .is_none()
        );
        assert!(
            parse(&[
                "--screenshot",
                "x.png",
                "--isolated",
                "--iir-designer-case",
                "bandpass-1",
                "--open-files",
                "--iir-designer-demo"
            ])
            .is_err()
        );
    }
}
