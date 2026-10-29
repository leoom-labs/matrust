use matrust::{Engine, Matrix};
use std::sync::atomic::Ordering;

#[test]
fn reshape_inference_and_size_vectors_keep_typed_column_major_aliases() {
    let mut e = Engine::default();
    e.execute("A=[1 2 3;4 5 6];B=reshape(A,[],2);C=reshape(A,[3 2]);D=reshape(A,2,[],1,1);L=logical(A);M=reshape(L,3,[]);Z=complex(A,-A);W=reshape(Z,[1 6 1]);T=reshape(Z,[],6);U=reshape(A,2,3,[]);").unwrap();
    for name in ["B", "C", "D", "U"] {
        assert_eq!(
            e.variables["A"].data().as_ptr(),
            e.variables[name].data().as_ptr()
        );
        assert_eq!(e.variables[name].data(), &[1., 4., 2., 5., 3., 6.]);
    }
    assert_eq!((e.variables["B"].rows(), e.variables["B"].cols()), (3, 2));
    assert_eq!((e.variables["D"].rows(), e.variables["D"].cols()), (2, 3));
    assert_eq!(
        e.logical_variables["L"].data().as_ptr(),
        e.logical_variables["M"].data().as_ptr()
    );
    for name in ["W", "T"] {
        assert!(e.complex_variables["Z"].shares_storage(&e.complex_variables[name]));
        assert_eq!(
            (
                e.complex_variables[name].rows(),
                e.complex_variables[name].cols()
            ),
            (1, 6)
        );
    }
    e.execute("B(1)=99;M(1)=false;W(1)=0;").unwrap();
    assert_eq!(e.variables["A"].data()[0], 1.);
    assert_eq!(e.logical_variables["L"].data()[0], 1);
    assert_eq!(e.complex_variables["Z"].data()[0].im, -1.);
}

#[test]
fn empty_inference_retains_shapes_and_types_without_dividing_by_zero() {
    let mut e = Engine::default();
    e.execute("A=zeros(0,7);B=reshape(A,[],2);C=reshape(A,3,[]);D=reshape(A,0,[]);E=reshape(A,[],0);F=reshape(A,[5 0 1]);L=reshape(logical(A),[],9);Z=reshape(complex(A),8,[]);R=reshape([],zeros(1,0),2);S=reshape([],2,zeros(0,1));").unwrap();
    for (name, shape) in [
        ("B", (0, 2)),
        ("C", (3, 0)),
        ("D", (0, 0)),
        ("E", (0, 0)),
        ("F", (5, 0)),
        ("R", (0, 2)),
        ("S", (2, 0)),
    ] {
        assert_eq!((e.variables[name].rows(), e.variables[name].cols()), shape);
        assert!(e.variables[name].is_empty());
    }
    assert_eq!(
        (
            e.logical_variables["L"].rows(),
            e.logical_variables["L"].cols()
        ),
        (0, 9)
    );
    assert_eq!(
        (
            e.complex_variables["Z"].rows(),
            e.complex_variables["Z"].cols()
        ),
        (8, 0)
    );
}

#[test]
fn constructors_accept_size_vectors_negative_sizes_and_trailing_singletons() {
    let mut e = Engine::default();
    e.execute("A=[1 2 3;4 5 6];Z=zeros(size(A));O=ones(size(complex(A)));T=ones([2 3 1 1]);S=zeros(2,3,1);N=ones(-2);R=zeros([-3 4]);C=ones(5,-1);Q=zeros(-1e300,2);").unwrap();
    for (name, value) in [("Z", 0.), ("O", 1.), ("T", 1.), ("S", 0.)] {
        assert_eq!((e.variables[name].rows(), e.variables[name].cols()), (2, 3));
        assert_eq!(e.variables[name].data(), &[value; 6]);
    }
    for (name, shape) in [("N", (0, 0)), ("R", (0, 4)), ("C", (5, 0)), ("Q", (0, 2))] {
        assert_eq!((e.variables[name].rows(), e.variables[name].cols()), shape);
    }
    e.execute("P=zeros(true);F=ones(false);B=ones(true,true);")
        .unwrap();
    assert_eq!(e.variables["P"].data(), &[0.]);
    assert_eq!((e.variables["F"].rows(), e.variables["F"].cols()), (0, 0));
    assert_eq!(e.variables["B"].data(), &[1.]);
    e.execute("K=zeros(size(A),'like',A);J=ones(2,'like',single(1));")
        .unwrap();
    assert_eq!(e.variables["K"].data(), &[0.; 6]);
    assert_eq!((e.variables["K"].rows(), e.variables["K"].cols()), (2, 3));
    assert_eq!(e.array("J").unwrap().class(), "single");
}

#[test]
fn shape_rejections_are_atomic_and_bounds_apply_even_to_empty_outputs() {
    let mut e = Engine::default();
    e.execute("saved=7;A=1:6;").unwrap();
    for rhs in [
        "reshape(A,[],4)",
        "reshape(A,[],[])",
        "reshape(A,0,[])",
        "reshape(A,[6])",
        "reshape(A,[2;3])",
        "reshape(A,1,1,[])",
        "reshape(A,[],2,0)",
        "reshape(A,-2,-3)",
        "reshape(A,1.5,4)",
        "reshape(A,NaN,2)",
        "reshape(A,Inf,2)",
        "reshape([],0,16777217)",
        "reshape([],zeros(0,2),2)",
        "zeros([2;3])",
        "ones([2 3 2])",
        "zeros(-1,16777217)",
        "ones(4097,4097)",
        "zeros(NaN,0)",
        "zeros(-Inf,0)",
        "zeros(-0.5,2)",
        "ones(2,2,'int9')",
        "zeros(size(A),'like',struct())",
        "reshape(A,complex(2),3)",
    ] {
        assert!(
            e.execute(&format!("saved={rhs};")).is_err(),
            "accepted {rhs}"
        );
        assert_eq!(e.variables["saved"].data(), &[7.]);
    }
    let too_many = vec!["1"; 33].join(",");
    assert!(e.execute(&format!("saved=zeros({too_many});")).is_err());
    assert_eq!(e.variables["saved"].data(), &[7.]);
}

#[test]
fn constructors_and_reshapes_are_bit_identical_under_one_and_eight_worker_pools() {
    let mut reference = None;
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        let mut e = Engine::default();
        pool.install(|| e.execute("A=ones([8193 8]);Z=zeros(size(A));B=reshape(A,8,[]);"))
            .unwrap();
        let bits: Vec<_> = e.variables["B"]
            .data()
            .iter()
            .map(|v| v.to_bits())
            .collect();
        if let Some(old) = &reference {
            assert_eq!(&bits, old);
        } else {
            reference = Some(bits);
        }
        assert!(e.variables["Z"].data().iter().all(|v| v.to_bits() == 0));
        assert_eq!(
            e.variables["A"].data().as_ptr(),
            e.variables["B"].data().as_ptr()
        );
        e.cancellation().store(true, Ordering::Relaxed);
        assert!(
            e.execute("A=ones([8193 8]);")
                .unwrap_err()
                .contains("cancelled")
        );
    }
}

#[test]
fn exported_empty_series_data_fragment_executes_without_copy_or_shape_loss() {
    // This exact data fragment is emitted by the MATLAB plot-script writer.
    // Full exported script execution also needs the separate graphics-handle layer.
    let mut e = Engine::default();
    e.variables
        .insert("mr_data".into(), Matrix::filled(0, 0, 0.).unwrap());
    e.execute("mr_data = reshape(mr_data,[],2); mr_y=ones(size(mr_data));")
        .unwrap();
    assert_eq!(
        (e.variables["mr_data"].rows(), e.variables["mr_data"].cols()),
        (0, 2)
    );
    assert_eq!(
        (e.variables["mr_y"].rows(), e.variables["mr_y"].cols()),
        (0, 2)
    );
}
