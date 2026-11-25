use matrust::Engine;

#[test]
fn user_launcher_functions_keep_output_targets_and_input_names() {
    let function = "function [a,b] = imtool(x)
        assert(nargout == 2);
        a = x;
        if x == 3
            b = inputname(1);
        end
    end";
    let calls =
        "[v,~] = imtool(2); source = 3; [w,name] = imtool(source); assert(strcmp(name,'source'));";
    let folder = tempfile::tempdir().unwrap();
    let apps = folder.path().join("apps");
    std::fs::create_dir(&apps).unwrap();
    std::fs::write(apps.join("imtool.m"), function).unwrap();

    for (location, cwd, script) in [
        ("local", folder.path(), format!("{calls}\n{function}")),
        ("cwd", apps.as_path(), calls.to_owned()),
        (
            "addpath",
            folder.path(),
            format!("addpath('apps'); {calls}"),
        ),
    ] {
        let mut engine = Engine::default();
        engine.set_current_folder(cwd).unwrap();
        engine
            .execute(&script)
            .unwrap_or_else(|error| panic!("{location}: {error}"));
        assert_eq!(engine.variables["v"].data(), &[2.], "{location}");
        assert_eq!(engine.variables["w"].data(), &[3.], "{location}");
    }
}

#[test]
fn launcher_counts_expanded_inputs() {
    let mut engine = Engine::default();
    for (call, expected) in [
        ("C={}; imtool(C{:})", "imtool requires the desktop"),
        (
            "wrapper()\nfunction wrapper(varargin)\nimtool(varargin{:});\nend",
            "imtool requires the desktop",
        ),
        ("C={}; [x,y]=imtool(C{:})", "imtool: expected no outputs"),
        (
            "C={1,2}; imtool(C{:})",
            "imtool with arguments is not supported; call imtool with no inputs",
        ),
    ] {
        let error = engine.execute(call).unwrap_err();
        assert!(error.ends_with(expected), "{call}: {error}");
    }
}

#[test]
fn indirect_launcher_calls_use_requested_output_count() {
    let mut engine = Engine::default();
    engine.execute("h=@imtool;").unwrap();
    for call in ["h()", "feval('imtool')", "feval(h)"] {
        let error = engine.execute(call).unwrap_err();
        assert!(
            error.ends_with("imtool requires the desktop"),
            "{call}: {error}"
        );
        let error = engine.execute(&format!("x = {call};")).unwrap_err();
        assert!(
            error.ends_with("imtool: expected no outputs"),
            "{call}: {error}"
        );
        let presentation = matrust::engine::app_launchers::present();
        engine.execute(call).unwrap();
        let launches = presentation.take();
        assert_eq!(launches.len(), 1, "{call}");
        assert_eq!(
            launches[0],
            matrust::engine::app_launchers::Launch::App(matrust::session::ToolTab::ImageViewer)
        );
    }
}

#[test]
fn app_launchers_require_the_desktop_and_no_inputs_or_outputs() {
    let mut engine = Engine::default();
    engine.execute("obj = inputParser;").unwrap();
    for name in [
        "signalAnalyzer",
        "fvtool",
        "windowDesigner",
        "signalMultiresolutionAnalyzer",
        "waveletSignalDenoiser",
        "waveletTimeFrequencyAnalyzer",
        "pidTuner",
        "controlSystemDesigner",
        "linearSystemAnalyzer",
        "modelReducer",
        "systemIdentification",
        "curveFitter",
        "cftool",
        "splineTool",
        "distributionFitter",
        "disttool",
        "randtool",
        "classificationLearner",
        "regressionLearner",
        "imageViewer",
        "imtool",
        "imageSegmenter",
        "colorThresholder",
        "imageRegionAnalyzer",
        "imageBatchProcessor",
        "dataCleaner",
        "uiimport",
    ] {
        for call in [name.to_owned(), format!("{name}()")] {
            let error = engine.execute(&call).unwrap_err();
            assert!(
                error.ends_with(&format!("{name} requires the desktop")),
                "{call}: {error}"
            );
        }
        for call in [
            format!("{name}(1)"),
            format!("{name}(obj)"),
            format!("{name}(zeros(2,2,3))"),
            format!("{name} filename"),
        ] {
            let error = engine.execute(&call).unwrap_err();
            assert!(
                error.ends_with(&format!(
                    "{name} with arguments is not supported; call {name} with no inputs"
                )),
                "{call}: {error}"
            );
        }
        for call in [format!("x = {name}()"), format!("[x,y] = {name}()")] {
            let error = engine.execute(&call).unwrap_err();
            assert!(
                error.ends_with(&format!("{name}: expected no outputs")),
                "{call}: {error}"
            );
        }
        engine.execute(&format!("code = exist('{name}');")).unwrap();
        assert_eq!(engine.variables["code"].data(), &[5.], "{name}");
        let which = engine
            .execute(&format!("which {name}"))
            .unwrap()
            .output
            .join("\n");
        assert!(
            which.contains("built-in (") && which.ends_with(&format!("{name})")),
            "{name}: {which}"
        );
    }
}
