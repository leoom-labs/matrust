//! Tests for matrix and 'rows' set operations, ismember, and unique ('first'/'last'/'rows').
use matrust::{Array, Engine};

fn eval(code: &str) -> Engine {
    let mut e = Engine::default();
    e.execute(code).unwrap();
    e
}

fn mat(e: &Engine, name: &str) -> (usize, usize, Vec<f64>) {
    let a = &e.variables[name];
    (a.rows(), a.cols(), a.data().to_vec())
}

fn logical_mat(e: &Engine, name: &str) -> (usize, usize, Vec<u8>) {
    let a = e.array(name).unwrap();
    let Array::Logical(l) = a else {
        panic!("expected logical array for {name}");
    };
    (l.rows(), l.cols(), l.data().to_vec())
}

#[test]
fn unique_rows_and_first_last_occurrence() {
    let e = eval("[u,~,j]=unique([1 2;3 4;1 2],'rows');");
    assert_eq!(mat(&e, "u"), (2, 2, vec![1., 3., 2., 4.]));
    assert_eq!(mat(&e, "j"), (3, 1, vec![1., 2., 1.]));

    // Octave 10.3 cross-check:
    // tool/ref-octave.sh --eval "[~, i] = unique([3 1 3], 'last'); disp(size(i)); disp(i);"
    let e = eval("[~,i]=unique([3 1 3],'last'); [~,f]=unique([3 1 3],'first');");
    assert_eq!(mat(&e, "i"), (2, 1, vec![2., 3.]));
    assert_eq!(mat(&e, "f"), (2, 1, vec![2., 1.]));

    // Logical 'last' and 'stable'
    let e = eval("[c,i,j]=unique([true false true],'last'); [cs,is]=unique([3 1 3 2],'rows');");
    assert_eq!(logical_mat(&e, "c"), (1, 2, vec![0, 1]));
    assert_eq!(mat(&e, "i"), (2, 1, vec![2., 3.]));
    assert_eq!(mat(&e, "j"), (3, 1, vec![2., 1., 2.]));
    assert_eq!(mat(&e, "cs"), (1, 4, vec![3., 1., 3., 2.]));
    assert_eq!(mat(&e, "is"), (1, 1, vec![1.]));

    // 'stable' with 'rows'
    let e = eval("[u,i,j]=unique([3 4;1 2;3 4],'rows','stable');");
    assert_eq!(mat(&e, "u"), (2, 2, vec![3., 1., 4., 2.]));
    assert_eq!(mat(&e, "i"), (2, 1, vec![1., 2.]));
    assert_eq!(mat(&e, "j"), (3, 1, vec![1., 2., 1.]));
}

#[test]
fn ismember_matrices_and_rows() {
    let e = eval(
        "tf=ismember([1 2;5 6],[1 2;3 4],'rows'); [tf2,loc2]=ismember([1 2;5 6],[1 2;3 4],'rows');",
    );
    assert_eq!(logical_mat(&e, "tf"), (2, 1, vec![1, 0]));
    assert_eq!(logical_mat(&e, "tf2"), (2, 1, vec![1, 0]));
    assert_eq!(mat(&e, "loc2"), (2, 1, vec![1., 0.]));

    let e = eval("M=magic(3); [tf,loc]=ismember(M,[1 2]); exp=logical(M<=2);");
    assert_eq!(logical_mat(&e, "tf"), logical_mat(&e, "exp"));
    assert_eq!(mat(&e, "loc").0, 3);
    assert_eq!(mat(&e, "loc").1, 3);
    let m = mat(&e, "M").2;
    let want_loc: Vec<f64> = m
        .iter()
        .map(|&v| {
            if v == 1. {
                1.
            } else if v == 2. {
                2.
            } else {
                0.
            }
        })
        .collect();
    assert_eq!(mat(&e, "loc").2, want_loc);
}

#[test]
fn set_operations_rows_match_octave_reference() {
    // Octave 10.3 cross-check command:
    // tool/ref-octave.sh --eval "A=[1 2;3 4;5 6]; B=[3 4;7 8;1 2];
    //   [c,ia,ib]=intersect(A,B,'rows'); disp(c); disp(ia); disp(ib);
    //   [c,ia,ib]=union(A,B,'rows'); disp(c); disp(ia); disp(ib);
    //   [c,ia]=setdiff(A,B,'rows'); disp(c); disp(ia);
    //   [c,ia,ib]=setxor(A,B,'rows'); disp(c); disp(ia); disp(ib);"
    let e = eval(
        "A=[1 2;3 4;5 6]; B=[3 4;7 8;1 2];\
         [ci,iai,ibi]=intersect(A,B,'rows');\
         [cu,iau,ibu]=union(A,B,'rows');\
         [cd,iad]=setdiff(A,B,'rows');\
         [cx,iax,ibx]=setxor(A,B,'rows');",
    );
    assert_eq!(mat(&e, "ci"), (2, 2, vec![1., 3., 2., 4.]));
    assert_eq!(mat(&e, "iai"), (2, 1, vec![1., 2.]));
    assert_eq!(mat(&e, "ibi"), (2, 1, vec![3., 1.]));

    assert_eq!(mat(&e, "cu"), (4, 2, vec![1., 3., 5., 7., 2., 4., 6., 8.]));
    assert_eq!(mat(&e, "iau"), (3, 1, vec![1., 2., 3.]));
    assert_eq!(mat(&e, "ibu"), (1, 1, vec![2.]));

    assert_eq!(mat(&e, "cd"), (1, 2, vec![5., 6.]));
    assert_eq!(mat(&e, "iad"), (1, 1, vec![3.]));

    assert_eq!(mat(&e, "cx"), (2, 2, vec![5., 7., 6., 8.]));
    assert_eq!(mat(&e, "iax"), (1, 1, vec![3.]));
    assert_eq!(mat(&e, "ibx"), (1, 1, vec![2.]));
}

#[test]
fn set_operations_rows_stable_matches_octave_reference() {
    // Octave 10.3 cross-check command:
    // tool/ref-octave.sh --eval "A=[3 4;1 2;3 4]; B=[5 6;1 2;7 8];
    //   [c,ia,ib]=intersect(A,B,'rows','stable');
    //   [c,ia,ib]=union(A,B,'rows','stable');
    //   [c,ia]=setdiff(A,B,'rows','stable');
    //   [c,ia,ib]=setxor(A,B,'rows','stable');"
    let e = eval(
        "A=[3 4;1 2;3 4]; B=[5 6;1 2;7 8];\
         [ci,iai,ibi]=intersect(A,B,'rows','stable');\
         [cu,iau,ibu]=union(A,B,'rows','stable');\
         [cd,iad]=setdiff(A,B,'rows','stable');\
         [cx,iax,ibx]=setxor(A,B,'rows','stable');",
    );
    assert_eq!(mat(&e, "ci"), (1, 2, vec![1., 2.]));
    assert_eq!(mat(&e, "iai"), (1, 1, vec![2.]));
    assert_eq!(mat(&e, "ibi"), (1, 1, vec![2.]));

    assert_eq!(mat(&e, "cu"), (4, 2, vec![3., 1., 5., 7., 4., 2., 6., 8.]));
    assert_eq!(mat(&e, "iau"), (2, 1, vec![1., 2.]));
    assert_eq!(mat(&e, "ibu"), (2, 1, vec![1., 3.]));

    assert_eq!(mat(&e, "cd"), (1, 2, vec![3., 4.]));
    assert_eq!(mat(&e, "iad"), (1, 1, vec![1.]));

    assert_eq!(mat(&e, "cx"), (3, 2, vec![3., 5., 7., 4., 6., 8.]));
    assert_eq!(mat(&e, "iax"), (1, 1, vec![1.]));
    assert_eq!(mat(&e, "ibx"), (2, 1, vec![1., 3.]));
}

#[test]
fn matrix_inputs_without_rows_and_nan_rows_and_char_rows() {
    // Matrix inputs without 'rows' flatten column-major and return column vectors
    // (or row vectors when both inputs are rows, or setdiff(row, mat)).
    let e = eval(
        "A=[3 1;2 3]; B=[2 4;3 5];\
         u=unique(A); un=union(A,B); ix=intersect(A,B); sd=setdiff(A,B); sx=setxor(A,B);\
         sd_row=setdiff([1 2 3], [2 4;5 6]); un_row=union([1 2], [2 3;4 5]);",
    );
    assert_eq!(mat(&e, "u"), (3, 1, vec![1., 2., 3.]));
    assert_eq!(mat(&e, "un"), (5, 1, vec![1., 2., 3., 4., 5.]));
    assert_eq!(mat(&e, "ix"), (2, 1, vec![2., 3.]));
    assert_eq!(mat(&e, "sd"), (1, 1, vec![1.]));
    assert_eq!(mat(&e, "sx"), (3, 1, vec![1., 4., 5.]));
    assert_eq!(mat(&e, "sd_row"), (1, 2, vec![1., 3.]));
    assert_eq!(mat(&e, "un_row"), (5, 1, vec![1., 2., 3., 4., 5.]));

    // NaN rows remain distinct
    let e = eval(
        "A=[1 NaN; 1 2; 1 NaN];\
         [u,ia,ic]=unique(A,'rows');\
         [tf,loc]=ismember([1 NaN; 1 2],[1 NaN; 1 2],'rows');\
         ci=intersect([1 NaN; 1 2],[1 NaN; 1 2],'rows');",
    );
    assert_eq!(mat(&e, "u").0, 3);
    assert_eq!(mat(&e, "ia"), (3, 1, vec![2., 1., 3.]));
    assert_eq!(mat(&e, "ic"), (3, 1, vec![2., 1., 3.]));
    assert_eq!(logical_mat(&e, "tf"), (2, 1, vec![0, 1]));
    assert_eq!(mat(&e, "loc"), (2, 1, vec![0., 2.]));
    assert_eq!(mat(&e, "ci"), (1, 2, vec![1., 2.]));

    // Char rows
    let e = eval(
        "C=['ba';'ab';'ba']; [uc,ic1,ic2]=unique(C,'rows');\
         tfc=ismember(['ba';'zz'],['ab';'ba'],'rows');\
         cc=intersect(['ab';'ba'],['ba';'zz'],'rows');",
    );
    let char_text = |name: &str| -> String {
        let Array::Character(c) = e.array(name).unwrap() else {
            panic!("expected char array for {name}");
        };
        String::from_utf16(c.units()).unwrap()
    };
    assert_eq!(char_text("uc"), "abba");
    assert_eq!(e.array("uc").unwrap().rows(), 2);
    assert_eq!(e.array("uc").unwrap().cols(), 2);
    assert_eq!(mat(&e, "ic1"), (2, 1, vec![2., 1.]));
    assert_eq!(mat(&e, "ic2"), (3, 1, vec![2., 1., 2.]));
    assert_eq!(logical_mat(&e, "tfc"), (2, 1, vec![1, 0]));
    assert_eq!(char_text("cc"), "ba");

    // Column mismatch error for 'rows'
    let mut err_e = Engine::default();
    assert!(err_e.execute("ismember([1 2],[1 2 3],'rows')").is_err());
    assert!(err_e.execute("union([1 2],[1 2 3],'rows')").is_err());
}

#[test]
fn unique_stable_last_preserves_first_occurrence_order() {
    let e = eval(
        "[cr,ir,jr]=unique([1;2;1],'rows','stable','last');\
         [ce,ie,je]=unique([2;1;2],'stable','last');\
         [cm,im,jm]=unique([3 4;1 2;3 4;5 6;1 2],'rows','stable','last');",
    );
    assert_eq!(mat(&e, "cr"), (2, 1, vec![1., 2.]));
    assert_eq!(mat(&e, "ir"), (2, 1, vec![3., 2.]));
    assert_eq!(mat(&e, "jr"), (3, 1, vec![1., 2., 1.]));

    assert_eq!(mat(&e, "ce"), (2, 1, vec![2., 1.]));
    assert_eq!(mat(&e, "ie"), (2, 1, vec![3., 2.]));
    assert_eq!(mat(&e, "je"), (3, 1, vec![1., 2., 1.]));

    assert_eq!(mat(&e, "cm"), (3, 2, vec![3., 1., 5., 4., 2., 6.]));
    assert_eq!(mat(&e, "im"), (3, 1, vec![3., 5., 4.]));
    assert_eq!(mat(&e, "jm"), (5, 1, vec![1., 2., 1., 3., 2.]));
}

#[test]
fn typed_integer_and_single_rows_preserve_class() {
    let e = eval("a=uint8([1 2]); [t,i]=ismember(a,a,'rows'); [c,ia,ib]=intersect(a,a,'rows');");
    assert_eq!(logical_mat(&e, "t"), (1, 1, vec![1]));
    assert_eq!(mat(&e, "i"), (1, 1, vec![1.]));
    assert_eq!(e.array("c").unwrap().class(), "uint8");
    assert_eq!(mat(&e, "ia"), (1, 1, vec![1.]));
    assert_eq!(mat(&e, "ib"), (1, 1, vec![1.]));

    for class in [
        "int8", "uint8", "int16", "uint16", "int32", "uint32", "int64", "uint64", "single",
    ] {
        let script = format!(
            "A={class}([3 4;1 2;3 4]); B={class}([5 6;1 2;7 8]);\
             [tf,loc]=ismember(A,B,'rows');\
             [cu,iau,ibu]=union(A,B,'rows');\
             [ci,iai,ibi]=intersect(A,B,'rows');\
             [cd,iad]=setdiff(A,B,'rows');\
             [cx,iax,ibx]=setxor(A,B,'rows');\
             [cq,iq,jq]=unique(A,'rows','stable','last');"
        );
        let e = eval(&script);
        assert_eq!(logical_mat(&e, "tf"), (3, 1, vec![0, 1, 0]), "{class}");
        assert_eq!(mat(&e, "loc"), (3, 1, vec![0., 2., 0.]), "{class}");
        for name in ["cu", "ci", "cd", "cx", "cq"] {
            assert_eq!(e.array(name).unwrap().class(), class, "{class}.{name}");
        }
        assert_eq!(mat(&e, "iai"), (1, 1, vec![2.]), "{class}");
        assert_eq!(mat(&e, "ibi"), (1, 1, vec![2.]), "{class}");
        assert_eq!(mat(&e, "iau"), (2, 1, vec![2., 1.]), "{class}");
        assert_eq!(mat(&e, "ibu"), (2, 1, vec![1., 3.]), "{class}");
        assert_eq!(mat(&e, "iad"), (1, 1, vec![1.]), "{class}");
        assert_eq!(mat(&e, "iax"), (1, 1, vec![1.]), "{class}");
        assert_eq!(mat(&e, "ibx"), (2, 1, vec![1., 3.]), "{class}");
        assert_eq!(mat(&e, "iq"), (2, 1, vec![3., 2.]), "{class}");
        assert_eq!(mat(&e, "jq"), (3, 1, vec![1., 2., 1.]), "{class}");
    }

    // 64-bit integer precision > 2^53 is preserved in comparisons
    let e = eval(
        "A=[int64(9007199254740993) int64(1); int64(9007199254740992) int64(1)];\
         [tf,loc]=ismember(A,A(1,:),'rows');\
         [ci,~,~]=intersect(A,A(1,:),'rows');",
    );
    assert_eq!(logical_mat(&e, "tf"), (2, 1, vec![1, 0]));
    assert_eq!(mat(&e, "loc"), (2, 1, vec![1., 0.]));
    assert_eq!(e.array("ci").unwrap().class(), "int64");
    assert_eq!(e.array("ci").unwrap().rows(), 1);
}

#[test]
fn string_scalar_flags_accepted_everywhere_char_flags_are() {
    let e = eval(
        "u=unique([2;1;2],\"rows\");\
         [us,is,js]=unique([1;2;1],\"rows\",\"stable\",\"last\");\
         [tf,loc]=ismember([1 2;3 4],[3 4;1 2],\"rows\");\
         [cu,iau,ibu]=union([3 4;1 2],[1 2;5 6],\"rows\",\"stable\");\
         ci=intersect([3 4;1 2],[1 2;5 6],\"rows\",\"sorted\");\
         cd=setdiff([3 4;1 2],[1 2;5 6],\"rows\");\
         cx=setxor([3 4;1 2],[1 2;5 6],string('rows'));",
    );
    assert_eq!(mat(&e, "u"), (2, 1, vec![1., 2.]));
    assert_eq!(mat(&e, "us"), (2, 1, vec![1., 2.]));
    assert_eq!(mat(&e, "is"), (2, 1, vec![3., 2.]));
    assert_eq!(mat(&e, "js"), (3, 1, vec![1., 2., 1.]));
    assert_eq!(logical_mat(&e, "tf"), (2, 1, vec![1, 1]));
    assert_eq!(mat(&e, "loc"), (2, 1, vec![2., 1.]));
    assert_eq!(mat(&e, "cu"), (3, 2, vec![3., 1., 5., 4., 2., 6.]));
    assert_eq!(mat(&e, "iau"), (2, 1, vec![1., 2.]));
    assert_eq!(mat(&e, "ibu"), (1, 1, vec![2.]));
    assert_eq!(mat(&e, "ci"), (1, 2, vec![1., 2.]));
    assert_eq!(mat(&e, "cd"), (1, 2, vec![3., 4.]));
    assert_eq!(mat(&e, "cx"), (2, 2, vec![3., 5., 4., 6.]));

    let mut err_e = Engine::default();
    assert!(
        err_e
            .execute("unique([1 2],[\"rows\" \"stable\"])")
            .is_err()
    );
    assert!(
        err_e
            .execute("ismember([1 2],[1 2],[\"rows\" \"stable\"])")
            .is_err()
    );
    assert!(
        err_e
            .execute("union([1 2],[1 2],[\"rows\" \"stable\"])")
            .is_err()
    );
}
