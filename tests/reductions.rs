//! min/max/sum/mean/any/all/find in their MATLAB forms.
//! `REFERENCE_VALUES` contains cases mm*, f* and a*; `REFERENCE` contains the
//! broader batch. `REFERENCE_REV2` adds cases b*, c* and n* from review.
use matrust::{Array, Engine, LogicalMatrix, Matrix, matrix::Extreme};
use std::sync::atomic::{AtomicBool, Ordering};

const NAN: f64 = f64::NAN;
const INF: f64 = f64::INFINITY;
type Expected = (
    &'static str,
    &'static str,
    (usize, usize),
    &'static [f64],
    &'static [f64],
);

/// (id, statements, [(variable, class, shape, real parts, imaginary parts)])
type Case = (&'static str, &'static str, &'static [Expected]);

const REFERENCE_VALUES: &[Case] = &[
    (
        "mm1",
        "r=min([1 NaN 3]);",
        &[("r", "double", (1, 1), &[1.], &[])],
    ),
    (
        "mm2",
        "r=max([NaN NaN]);",
        &[("r", "double", (1, 1), &[NAN], &[])],
    ),
    (
        "mm3",
        "[m,i]=max([1 NaN 5 5]);",
        &[
            ("m", "double", (1, 1), &[5.], &[]),
            ("i", "double", (1, 1), &[3.], &[]),
        ],
    ),
    (
        "mm4",
        "[m,i]=min([NaN NaN]);",
        &[
            ("m", "double", (1, 1), &[NAN], &[]),
            ("i", "double", (1, 1), &[1.], &[]),
        ],
    ),
    (
        "mm5",
        "r=min([4 2;1 3],[],2);",
        &[("r", "double", (2, 1), &[2., 1.], &[])],
    ),
    (
        "mm6",
        "[m,i]=max([1 5;7 2],[],1); r=[m;i];",
        &[("r", "double", (2, 2), &[7., 2., 5., 1.], &[])],
    ),
    (
        "mm7",
        "r=max([1 NaN;NaN 2],[],2);",
        &[("r", "double", (2, 1), &[1., 2.], &[])],
    ),
    (
        "mm8",
        "r=min(5,[1 7 NaN]);",
        &[("r", "double", (1, 3), &[1., 5., 5.], &[])],
    ),
    // The report records the real part only; MatRust keeps complex storage.
    (
        "mm9",
        "r=real(max([1+1i 2 -3]));",
        &[("r", "double", (1, 1), &[-3.], &[])],
    ),
    (
        "f1",
        "r=find([0 1 1 0 1],2);",
        &[("r", "double", (1, 2), &[2., 3.], &[])],
    ),
    (
        "f2",
        "r=find([0 1 1 0 1],2,'last');",
        &[("r", "double", (1, 2), &[3., 5.], &[])],
    ),
    (
        "f3",
        "[i,j]=find([0 1;1 1]); r=[i j];",
        &[("r", "double", (3, 2), &[2., 1., 2., 1., 2., 2.], &[])],
    ),
    ("f4", "r=find([0 0]);", &[("r", "double", (1, 0), &[], &[])]),
    (
        "a1",
        "r=any([0 0;0 1]);",
        &[("r", "logical", (1, 2), &[0., 1.], &[])],
    ),
    (
        "a2",
        "r=all([1 1;0 1]);",
        &[("r", "logical", (1, 2), &[0., 1.], &[])],
    ),
    (
        "a3",
        "r=[any([]) all([])];",
        &[("r", "logical", (1, 2), &[0., 1.], &[])],
    ),
    (
        "a4",
        "r=any([0 0;0 1],2);",
        &[("r", "logical", (2, 1), &[0., 1.], &[])],
    ),
    (
        "a5",
        "r=all(zeros(0,3));",
        &[("r", "logical", (1, 3), &[1., 1., 1.], &[])],
    ),
];

/// Expected values for the broader batch, keyed by stable case identifiers.
const REFERENCE: &[Case] = &[
    (
        "mx1",
        "[m,k]=max([1 NaN 5 5]);",
        &[
            ("m", "double", (1, 1), &[5.0], &[]),
            ("k", "double", (1, 1), &[3.0], &[]),
        ],
    ),
    (
        "mx2",
        "[m,k]=min([NaN NaN]);",
        &[
            ("m", "double", (1, 1), &[NAN], &[]),
            ("k", "double", (1, 1), &[1.0], &[]),
        ],
    ),
    (
        "mx3",
        "r=min([4 2;1 3],[],2);",
        &[("r", "double", (2, 1), &[2.0, 1.0], &[])],
    ),
    (
        "mx4",
        "[m,k]=max([1 5;7 2],[],1);",
        &[
            ("m", "double", (1, 2), &[7.0, 5.0], &[]),
            ("k", "double", (1, 2), &[2.0, 1.0], &[]),
        ],
    ),
    (
        "mx5",
        "[m,k]=max([1 NaN;NaN 2],[],2);",
        &[
            ("m", "double", (2, 1), &[1.0, 2.0], &[]),
            ("k", "double", (2, 1), &[1.0, 2.0], &[]),
        ],
    ),
    (
        "mx6",
        "[m,k]=min([3 1;1 3;NaN 0]);",
        &[
            ("m", "double", (1, 2), &[1.0, 0.0], &[]),
            ("k", "double", (1, 2), &[2.0, 3.0], &[]),
        ],
    ),
    (
        "mx7",
        "[m,k]=min([4 2;1 3],[],3);",
        &[
            ("m", "double", (2, 2), &[4.0, 1.0, 2.0, 3.0], &[]),
            ("k", "double", (2, 2), &[1.0, 1.0, 1.0, 1.0], &[]),
        ],
    ),
    (
        "mx8",
        "[m,k]=min(zeros(0,3));",
        &[
            ("m", "double", (0, 3), &[], &[]),
            ("k", "double", (0, 3), &[], &[]),
        ],
    ),
    (
        "mx9",
        "[m,k]=max([]);",
        &[
            ("m", "double", (0, 0), &[], &[]),
            ("k", "double", (0, 0), &[], &[]),
        ],
    ),
    (
        "mx10",
        "[m,k]=min(zeros(3,0));",
        &[
            ("m", "double", (1, 0), &[], &[]),
            ("k", "double", (1, 0), &[], &[]),
        ],
    ),
    (
        "mx11",
        "[m,k]=min(zeros(1,0));",
        &[
            ("m", "double", (1, 0), &[], &[]),
            ("k", "double", (1, 0), &[], &[]),
        ],
    ),
    (
        "mx12",
        "[m,k]=min(zeros(0,3),[],2);",
        &[
            ("m", "double", (0, 1), &[], &[]),
            ("k", "double", (0, 1), &[], &[]),
        ],
    ),
    (
        "mx13",
        "[m,k]=max(zeros(0,3),[],1);",
        &[
            ("m", "double", (0, 3), &[], &[]),
            ("k", "double", (0, 3), &[], &[]),
        ],
    ),
    (
        "mx14",
        "[m,k]=max([],[],2);",
        &[
            ("m", "double", (0, 0), &[], &[]),
            ("k", "double", (0, 0), &[], &[]),
        ],
    ),
    (
        "mx15",
        "[m,k]=max(zeros(0,3),[],3);",
        &[
            ("m", "double", (0, 3), &[], &[]),
            ("k", "double", (0, 3), &[], &[]),
        ],
    ),
    (
        "mx16",
        "[m,k]=max([NaN NaN;1 NaN],[],2);",
        &[
            ("m", "double", (2, 1), &[NAN, 1.0], &[]),
            ("k", "double", (2, 1), &[1.0, 1.0], &[]),
        ],
    ),
    (
        "mx17",
        "r=1./[min([0 -0]) min([-0 0]) max([0 -0]) max([-0 0])];",
        &[(
            "r",
            "double",
            (1, 4),
            &[f64::INFINITY, -f64::INFINITY, f64::INFINITY, -f64::INFINITY],
            &[],
        )],
    ),
    (
        "mx18",
        "[m,k]=max([-Inf NaN Inf]);",
        &[
            ("m", "double", (1, 1), &[f64::INFINITY], &[]),
            ("k", "double", (1, 1), &[3.0], &[]),
        ],
    ),
    (
        "mx19",
        "[m,k]=max(5);",
        &[
            ("m", "double", (1, 1), &[5.0], &[]),
            ("k", "double", (1, 1), &[1.0], &[]),
        ],
    ),
    (
        "mx20",
        "[m,k]=max([2 7 7;7 1 7],[],2);",
        &[
            ("m", "double", (2, 1), &[7.0, 7.0], &[]),
            ("k", "double", (2, 1), &[2.0, 1.0], &[]),
        ],
    ),
    (
        "mx21",
        "[m,k]=min([5 3 3 1 1]);",
        &[
            ("m", "double", (1, 1), &[1.0], &[]),
            ("k", "double", (1, 1), &[4.0], &[]),
        ],
    ),
    (
        "mx24",
        "[m,k]=min([2;NaN;1],[],1);",
        &[
            ("m", "double", (1, 1), &[1.0], &[]),
            ("k", "double", (1, 1), &[3.0], &[]),
        ],
    ),
    (
        "mc1",
        "[m,k]=max([1+1i 2 -3]);",
        &[
            ("m", "double", (1, 1), &[-3.0], &[]),
            ("k", "double", (1, 1), &[3.0], &[]),
        ],
    ),
    (
        "mc2",
        "[m,k]=max([1i -1 1 -1i]);",
        &[
            ("m", "double", (1, 1), &[-1.0], &[]),
            ("k", "double", (1, 1), &[2.0], &[]),
        ],
    ),
    (
        "mc3",
        "[m,k]=min([1i -1 1 -1i]);",
        &[
            ("m", "complex", (1, 1), &[-0.0], &[-1.0]),
            ("k", "double", (1, 1), &[4.0], &[]),
        ],
    ),
    (
        "mc4",
        "[m,k]=max([1+1i 1-1i]);",
        &[
            ("m", "complex", (1, 1), &[1.0], &[1.0]),
            ("k", "double", (1, 1), &[1.0], &[]),
        ],
    ),
    (
        "mc5",
        "[m,k]=min([1+1i 1-1i]);",
        &[
            ("m", "complex", (1, 1), &[1.0], &[-1.0]),
            ("k", "double", (1, 1), &[2.0], &[]),
        ],
    ),
    (
        "mc6",
        "[m,k]=max([1+1i 1+1i 1]);",
        &[
            ("m", "complex", (1, 1), &[1.0], &[1.0]),
            ("k", "double", (1, 1), &[1.0], &[]),
        ],
    ),
    (
        "mc7",
        "[m,k]=max([NaN+1i 2 3i]);",
        &[
            ("m", "complex", (1, 1), &[0.0], &[3.0]),
            ("k", "double", (1, 1), &[3.0], &[]),
        ],
    ),
    (
        "mc8",
        "[m,k]=min([1 2i;-3 1i],[],2);",
        &[
            ("m", "complex", (2, 1), &[1.0, 0.0], &[0.0, 1.0]),
            ("k", "double", (2, 1), &[1.0, 2.0], &[]),
        ],
    ),
    (
        "mc9",
        "[m,k]=min([complex(NaN,0) complex(NaN,1)]);",
        &[
            ("m", "double", (1, 1), &[NAN], &[]),
            ("k", "double", (1, 1), &[1.0], &[]),
        ],
    ),
    (
        "mc10",
        "[m,k]=max([-2 1i;2i -1]);",
        &[
            ("m", "double", (1, 2), &[-2.0, -1.0], &[]),
            ("k", "double", (1, 2), &[1.0, 2.0], &[]),
        ],
    ),
    (
        "mc11",
        "r=min([1+1i 2],[3 1i]);",
        &[("r", "complex", (1, 2), &[1.0, 0.0], &[1.0, 1.0])],
    ),
    (
        "mc12",
        "r=max(2i,[1 -3 2]);",
        &[("r", "complex", (1, 3), &[0.0, -3.0, 0.0], &[2.0, 0.0, 2.0])],
    ),
    (
        "mc13",
        "r=max([1 NaN],[NaN 1i]);",
        &[("r", "complex", (1, 2), &[1.0, 0.0], &[0.0, 1.0])],
    ),
    (
        "mc14",
        "[m,k]=max([-2 2i 2]);",
        &[
            ("m", "double", (1, 1), &[-2.0], &[]),
            ("k", "double", (1, 1), &[1.0], &[]),
        ],
    ),
    (
        "mc15",
        "[m,k]=min([-2 2i 2]);",
        &[
            ("m", "double", (1, 1), &[2.0], &[]),
            ("k", "double", (1, 1), &[3.0], &[]),
        ],
    ),
    (
        "mc16",
        "[m,k]=max([1 2i],[],3);",
        &[
            ("m", "complex", (1, 2), &[1.0, 0.0], &[0.0, 2.0]),
            ("k", "double", (1, 2), &[1.0, 1.0], &[]),
        ],
    ),
    (
        "mc17",
        "[m,k]=min(complex([3 1 2],0));",
        &[
            ("m", "double", (1, 1), &[1.0], &[]),
            ("k", "double", (1, 1), &[2.0], &[]),
        ],
    ),
    (
        "mb1",
        "r=min([1 5;NaN 2],[3 NaN;NaN 1]);",
        &[("r", "double", (2, 2), &[1.0, NAN, 5.0, 1.0], &[])],
    ),
    (
        "mb2",
        "r=max(3,[1 5 NaN]);",
        &[("r", "double", (1, 3), &[3.0, 5.0, 3.0], &[])],
    ),
    ("mb4", "r=min([],5);", &[("r", "double", (0, 0), &[], &[])]),
    (
        "mb6",
        "r=max('ab','b');",
        &[("r", "double", (1, 2), &[98.0, 98.0], &[])],
    ),
    (
        "mb7",
        "r=max(true,false);",
        &[("r", "logical", (1, 1), &[1.0], &[])],
    ),
    (
        "ml1",
        "[m,k]=max([true false true]);",
        &[
            ("m", "logical", (1, 1), &[1.0], &[]),
            ("k", "double", (1, 1), &[1.0], &[]),
        ],
    ),
    (
        "ml2",
        "[m,k]=min('hello');",
        &[
            ("m", "double", (1, 1), &[101.0], &[]),
            ("k", "double", (1, 1), &[2.0], &[]),
        ],
    ),
    (
        "ml3",
        "r=max([true false;false false],[],2);",
        &[("r", "logical", (2, 1), &[1.0, 0.0], &[])],
    ),
    (
        "ml4",
        "r=min('ab',[],1);",
        &[("r", "double", (1, 2), &[97.0, 98.0], &[])],
    ),
    (
        "sm1",
        "r=sum([1 2;3 4],2);",
        &[("r", "double", (2, 1), &[3.0, 7.0], &[])],
    ),
    (
        "sm2",
        "r=mean([1 2;3 4],2);",
        &[("r", "double", (2, 1), &[1.5, 3.5], &[])],
    ),
    (
        "sm3",
        "r=sum([1 2;3 4],3);",
        &[("r", "double", (2, 2), &[1.0, 3.0, 2.0, 4.0], &[])],
    ),
    (
        "sm4",
        "r=mean([1 2;3 4],3);",
        &[("r", "double", (2, 2), &[1.0, 3.0, 2.0, 4.0], &[])],
    ),
    (
        "sm5",
        "r=sum(zeros(0,3),1);",
        &[("r", "double", (1, 3), &[0.0, 0.0, 0.0], &[])],
    ),
    (
        "sm6",
        "r=sum(zeros(0,3),2);",
        &[("r", "double", (0, 1), &[], &[])],
    ),
    (
        "sm7",
        "r=mean(zeros(0,3));",
        &[("r", "double", (1, 3), &[NAN, NAN, NAN], &[])],
    ),
    (
        "sm8",
        "r=mean(zeros(0,3),2);",
        &[("r", "double", (0, 1), &[], &[])],
    ),
    ("sm9", "r=sum([],1);", &[("r", "double", (1, 0), &[], &[])]),
    ("sm10", "r=sum([],2);", &[("r", "double", (0, 1), &[], &[])]),
    (
        "sm11",
        "r=mean([],1);",
        &[("r", "double", (1, 0), &[], &[])],
    ),
    (
        "sm12",
        "r=sum(zeros(3,0));",
        &[("r", "double", (1, 0), &[], &[])],
    ),
    (
        "sm13",
        "r=sum(zeros(1,0));",
        &[("r", "double", (1, 1), &[0.0], &[])],
    ),
    (
        "sm14",
        "r=mean(zeros(1,0));",
        &[("r", "double", (1, 1), &[NAN], &[])],
    ),
    (
        "sm15",
        "r=sum([true true],2);",
        &[("r", "double", (1, 1), &[2.0], &[])],
    ),
    (
        "sm16",
        "r=sum('ab');",
        &[("r", "double", (1, 1), &[195.0], &[])],
    ),
    (
        "sm17",
        "r=mean('ab',2);",
        &[("r", "double", (1, 1), &[97.5], &[])],
    ),
    (
        "sm18",
        "r=sum([1 NaN;2 3],2);",
        &[("r", "double", (2, 1), &[NAN, 5.0], &[])],
    ),
    (
        "sm20",
        "r=mean([1 2 3],1);",
        &[("r", "double", (1, 3), &[1.0, 2.0, 3.0], &[])],
    ),
    ("sm21", "r=sum([],3);", &[("r", "double", (0, 0), &[], &[])]),
    (
        "sm22",
        "r=mean([true false true],2);",
        &[("r", "double", (1, 1), &[0.6666666666666666], &[])],
    ),
    (
        "sm23",
        "r=mean([],3);",
        &[("r", "double", (0, 0), &[], &[])],
    ),
    (
        "sm24",
        "r=sum(zeros(0,3),3);",
        &[("r", "double", (0, 3), &[], &[])],
    ),
    (
        "sm25",
        "r=mean([],2);",
        &[("r", "double", (0, 1), &[], &[])],
    ),
    (
        "sm27",
        "r=mean(zeros(3,0),1);",
        &[("r", "double", (1, 0), &[], &[])],
    ),
    (
        "an1",
        "r=any([0 0;0 1],2);",
        &[("r", "logical", (2, 1), &[0.0, 1.0], &[])],
    ),
    (
        "an2",
        "r=any([0 NaN]);",
        &[("r", "logical", (1, 1), &[0.0], &[])],
    ),
    (
        "an3",
        "r=all([NaN 1]);",
        &[("r", "logical", (1, 1), &[1.0], &[])],
    ),
    (
        "an4",
        "r=all([1 1;0 1],2);",
        &[("r", "logical", (2, 1), &[1.0, 0.0], &[])],
    ),
    (
        "an5",
        "r=any(zeros(0,3));",
        &[("r", "logical", (1, 3), &[0.0, 0.0, 0.0], &[])],
    ),
    (
        "an6",
        "r=any(zeros(0,3),2);",
        &[("r", "logical", (0, 1), &[], &[])],
    ),
    (
        "an7",
        "r=all(zeros(0,3),2);",
        &[("r", "logical", (0, 1), &[], &[])],
    ),
    ("an8", "r=any([],1);", &[("r", "logical", (1, 0), &[], &[])]),
    ("an9", "r=all([],2);", &[("r", "logical", (0, 1), &[], &[])]),
    (
        "an10",
        "r=any(zeros(3,0));",
        &[("r", "logical", (1, 0), &[], &[])],
    ),
    (
        "an11",
        "r=all(zeros(1,0));",
        &[("r", "logical", (1, 1), &[1.0], &[])],
    ),
    (
        "an12",
        "r=any(zeros(1,0));",
        &[("r", "logical", (1, 1), &[0.0], &[])],
    ),
    (
        "an13",
        "r=any([1 0;0 0],3);",
        &[("r", "logical", (2, 2), &[1.0, 0.0, 0.0, 0.0], &[])],
    ),
    (
        "an14",
        "r=all([1 NaN;0 2],3);",
        &[("r", "logical", (2, 2), &[1.0, 0.0, 1.0, 1.0], &[])],
    ),
    (
        "an15",
        "r=any('a b');",
        &[("r", "logical", (1, 1), &[1.0], &[])],
    ),
    (
        "an16",
        "r=any([0 1i]);",
        &[("r", "logical", (1, 1), &[1.0], &[])],
    ),
    (
        "an17",
        "r=all([1 1i 0],2);",
        &[("r", "logical", (1, 1), &[0.0], &[])],
    ),
    (
        "an18",
        "r=any(NaN);",
        &[("r", "logical", (1, 1), &[0.0], &[])],
    ),
    (
        "an19",
        "r=all([],1);",
        &[("r", "logical", (1, 0), &[], &[])],
    ),
    (
        "an20",
        "r=any([0 0 1]);",
        &[("r", "logical", (1, 1), &[1.0], &[])],
    ),
    (
        "an21",
        "r=all([1 2;3 0]);",
        &[("r", "logical", (1, 2), &[1.0, 0.0], &[])],
    ),
    (
        "an22",
        "r=any([],3);",
        &[("r", "logical", (0, 0), &[], &[])],
    ),
    (
        "an23",
        "r=any([0 0;0 0],1);",
        &[("r", "logical", (1, 2), &[0.0, 0.0], &[])],
    ),
    (
        "an24",
        "r=all(zeros(3,0),2);",
        &[("r", "logical", (3, 1), &[1.0, 1.0, 1.0], &[])],
    ),
    (
        "fd1",
        "r=find([0 1 1 0 1],2);",
        &[("r", "double", (1, 2), &[2.0, 3.0], &[])],
    ),
    (
        "fd2",
        "r=find([0 1 1 0 1],2,'last');",
        &[("r", "double", (1, 2), &[3.0, 5.0], &[])],
    ),
    (
        "fd3",
        "r=find([0 1;1 1],2,'last');",
        &[("r", "double", (2, 1), &[3.0, 4.0], &[])],
    ),
    (
        "fd4",
        "[r,c]=find([0 1;1 1]);",
        &[
            ("r", "double", (3, 1), &[2.0, 1.0, 2.0], &[]),
            ("c", "double", (3, 1), &[1.0, 2.0, 2.0], &[]),
        ],
    ),
    (
        "fd5",
        "[r,c,v]=find([0 5;7 0]);",
        &[
            ("r", "double", (2, 1), &[2.0, 1.0], &[]),
            ("c", "double", (2, 1), &[1.0, 2.0], &[]),
            ("v", "double", (2, 1), &[7.0, 5.0], &[]),
        ],
    ),
    (
        "fd6",
        "[r,c,v]=find([0 2 0 3]);",
        &[
            ("r", "double", (1, 2), &[1.0, 1.0], &[]),
            ("c", "double", (1, 2), &[2.0, 4.0], &[]),
            ("v", "double", (1, 2), &[2.0, 3.0], &[]),
        ],
    ),
    (
        "fd7",
        "r=find([0 0]);",
        &[("r", "double", (1, 0), &[], &[])],
    ),
    (
        "fd8",
        "r=find([0;0]);",
        &[("r", "double", (0, 1), &[], &[])],
    ),
    (
        "fd9",
        "r=find(zeros(2,2));",
        &[("r", "double", (0, 1), &[], &[])],
    ),
    ("fd10", "r=find([]);", &[("r", "double", (0, 0), &[], &[])]),
    (
        "fd11",
        "r=find(zeros(1,0));",
        &[("r", "double", (1, 0), &[], &[])],
    ),
    (
        "fd12",
        "r=find(zeros(0,3));",
        &[("r", "double", (0, 1), &[], &[])],
    ),
    (
        "fd13",
        "r=find(zeros(3,0));",
        &[("r", "double", (0, 1), &[], &[])],
    ),
    (
        "fd14",
        "[r,c]=find([]);",
        &[
            ("r", "double", (0, 0), &[], &[]),
            ("c", "double", (0, 0), &[], &[]),
        ],
    ),
    (
        "fd15",
        "[r,c]=find([0 0]);",
        &[
            ("r", "double", (1, 0), &[], &[]),
            ("c", "double", (1, 0), &[], &[]),
        ],
    ),
    (
        "fd16",
        "r=find(5);",
        &[("r", "double", (1, 1), &[1.0], &[])],
    ),
    ("fd17", "r=find(0);", &[("r", "double", (0, 0), &[], &[])]),
    (
        "fd19",
        "[r,c,v]=find([true false true]);",
        &[
            ("r", "double", (1, 2), &[1.0, 1.0], &[]),
            ("c", "double", (1, 2), &[1.0, 3.0], &[]),
            ("v", "logical", (1, 2), &[1.0, 1.0], &[]),
        ],
    ),
    (
        "fd20",
        "[r,c,v]=find('a b');",
        &[
            ("r", "double", (1, 3), &[1.0, 1.0, 1.0], &[]),
            ("c", "double", (1, 3), &[1.0, 2.0, 3.0], &[]),
            ("v", "double", (1, 3), &[97.0, 32.0, 98.0], &[]),
        ],
    ),
    (
        "fd21",
        "r=find([0 3 NaN]);",
        &[("r", "double", (1, 2), &[2.0, 3.0], &[])],
    ),
    (
        "fd22",
        "[r,c,v]=find([0 1i 2]);",
        &[
            ("r", "double", (1, 2), &[1.0, 1.0], &[]),
            ("c", "double", (1, 2), &[2.0, 3.0], &[]),
            ("v", "complex", (1, 2), &[0.0, 2.0], &[1.0, 0.0]),
        ],
    ),
    (
        "fd23",
        "r=find([0 1 1 0 1],10,'last');",
        &[("r", "double", (1, 3), &[2.0, 3.0, 5.0], &[])],
    ),
    (
        "fd24",
        "r=find([1 1 1],2,'first');",
        &[("r", "double", (1, 2), &[1.0, 2.0], &[])],
    ),
    (
        "fd25",
        "[r,c]=find([0 1 0 1;1 0 0 1],2,'last');",
        &[
            ("r", "double", (2, 1), &[1.0, 2.0], &[]),
            ("c", "double", (2, 1), &[4.0, 4.0], &[]),
        ],
    ),
    (
        "fd26",
        "[r,c,v]=find([0 4 0;5 0 6],2);",
        &[
            ("r", "double", (2, 1), &[2.0, 1.0], &[]),
            ("c", "double", (2, 1), &[1.0, 2.0], &[]),
            ("v", "double", (2, 1), &[5.0, 4.0], &[]),
        ],
    ),
    (
        "fd29",
        "[r,c]=find(zeros(0,3));",
        &[
            ("r", "double", (0, 1), &[], &[]),
            ("c", "double", (0, 1), &[], &[]),
        ],
    ),
    (
        "fd30",
        "[r,c,v]=find(zeros(1,0));",
        &[
            ("r", "double", (1, 0), &[], &[]),
            ("c", "double", (1, 0), &[], &[]),
            ("v", "double", (1, 0), &[], &[]),
        ],
    ),
    (
        "fd31",
        "r=find([0;1;1],1,'last');",
        &[("r", "double", (1, 1), &[3.0], &[])],
    ),
    ("fd32", "r=find(0,1);", &[("r", "double", (0, 0), &[], &[])]),
    (
        "fd33",
        "r=find([0 0],1);",
        &[("r", "double", (1, 0), &[], &[])],
    ),
    (
        "fd34",
        "r=find(zeros(2,2),1);",
        &[("r", "double", (0, 1), &[], &[])],
    ),
    (
        "fd35",
        "[r,c,v]=find([0 0;0 0]);",
        &[
            ("r", "double", (0, 1), &[], &[]),
            ("c", "double", (0, 1), &[], &[]),
            ("v", "double", (0, 1), &[], &[]),
        ],
    ),
    (
        "fd36",
        "[r,c,v]=find([0 0]);",
        &[
            ("r", "double", (1, 0), &[], &[]),
            ("c", "double", (1, 0), &[], &[]),
            ("v", "double", (1, 0), &[], &[]),
        ],
    ),
];

/// Reference output of the fix-round-2 batch: signed-zero ties in min/max(A,B), stored char
/// arrays (1x0 and non-row shapes), and any/all of complex values with a NaN part.
const REFERENCE_REV2: &[Case] = &[
    (
        "b1",
        "r=1./[min(0,-0) min(-0,0) max(0,-0) max(-0,0)];",
        &[("r", "double", (1, 4), &[INF, -INF, INF, -INF], &[])],
    ),
    (
        "b2",
        "r=1./[min([0 -0],[-0 0]) max([0 -0],[-0 0])];",
        &[("r", "double", (1, 4), &[INF, -INF, INF, -INF], &[])],
    ),
    (
        "c1",
        "e=char(zeros(1,0)); r=size(min(e));",
        &[("r", "double", (1, 2), &[1., 0.], &[])],
    ),
    (
        "c2",
        "e=char(zeros(1,0)); r=size(find(e));",
        &[("r", "double", (1, 2), &[1., 0.], &[])],
    ),
    (
        "c3",
        "e=char(zeros(1,0)); r=size(sum(e));",
        &[("r", "double", (1, 2), &[1., 1.], &[])],
    ),
    (
        "c4",
        "e=char(zeros(1,0)); r=size(any(e));",
        &[("r", "double", (1, 2), &[1., 1.], &[])],
    ),
    (
        "c5",
        "[m,i]=min(char([65;66]));",
        &[
            ("m", "double", (1, 1), &[65.], &[]),
            ("i", "double", (1, 1), &[1.], &[]),
        ],
    ),
    // The reference case uses max(['ab';'cd']); MatRust does not concatenate character rows vertically yet,
    // so the same stored 2x2 array comes from char().
    (
        "c6",
        "m=max(char([97 98;99 100]));",
        &[("m", "double", (1, 2), &[99., 100.], &[])],
    ),
    (
        "n1",
        "r=any(complex(NaN,1));",
        &[("r", "logical", (1, 1), &[1.], &[])],
    ),
    (
        "n2",
        "r=any(complex(1,NaN));",
        &[("r", "logical", (1, 1), &[1.], &[])],
    ),
    (
        "n3",
        "r=all(complex(NaN,1));",
        &[("r", "logical", (1, 1), &[1.], &[])],
    ),
    (
        "n4",
        "r=any([complex(NaN,1) 0]);",
        &[("r", "logical", (1, 1), &[1.], &[])],
    ),
];

const REFERENCE_ERRORS: &[(&str, &str)] = &[
    ("mx22", "r=max([1 2;3 4],[],0);"), // Dimension argument must be a positive integer scalar within indexing range.
    ("mx23", "r=max([1 2;3 4],5,2);"), // MAX with two matrices to compare and a working dimension is not supported.
    ("mb3", "r=min([1 2 3],[2;1]);"),  // Matrix dimensions must agree.
    ("mb5", "[m,k]=min([1 2],3);"), // MIN with two matrices to compare and two output arguments is not supported.
    ("sm19", "r=sum([1 2;3 4],0);"), // Dimension argument must be a positive integer scalar within indexing range.
    ("sm26", "r=sum([1 2;3 4],1.5);"), // Dimension argument must be a positive integer scalar within indexing range.
    ("an25", "r=any([0 1],0);"), // Dimension argument must be a positive integer scalar within indexing range.
    ("fd18", "r=find([1 1],0);"), // Second argument must be a positive scalar integer.
    ("fd27", "r=find([0 1 1],1.5);"), // Second argument must be a positive scalar integer.
    ("fd28", "r=find([0 1 1],2,'middle');"), // Invalid search option. Must be 'first' or 'last'.
    ("fd37", "r=find([0 1 1],'last');"), // Second argument must be a positive scalar integer.
];

/// Not in the reference batch: they follow from the reference rows (mx1 first index, an1 logical input,
/// fd2 and an16 for complex find/any, mb7 for logical max, c5/c6 for stored char codes; c7/c8
/// hold an unpaired UTF-16 surrogate, fd40 a stored direction) and test the `~` output placeholder.
const NOT_IN_REFERENCE: &[Case] = &[
    (
        "mx25",
        "[~,k]=max([1 9 9]);",
        &[("k", "double", (1, 1), &[2.], &[])],
    ),
    (
        "an26",
        "r=any(logical([0 1;0 0]),2);",
        &[("r", "logical", (2, 1), &[1., 0.], &[])],
    ),
    (
        "fd38",
        "[~,c]=find([0 1;1 0]);",
        &[("c", "double", (2, 1), &[1., 2.], &[])],
    ),
    (
        "fd39",
        "r=find([0 1i 2 0 3],2,'last');",
        &[("r", "double", (1, 2), &[3., 5.], &[])],
    ),
    (
        "an27",
        "r=any([0 0;0 1i],2);",
        &[("r", "logical", (2, 1), &[0., 1.], &[])],
    ),
    (
        "mb8",
        "r=max(true,2);",
        &[("r", "double", (1, 1), &[2.], &[])],
    ),
    (
        "c7",
        "[m,i]=max(char([65 55296 66]));",
        &[
            ("m", "double", (1, 1), &[55296.], &[]),
            ("i", "double", (1, 1), &[2.], &[]),
        ],
    ),
    (
        "c8",
        "r=find(char([0 55296;66 0]));",
        &[("r", "double", (2, 1), &[2., 3.], &[])],
    ),
    (
        "fd40",
        "d='last'; r=find([0 1 1],1,d);",
        &[("r", "double", (1, 1), &[3.], &[])],
    ),
];

/// Invalid in every MATLAB release (too many outputs) or only valid from R2018 on (flags).
const ERRORS: &[&str] = &[
    "[a,b,c]=max([1 2]);",
    "[a,b]=sum([1 2]);",
    "[a,b,c,d]=find([1 2]);",
    "r=sum([1 2],1,'omitnan');",
    "r=max([1 2],[],2,'omitnan');",
];

fn class(array: &Array) -> &'static str {
    match array {
        Array::Numeric(_) => "double",
        Array::Logical(_) => "logical",
        Array::Complex(_) => "complex",
        Array::Character(_) => "char",
        _ => "other",
    }
}

fn parts(array: &Array) -> (Vec<f64>, Vec<f64>) {
    match array {
        Array::Numeric(a) => (a.data().to_vec(), Vec::new()),
        Array::Logical(a) => (a.data().iter().map(|&v| f64::from(v)).collect(), Vec::new()),
        Array::Complex(a) => a.data().iter().map(|z| (z.re, z.im)).unzip(),
        Array::Character(a) => (
            a.units().iter().map(|&u| f64::from(u)).collect(),
            Vec::new(),
        ),
        _ => panic!("array result"),
    }
}

fn same(got: &[f64], want: &[f64]) -> bool {
    got.len() == want.len()
        && got
            .iter()
            .zip(want)
            .all(|(g, w)| g == w || (g.is_nan() && w.is_nan()))
}

/// Runs every case and panics once with all mismatches, so one run shows every difference.
fn verify(cases: &[Case]) -> usize {
    let mut failures = Vec::new();
    for &(id, source, expected) in cases {
        let mut engine = Engine::default();
        if let Err(e) = engine.execute(source) {
            failures.push(format!("{id} `{source}`: {e}"));
            continue;
        }
        for &(name, want_class, shape, re, im) in expected {
            let Some(array) = engine.array(name) else {
                failures.push(format!("{id} `{source}`: no {name}"));
                continue;
            };
            let (got_re, got_im) = parts(&array);
            let got_shape = (array.rows(), array.cols());
            let got_class = class(&array);
            let same_im = want_class != "complex" || same(&got_im, im);
            if got_class != want_class || got_shape != shape || !same(&got_re, re) || !same_im {
                failures.push(format!(
                    "{id} `{source}` {name}: got {got_class} {got_shape:?} {got_re:?} {got_im:?}, \
                     want {want_class} {shape:?} {re:?} {im:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    cases.len()
}

#[test]
fn recorded_reference_values_match() {
    assert_eq!(verify(REFERENCE_VALUES), 18);
}

#[test]
fn reference_values_match_nan_empty_complex_dim3_and_index_outputs() {
    assert_eq!(verify(REFERENCE), REFERENCE.len());
    assert_eq!((REFERENCE.len(), REFERENCE_ERRORS.len()), (130, 11));
}

#[test]
fn fix_round_two_reference_values_match() {
    assert_eq!(verify(REFERENCE_REV2), 12);
}

#[test]
fn forms_outside_the_reference_batch() {
    verify(NOT_IN_REFERENCE);
}

#[test]
fn invalid_forms_error_without_assigning() {
    let statements = REFERENCE_ERRORS
        .iter()
        .map(|&(_, s)| s)
        .chain(ERRORS.iter().copied());
    for source in statements {
        let mut engine = Engine::default();
        engine.execute("r=7;").unwrap();
        assert!(engine.execute(source).is_err(), "accepted {source}");
        assert_eq!(engine.variables["r"].data(), &[7.], "{source}");
    }
}

#[test]
fn signed_zero_keeps_the_first_extreme() {
    let mut engine = Engine::default();
    engine
        .execute("r=1./[min([0 -0]) min([-0 0]) max([0 -0]) max([-0 0])];")
        .unwrap();
    let inf = f64::INFINITY;
    assert_eq!(engine.variables["r"].data(), &[inf, -inf, inf, -inf]);
}

#[test]
fn dimension_above_two_shares_storage() {
    let cancel = AtomicBool::new(false);
    let a = Matrix::from_column_slice(2, 2, &[1., NAN, -0., 4.]).unwrap();
    for mean in [false, true] {
        let r = a.sum_along(Some(3), mean, &cancel).unwrap();
        assert_eq!(r.data().as_ptr(), a.data().as_ptr());
    }
    let (m, i) = a
        .extreme_along(Extreme::Max, Some(7), true, &cancel)
        .unwrap();
    assert_eq!(m.data().as_ptr(), a.data().as_ptr());
    assert_eq!(i.unwrap().data(), &[1.; 4]);
    let l = LogicalMatrix::from_column_vec(1, 2, vec![0, 1], &cancel).unwrap();
    let t = matrust::matrix::truth_along(
        matrust::logical::ArrayRef::Logical(&l),
        true,
        Some(3),
        &cancel,
    )
    .unwrap();
    assert_eq!(t.data().as_ptr(), l.data().as_ptr());
}

/// Deterministic data with NaN, ties, signed zeros and negative values.
fn sample(rows: usize, cols: usize) -> Matrix {
    let data = (0..rows * cols)
        .map(|i| match i % 97 {
            0 => NAN,
            1 => -0.,
            2 => 0.,
            _ => ((i * 7919) % 1013) as f64 - 500.,
        })
        .collect();
    Matrix::from_column_vec(rows, cols, data).unwrap()
}

#[test]
fn every_worker_count_gives_identical_bits_above_the_parallel_threshold() {
    let cancel = AtomicBool::new(false);
    let pools = [1, 8].map(|n| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .unwrap()
    });
    for (rows, cols) in [
        (700, 700),
        (3, 200_000),
        (200_000, 3),
        (1, 300_000),
        (300_000, 1),
    ] {
        let a = sample(rows, cols);
        let l = LogicalMatrix::from_column_vec(
            rows,
            cols,
            a.data().iter().map(|&x| u8::from(x > 0.)).collect(),
            &cancel,
        )
        .unwrap();
        let run = |pool: &rayon::ThreadPool| {
            pool.install(|| {
                let mut bits = Vec::new();
                for dim in [None, Some(1), Some(2)] {
                    for mean in [false, true] {
                        bits.extend(a.sum_along(dim, mean, &cancel).unwrap().data().to_vec());
                    }
                    for kind in [Extreme::Min, Extreme::Max] {
                        let (m, i) = a.extreme_along(kind, dim, true, &cancel).unwrap();
                        bits.extend(m.data().iter().chain(i.unwrap().data()).copied());
                    }
                    for all in [false, true] {
                        for view in [
                            matrust::logical::ArrayRef::Numeric(&a),
                            matrust::logical::ArrayRef::Logical(&l),
                        ] {
                            let t = matrust::matrix::truth_along(view, all, dim, &cancel).unwrap();
                            bits.extend(t.data().iter().map(|&v| f64::from(v)));
                        }
                    }
                }
                for (limit, last) in [(None, false), (Some(5), false), (Some(5), true)] {
                    let found = matrust::matrix::find_nonzero(
                        matrust::logical::ArrayRef::Numeric(&a),
                        limit,
                        last,
                        &cancel,
                    )
                    .unwrap();
                    bits.extend(found.iter().map(|&i| i as f64));
                }
                bits.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            })
        };
        assert_eq!(run(&pools[0]), run(&pools[1]), "{rows}x{cols}");
    }
}

#[test]
fn serial_reference_agrees_with_kernels() {
    let cancel = AtomicBool::new(false);
    let (rows, cols) = (37, 23);
    let a = sample(rows, cols);
    let at = |r: usize, c: usize| a.data()[c * rows + r];
    let (m, i) = a
        .extreme_along(Extreme::Min, Some(2), true, &cancel)
        .unwrap();
    let s = a.sum_along(Some(2), false, &cancel).unwrap();
    let i = i.unwrap();
    for r in 0..rows {
        let lane: Vec<f64> = (0..cols).map(|c| at(r, c)).collect();
        let mut best = (NAN, 0);
        for (k, &x) in lane.iter().enumerate() {
            if !x.is_nan() && (best.0.is_nan() || x < best.0) {
                best = (x, k);
            }
        }
        assert_eq!(m.data()[r].to_bits(), best.0.to_bits(), "row {r}");
        assert_eq!(i.data()[r], (best.1 + 1) as f64, "row {r}");
        let sum = lane
            .iter()
            .fold(None, |acc: Option<f64>, &x| Some(acc.map_or(x, |s| s + x)));
        assert_eq!(s.data()[r].to_bits(), sum.unwrap().to_bits(), "row {r}");
    }
}

#[test]
fn cancellation_stops_every_kernel() {
    let cancel = AtomicBool::new(true);
    let a = sample(300, 300);
    assert!(a.sum_along(Some(2), false, &cancel).is_err());
    assert!(
        a.extreme_along(Extreme::Max, Some(1), true, &cancel)
            .is_err()
    );
    assert!(
        matrust::matrix::truth_along(
            matrust::logical::ArrayRef::Numeric(&a),
            false,
            None,
            &cancel
        )
        .is_err()
    );
    assert!(
        matrust::matrix::find_nonzero(
            matrust::logical::ArrayRef::Numeric(&a),
            None,
            false,
            &cancel
        )
        .is_err()
    );
    let mut engine = Engine::default();
    engine.execute("A=ones(4);").unwrap();
    engine.cancellation().store(true, Ordering::Relaxed);
    for source in [
        "r=max(A,[],2);",
        "[r,c]=find(A);",
        "r=any(A,2);",
        "r=mean(A,2);",
    ] {
        assert!(engine.execute(source).is_err(), "{source}");
    }
}
