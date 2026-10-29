use super::*;
use std::{
    cell::Cell,
    sync::atomic::{AtomicBool, Ordering},
};
const CHAIN: &[u8] =
    include_bytes!("../../../tests/fixtures/slx/slx-inherited-rate-admission-1526/chain.slx");
const FEEDBACK: &[u8] =
    include_bytes!("../../../tests/fixtures/slx/slx-inherited-rate-admission-1526/feedback.slx");
const DIAGRAM: &str = include_str!(
    "../../../tests/fixtures/slx/slx-inherited-rate-admission-1526/chain-blockdiagram.xml"
);
const CONFIG: &str =
    include_str!("../../../tests/fixtures/slx/slx-import-1512/chain-configSet0.xml");
thread_local! { static STOP_AT: Cell<Option<usize>> = const { Cell::new(None) }; }
pub(super) fn checkpoint(cancel: &AtomicBool) {
    STOP_AT.with(|slot| {
        if let Some(n) = slot.get() {
            if n == 0 {
                cancel.store(true, Ordering::Relaxed);
                slot.set(None);
            } else {
                slot.set(Some(n - 1));
            }
        }
    });
}
fn decoded(diagram: &str, config: &str) -> Result<crate::simulation::Model> {
    let c = AtomicBool::new(false);
    graph::decode(
        &xml::parse(diagram.as_bytes(), &c)?,
        &xml::parse(config.as_bytes(), &c)?,
        &c,
    )
}
fn fixture(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("source.slx");
    std::fs::write(&p, bytes).unwrap();
    (dir, p)
}

#[test]
fn inherited_state_requires_a_proven_discrete_input_not_just_solver_step() {
    // Preserve the original authored all-inherited graphs as real public-import
    // negatives. Rejection describes importer uncertainty, not MATLAB invalidity.
    for bytes in [
        include_bytes!("../../../tests/fixtures/slx/slx-import-1512/chain.slx").as_slice(),
        include_bytes!("../../../tests/fixtures/slx/slx-import-1512/feedback.slx").as_slice(),
    ] {
        let (_dir, path) = fixture(bytes);
        let error = import(&path, &AtomicBool::new(false)).unwrap_err();
        assert!(
            error.contains("unresolved inherited UnitDelay sample time"),
            "{error}"
        );
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    let inherited=DIAGRAM.replace(
        "<Block BlockType=\"UnitDelay\"><P Name=\"InitialCondition\">5</P><P Name=\"SampleTime\">.25</P>",
        "<Block BlockType=\"UnitDelay\"><P Name=\"InitialCondition\">5</P><P Name=\"SampleTime\">-1</P>");
    assert_ne!(inherited, DIAGRAM);
    assert!(
        decoded(&inherited, CONFIG)
            .unwrap_err()
            .contains("unresolved inherited")
    );
    // A saved discrete source proves the rate through Gain and UnitDelay.
    let proven = inherited.replace(
        "<P Name=\"SampleTime\">inf</P>",
        "<P Name=\"SampleTime\">.25</P>",
    );
    let model = decoded(&proven, CONFIG).unwrap();
    assert_eq!(
        model.run(&AtomicBool::new(false)).unwrap().traces[0]
            .values
            .data(),
        &[5., -6., -6., -6., -6.]
    );
    // Another disconnected discrete clock cannot establish Delay's input rate.
    let unrelated=inherited.replace("<System>","<System><Block BlockType=\"Constant\" Name=\"Unrelated\" SID=\"99\"><P Name=\"Position\">[0,100,40,130]</P><P Name=\"SampleTime\">.25</P></Block>");
    assert!(
        decoded(&unrelated, CONFIG)
            .unwrap_err()
            .contains("unresolved inherited")
    );
    let wrong = proven.replace(
        "<P Name=\"SampleTime\">.25</P>",
        "<P Name=\"SampleTime\">.5</P>",
    );
    assert!(decoded(&wrong, CONFIG).unwrap_err().contains("single rate"));
    // The all-constant stateless graph remains admitted; no state clock is needed.
    let stateless = inherited.replace(
        "BlockType=\"UnitDelay\" Name=\"Delay\"",
        "BlockType=\"Gain\" Name=\"Delay\"",
    );
    assert_eq!(
        decoded(&stateless, CONFIG)
            .unwrap()
            .run(&AtomicBool::new(false))
            .unwrap()
            .traces[0]
            .values
            .data(),
        &[18.; 5]
    );
}

#[test]
fn authored_public_archive_recurrence_endpoint_layout_and_native_roundtrip() {
    for (bytes, expected) in [
        (CHAIN, vec![5., -6., -6., -6., -6.]),
        (FEEDBACK, vec![5., 7., 9., 11., 13.]),
    ] {
        let (_dir, path) = fixture(bytes);
        let c = AtomicBool::new(false);
        let loaded = import(&path, &c).unwrap();
        assert_eq!(
            loaded.source_blake3,
            blake3::hash(bytes).to_hex().to_string()
        );
        assert_eq!(loaded.model.steps, 5);
        assert_eq!(loaded.model.sample_time, 0.25);
        let result = loaded.model.run(&c).unwrap();
        assert_eq!(result.time.data(), &[0., 0.25, 0.5, 0.75, 1.]);
        assert_eq!(result.traces[0].values.data(), expected);
        let save = path.with_extension("mrsim");
        loaded.model.save(&save).unwrap();
        assert!(
            loaded
                .model
                .same_storage(&crate::simulation::Model::load(&save).unwrap())
        );
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    let m = decoded(DIAGRAM, CONFIG).unwrap();
    assert_eq!(
        m.positions,
        vec![[0., 30.], [120., 30.], [220., 30.], [320., 30.]]
    );
    let zero = CONFIG.replace(">1</P>", ">0</P>");
    assert_eq!(decoded(DIAGRAM, &zero).unwrap().steps, 1);
}

#[test]
fn explicit_defaults_overrides_signed_zero_and_allocation_free_shared_fanout() {
    let changed = DIAGRAM.replace("<P Name=\"Value\">2</P>", "<P Name=\"Value\">-0</P>");
    let model = decoded(&changed, CONFIG).unwrap();
    let crate::simulation::Block::Constant { value } = model.nodes[0].block else {
        panic!("constant")
    };
    assert_eq!(value.to_bits(), (-0f64).to_bits());
    let override_value = changed.replace(
        "Name=\"Input\" SID=\"1\">",
        "Name=\"Input\" SID=\"1\"><P Name=\"Value\">4</P>",
    );
    let m = decoded(&override_value, CONFIG).unwrap();
    assert_eq!(
        m.run(&AtomicBool::new(false)).unwrap().traces[0]
            .values
            .data(),
        &[5., -12., -12., -12., -12.]
    );
    let (_dir, path) = fixture(FEEDBACK);
    let m = import(&path, &AtomicBool::new(false)).unwrap().model;
    assert_eq!(m.nodes[0].inputs, vec![1]);
    assert_eq!(m.nodes[3].inputs, vec![2, 1]);
}

#[test]
fn unsupported_time_types_callbacks_graphs_and_missing_defaults_are_not_substituted() {
    for (a, b, message) in [
        ("FixedStepDiscrete", "ode1", "Solver"),
        (".25", "auto", "numeric scalar"),
        ("Name=\"StopTime\">1", "Name=\"StopTime\">.3", "exactly"),
        (
            "Name=\"LoadExternalInput\">off",
            "Name=\"LoadExternalInput\">on",
            "LoadExternalInput",
        ),
    ] {
        let error = decoded(DIAGRAM, &CONFIG.replace(a, b)).unwrap_err();
        assert!(error.contains(message), "{error}");
    }
    for (a, b, message) in [
        ("<P Name=\"InitialCondition\">5</P>", "", "InitialCondition"),
        (
            "<P Name=\"Value\">2</P>",
            "<P Name=\"Value\">workspace_name</P>",
            "literal",
        ),
        (
            "<P Name=\"OutDataTypeStr\">double</P>",
            "<P Name=\"OutDataTypeStr\">int8</P>",
            "OutDataTypeStr",
        ),
        (
            "Name=\"SampleTime\">-1",
            "Name=\"SampleTime\">0",
            "single rate",
        ),
        (
            "<System>",
            "<P Name=\"InitFcn\">delete('x')</P><System>",
            "callback",
        ),
        ("3#in:1", "3#in:2", "port"),
        (
            "BlockType=\"Gain\" Name=\"Gain\"",
            "BlockType=\"Integrator\" Name=\"Gain\"",
            "unsupported block type",
        ),
    ] {
        let error = decoded(&DIAGRAM.replace(a, b), CONFIG).unwrap_err();
        assert!(error.contains(message), "{error}");
    }
}

#[test]
fn admitted_scalar_block_parameters_and_destination_port_order_use_existing_kernels() {
    for (kind, extra, values, expected) in [
        (
            "Gain",
            "<P Name=\"Gain\">-3</P><P Name=\"Multiplication\">Matrix(K*u)</P>",
            vec![2.],
            -6.,
        ),
        ("Abs", "", vec![-2.], 2.),
        ("UnaryMinus", "", vec![2.], -2.),
        ("Bias", "<P Name=\"Bias\">3</P>", vec![2.], 5.),
        (
            "Saturate",
            "<P Name=\"LowerLimit\">-1</P><P Name=\"UpperLimit\">1</P>",
            vec![2.],
            1.,
        ),
        ("Sum", "<P Name=\"Inputs\">++</P>", vec![2., 7.], 9.),
        (
            "Product",
            "<P Name=\"Inputs\">*/</P><P Name=\"Multiplication\">Element-wise(.*)</P>",
            vec![8., 2.],
            4.,
        ),
        (
            "Switch",
            "<P Name=\"Criteria\">u2 &gt; Threshold</P><P Name=\"Threshold\">0</P>",
            vec![2., -1., 7.],
            7.,
        ),
    ] {
        let mut body = String::new();
        for (i, value) in values.iter().enumerate() {
            body.push_str(&format!("<Block BlockType=\"Constant\" Name=\"Source{i}\" SID=\"{}\"><P Name=\"Position\">[0,0,40,30]</P><P Name=\"Value\">{value}</P><P Name=\"OutDataTypeStr\">double</P><P Name=\"SampleTime\">inf</P></Block>",i+1));
        }
        body.push_str(&format!("<Block BlockType=\"{kind}\" Name=\"Operation\" SID=\"40\"><P Name=\"Position\">[100,0,140,30]</P><P Name=\"OutDataTypeStr\">double</P><P Name=\"SampleTime\">-1</P>{extra}</Block><Block BlockType=\"Scope\" Name=\"Output\" SID=\"41\"><P Name=\"Position\">[200,0,240,30]</P><P Name=\"Ports\">[1]</P><P Name=\"Floating\">off</P><P Name=\"NumInputPorts\">1</P></Block>"));
        // Reverse serialized line order; destination port numbering is authoritative.
        for i in (0..values.len()).rev() {
            body.push_str(&format!(
                "<Line><P Name=\"Src\">{}#out:1</P><P Name=\"Dst\">40#in:{}</P></Line>",
                i + 1,
                i + 1
            ));
        }
        body.push_str("<Line><P Name=\"Src\">40#out:1</P><P Name=\"Dst\">41#in:1</P></Line>");
        let xml = format!(
            "<ModelInformation><Model><BlockDefaults><P Name=\"BlockRotation\">0</P><P Name=\"BlockMirror\">off</P></BlockDefaults><BlockParameterDefaults/><System>{body}</System></Model></ModelInformation>"
        );
        let model = decoded(&xml, CONFIG).unwrap();
        let output = model.run(&AtomicBool::new(false)).unwrap();
        assert_eq!(output.traces[0].values.data(), &[expected; 5], "{kind}");
    }
}

#[test]
fn zip_and_xml_limits_reject_without_source_modification_and_cancel_every_checkpoint() {
    let cancel = AtomicBool::new(false);
    for xml in [
        "<!DOCTYPE x [<!ENTITY a 'x'>]><x/>",
        "<x a='1' a='2'/>",
        "<x><y></x>",
        "<x>&custom;</x>",
    ] {
        assert!(xml::parse(xml.as_bytes(), &cancel).is_err());
    }
    let deep = format!("{}{}", "<x>".repeat(33), "</x>".repeat(33));
    assert!(
        xml::parse(deep.as_bytes(), &cancel)
            .unwrap_err()
            .contains("depth")
    );
    let (_dir, path) = fixture(CHAIN);
    let mut succeeded = false;
    for at in 0..256 {
        cancel.store(false, Ordering::Relaxed);
        STOP_AT.with(|s| s.set(Some(at)));
        let result = import(&path, &cancel);
        STOP_AT.with(|s| s.set(None));
        if result.is_ok() {
            succeeded = true;
            break;
        }
        assert_eq!(result.unwrap_err(), "Calculation cancelled");
        assert_eq!(std::fs::read(&path).unwrap(), CHAIN);
    }
    assert!(
        succeeded,
        "all actual parsing checkpoints must have been visited"
    );
    let mut damaged = CHAIN.to_vec();
    let end = damaged.len() - 22;
    damaged[end + 10..end + 12].copy_from_slice(&513u16.to_le_bytes());
    assert!(archive::Package::new(&damaged, &cancel).is_err());
}

#[test]
#[ignore = "requires the named reference negative corpus relative to the current directory; never executes models"]
fn installed_original_models_retain_solver_hierarchy_and_callback_barriers() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/slx/slx-import-assessment-1512/evidence.json"
    ))
    .unwrap();
    for item in manifest["corpus"].as_array().unwrap() {
        let path = std::path::Path::new(item["path"].as_str().unwrap());
        let bytes = archive::read(path, &AtomicBool::new(false)).unwrap();
        assert_eq!(bytes.len() as u64, item["raw_bytes"].as_u64().unwrap());
        // Exact SHA256 origin is checked by the parent-owned corpus handoff;
        // the application adds no SHA dependency just for this ignored probe.
        let err = import(path, &AtomicBool::new(false)).unwrap_err();
        assert!(err.contains("SLX"), "{}: {err}", path.display());
        assert!(
            err.contains("Solver")
                || err.contains("numeric scalar")
                || err.contains("callback")
                || err.contains("missing Model")
                || err.contains("parameter text"),
            "{}: unexpected barrier {err}",
            path.display()
        );
    }
}

#[test]
fn empty_xml_elements_obey_the_same_exact_depth_boundary() {
    let c = AtomicBool::new(false);
    for parents in [31usize, 32] {
        let xml = format!("{}<leaf/>{}", "<x>".repeat(parents), "</x>".repeat(parents));
        let result = xml::parse(xml.as_bytes(), &c);
        if parents == 31 {
            assert!(result.is_ok());
        } else {
            assert!(result.unwrap_err().contains("depth limit"));
        }
    }
}

#[test]
fn earlier_zip_footer_cannot_bypass_preallocation_admission() {
    let c = AtomicBool::new(false);
    assert!(archive::Package::new(CHAIN, &c).is_ok());
    for earlier_count in [4u16, 513, u16::MAX] {
        // Retain the original archive, then append a small apparently admissible
        // footer pointing at a deliberately invalid one-byte central directory.
        // zip9 may otherwise retry the earlier footer, even beyond 65 KiB.
        let mut bytes = CHAIN.to_vec();
        let original_end = bytes.len() - 22;
        if earlier_count != 4 {
            bytes[original_end + 8..original_end + 10]
                .copy_from_slice(&earlier_count.to_le_bytes());
            bytes[original_end + 10..original_end + 12]
                .copy_from_slice(&earlier_count.to_le_bytes());
        }
        bytes.extend(std::iter::repeat_n(0u8, 65536));
        let central = bytes.len();
        bytes.push(0);
        let mut footer = [0u8; 22];
        footer[..4].copy_from_slice(b"PK\x05\x06");
        footer[8..10].copy_from_slice(&1u16.to_le_bytes());
        footer[10..12].copy_from_slice(&1u16.to_le_bytes());
        footer[12..16].copy_from_slice(&1u32.to_le_bytes());
        footer[16..20].copy_from_slice(&(central as u32).to_le_bytes());
        bytes.extend_from_slice(&footer);
        let result = archive::Package::new(&bytes, &c);
        assert!(matches!(result,Err(ref e) if e.contains("ambiguous ZIP directory footer")));
    }
    let mut trailing = CHAIN.to_vec();
    trailing.push(0);
    assert!(
        matches!(archive::Package::new(&trailing,&c),Err(ref e) if e.contains("trailing bytes"))
    );
}
