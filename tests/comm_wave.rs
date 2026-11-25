//! Hard-coded expectations from tool/refs/comm/wave.py (NumPy/SciPy).
use matrust::engine::Limits;
use matrust::{Array, Engine};
use std::sync::atomic::Ordering;
#[allow(clippy::type_complexity, clippy::excessive_precision, clippy::approx_constant)]
#[rustfmt::skip]
mod reference { include!("fixtures/comm/wave.rs"); }
use reference::*;
fn run(e: &mut Engine, s: &str) {
    e.execute(s).unwrap_or_else(|err| panic!("{s}\n{err}"));
}
fn fails(e: &mut Engine, s: &str, part: &str) {
    let err = e.execute(s).unwrap_err();
    assert!(err.contains(part), "{s}: {err}");
}
fn real(e: &Engine, n: &str) -> Vec<f64> {
    match e.array(n).unwrap() {
        Array::Numeric(m) => m.data().to_vec(),
        a => panic!("{n}: {a:?}"),
    }
}
fn literal(v: &[f64]) -> String {
    format!(
        "[{}]",
        v.iter()
            .map(|v| format!("{v:.17e}"))
            .collect::<Vec<_>>()
            .join(" ")
    )
}
fn complex_literal(v: &[(f64, f64)]) -> String {
    format!(
        "[{}]",
        v.iter()
            .map(|(r, i)| format!("({r:.17e}+({i:.17e})*1i)"))
            .collect::<Vec<_>>()
            .join(" ")
    )
}
fn near(e: &Engine, n: &str, want: &[f64], tol: f64) {
    let got = real(e, n);
    assert_eq!(got.len(), want.len(), "{n}");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() <= tol, "{n}[{i}]: {g} != {w}");
    }
}
fn near_complex(e: &Engine, n: &str, want: &[(f64, f64)]) {
    let Array::Complex(m) = e.array(n).unwrap() else {
        panic!("{n}: complex expected")
    };
    assert_eq!(m.len(), want.len(), "{n}");
    for (i, (g, (r, im))) in m.data().iter().zip(want).enumerate() {
        assert!(
            (g.re - r).abs().max((g.im - im).abs()) < 1e-12,
            "{n}[{i}]: {g} != ({r},{im})"
        );
    }
}
#[test]
fn rcosdesign_numpy_all_shapes_rolloffs_lengths_singularities_energy() {
    let mut e = Engine::default();
    for &(beta, span, sps, shape, expected) in PULSES {
        run(
            &mut e,
            &format!(
                "b=rcosdesign({beta},{span},{sps},'{shape}');assert(isequal(size(b),[1 {}]));assert(abs(sum(b.^2)-1)<2e-14);assert(max(abs(b-fliplr(b)))<1e-14);",
                span * sps + 1
            ),
        );
        near(&e, "b", expected, 1e-12);
    }
    run(
        &mut e,
        "assert(isequal(rcosdesign(.25,6,4),rcosdesign(.25,6,4,'sqrt')));",
    );
    for (s, part) in [
        ("rcosdesign(-.1,6,4)", "beta"),
        ("rcosdesign(1.1,6,4)", "beta"),
        ("rcosdesign(.5,3,3)", "even"),
        ("rcosdesign(.5,0,4)", "integer"),
        ("rcosdesign(.5,4,2.5)", "integer"),
        ("rcosdesign(.5,4,2,'bad')", "shape"),
        ("rcosdesign(NaN,4,2)", "finite"),
        ("rcosdesign(.5,8388608,8388608)", "limit"),
    ] {
        fails(&mut e, s, part);
    }
}
#[test]
fn rcosdesign_near_singular_points_mpmath() {
    let mut e = Engine::default();
    for &(beta, span, sps, shape, expected) in PULSES_NEAR_SINGULAR {
        run(
            &mut e,
            &format!(
                "b=rcosdesign({beta:.17e},{span},{sps},'{shape}');assert(abs(sum(b.^2)-1)<2e-14);assert(max(abs(b-fliplr(b)))<1e-14);"
            ),
        );
        near(&e, "b", expected, 1e-12);
    }
    // Continuous across beta = 0.25 and beta = 0.5.
    run(
        &mut e,
        "b0=rcosdesign(0.25,6,4,'sqrt');b1=rcosdesign(0.25000000000000006,6,4,'sqrt');assert(max(abs(b1-b0))<1e-14);b2=rcosdesign(0.5,6,4,'sqrt');b3=rcosdesign(0.5+1e-12,6,4,'sqrt');assert(max(abs(b3-b2))<1e-11);",
    );
}
#[test]
fn fsk_numpy_continuous_independent_oscillators_and_gray_labels() {
    let mut e = Engine::default();
    run(&mut e, "x=[0 1 2 3 4 5 6 7 7 0 5 2];");
    for (cont, order, expected) in [
        ("cont", "bin", FSK_CONT_BIN),
        ("discont", "bin", FSK_DISCONT_BIN),
        ("cont", "gray", FSK_CONT_GRAY),
        ("discont", "gray", FSK_DISCONT_GRAY),
    ] {
        run(
            &mut e,
            &format!(
                "y=fskmod(x,8,.7,3,9,'{cont}','{order}');z=fskdemod(y,8,.7,3,9,'{order}');assert(isequal(size(y),[1 36]));"
            ),
        );
        near_complex(&e, "y", expected);
        // The short waveform intentionally has nonorthogonal tones; use long
        // orthogonal tones below to test error-free correlation decisions.
    }
    for cont in ["cont", "discont"] {
        for order in ["bin", "gray"] {
            run(
                &mut e,
                &format!(
                    "x=reshape(mod(0:95,8),24,4);y=fskmod(x,8,1,16,16,'{cont}','{order}');assert(isequal(size(y),[384 4]));assert(isequal(fskdemod(y,8,1,16,16,'{order}'),x));"
                ),
            );
        }
    }
    run(
        &mut e,
        "x=[0 1 0 1];assert(isequal(fskmod(x,2,.25,4),fskmod(x,2,.25,4,1,'cont','bin')));assert(isequal(fskdemod(fskmod(x,2,.25,4),2,.25,4),x));",
    );
}
#[test]
fn dpsk_numpy_rotation_columns_row_and_gray_round_trips() {
    let mut e = Engine::default();
    for (order, expected) in [("bin", DPSK_BIN), ("gray", DPSK_GRAY)] {
        run(
            &mut e,
            &format!(
                "x=[0 1 2 3 4 5 6 7 7 0 5 2];y=dpskmod(x,8,pi/7,'{order}');assert(isequal(dpskdemod(y,8,pi/7,'{order}'),x));"
            ),
        );
        near_complex(&e, "y", expected);
        run(
            &mut e,
            &format!(
                "x=reshape(mod(0:95,8),24,4);y=dpskmod(x,8,pi/7,'{order}');z=dpskdemod(y,8,pi/7,'{order}');assert(isequal(z,x));z=dpskdemod(y*exp(1i*.7),8,pi/7,'{order}');assert(isequal(z(2:end,:),x(2:end,:)));"
            ),
        );
    }
    run(
        &mut e,
        "assert(isequal(dpskmod([0 1],2,[]),dpskmod([0 1],2)));assert(isequal(dpskdemod(dpskmod([0 1],2),2,[]),[0 1]));",
    );
}
#[test]
fn msk_numpy_both_encodings_phase_state_and_block_continuation() {
    let mut e = Engine::default();
    for (mode, expected) in [("diff", MSK_DIFF), ("nondiff", MSK_NONDIFF)] {
        run(
            &mut e,
            &format!(
                "b=[0;1;1;0;0;1;0];[y,p]=mskmod(b,4,'{mode}');[z,q,state]=mskdemod(y,4,'{mode}');assert(isequal(z,b));assert(p==q);assert(isequal(size(state),[4 1]));assert(isequal(state,y(end-3:end)));"
            ),
        );
        near_complex(&e, "y", expected);
        // Odd-size blocks exercise I/Q-axis parity across calls.
        run(
            &mut e,
            &format!(
                "[a,p]=mskmod(b(1:3),4,'{mode}');[c,p2]=mskmod(b(4:end),4,'{mode}',p);assert(max(abs([a;c]-y))<1e-14);[za,q,st]=mskdemod(a,4,'{mode}');[zc,q2,st2]=mskdemod(c,4,'{mode}',q,st);assert(isequal([za;zc],b));assert(q2==p2);assert(isequal(st2,state));"
            ),
        );
        for ns in [1, 2, 7, 8] {
            run(
                &mut e,
                &format!(
                    "b=reshape(mod(0:47,2),12,4);[y,p]=mskmod(b,{ns},'{mode}',[0 pi/2 pi 3*pi/2]);[z,q,st]=mskdemod(y,{ns},'{mode}',[0 pi/2 pi 3*pi/2]);assert(isequal(z,b));assert(isequal(p,q));assert(isequal(size(y),[{} 4]));assert(isequal(size(st),[{ns} 4]));",
                    12 * ns
                ),
            );
        }
    }
    run(
        &mut e,
        "[y,p]=mskmod([0 1 1],4,[],[]);assert(isequal(size(y),[12 1]));assert(isequal(mskdemod(y,4,[],[]),[0;1;1]));assert(isequal(mskmod([0;1],4),mskmod([0;1],4,'diff',0)));",
    );
}
#[test]
fn seeded_noise_symbol_error_counts() {
    let mut e = Engine::default();
    // Independent session RNG; all thresholds are explicit, out of 1024 symbols.
    run(
        &mut e,
        "rng(417);x=randi([0 7],1024,1);y=fskmod(x,8,1,16,16);r=y+.5*(randn(size(y))+1i*randn(size(y)));z=fskdemod(r,8,1,16,16);nf=sum(z~=x);assert(nf<12);y=dpskmod(x,8,pi/8,'gray');r=y+.08*(randn(size(y))+1i*randn(size(y)));z=dpskdemod(r,8,pi/8,'gray');nd=sum(z~=x);assert(nd<12);",
    );
    for mode in ["diff", "nondiff"] {
        run(
            &mut e,
            &format!(
                "b=randi([0 1],1024,1);y=mskmod(b,8,'{mode}');r=y+.08*(randn(size(y))+1i*randn(size(y)));z=mskdemod(r,8,'{mode}');nm=sum(z~=b);assert(nm<12);"
            ),
        );
    }
}
#[test]
fn demodulators_are_invariant_to_large_and_small_amplitudes() {
    let mut e = Engine::default();
    for amplitude in ["1e200", "1e-200"] {
        run(
            &mut e,
            &format!(
                "x=[0 1 2 3 3 1 0 2];y=dpskmod(x,4,pi/8,'gray');assert(isequal(dpskdemod({amplitude}*y,4,pi/8,'gray'),x));y=fskmod(x,4,1,8,8);assert(isequal(fskdemod({amplitude}*y,4,1,8,8),x));b=[0;1;1;0;1;0];y=mskmod(b,8);assert(isequal(mskdemod({amplitude}*y,8),b));"
            ),
        );
    }
}
#[test]
fn modem_input_errors_and_empty_arrays() {
    let mut e = Engine::default();
    for (s, part) in [
        ("fskmod([0 1],3,1,8,8)", "power of two"),
        ("dpskmod([0 4],4)", "integers"),
        ("dpskmod([0 NaN],4)", "finite"),
        ("dpskmod([0 .5],4)", "integers"),
        ("dpskmod([0 1],4,0,'bad')", "expected"),
        ("fskmod([0 1],4,3,8,8)", "freqsep"),
        ("fskmod([0 1],4,1,1,8)", "integer"),
        ("fskmod([0 1],4,-1,8,8)", "positive"),
        ("fskmod([0 1],4,1,8,8,'bad')", "expected"),
        ("fskdemod([1 2 3],2,1,2,2)", "multiple"),
        ("mskmod([0 2],8)", "integers"),
        ("mskmod([0 1],0)", "integer"),
        ("mskmod([0 1],8,'bad')", "expected"),
        ("mskmod([0 1],8,'diff',.1)", "pi/2"),
        ("mskmod([0 1;1 0],8,'diff',0)", "per channel"),
        ("mskdemod([1;2;3],2)", "multiple"),
        ("mskdemod(ones(8,1),4,[],[],ones(1,4))", "nsamp-by"),
        ("[a,b]=dpskmod([0 1],2)", "too many"),
        ("[a,b,c,d]=mskdemod(ones(8,1),4)", "too many"),
    ] {
        fails(&mut e, s, part);
    }
    run(
        &mut e,
        "assert(isempty(dpskmod([],4)));assert(isempty(dpskdemod([],4)));assert(isempty(fskmod([],4,1,8,8)));assert(isempty(fskdemod([],4,1,8,8)));assert(isempty(mskmod([],4)));assert(isempty(mskdemod([],4)));",
    );
}
#[test]
fn ofdm_numpy_cp_nulls_pilots_streams_batches_odd_fft_and_oversampling() {
    let mut e = Engine::default();
    for (n, nf, x, p, y, cp) in [
        (8, 8, OFDM_8_8_X, OFDM_8_8_P, OFDM_8_8_Y, "[2 4]"),
        (8, 12, OFDM_8_12_X, OFDM_8_12_P, OFDM_8_12_Y, "[2 4]"),
        (9, 12, OFDM_9_12_X, OFDM_9_12_P, OFDM_9_12_Y, "[3 6]"),
    ] {
        run(
            &mut e,
            &format!(
                "x=reshape({}, {},2,2,2);p=reshape({},2,2,2,2);cp={cp};null=[1;{}];piidx=[3;{n}];y=ofdmmod(x,{n},cp,null,piidx,p,OversamplingFactor={nf}/{n});[z,q]=ofdmdemod(y,{n},cp,[],null,piidx,'OversamplingFactor',{nf}/{n});assert(max(abs(z(:)-x(:)))<1e-12);assert(max(abs(q(:)-p(:)))<1e-12);assert(isequal(size(z),size(x)));assert(isequal(size(q),size(p)));[z,q]=ofdmdemod(y,{n},cp,[0 0],null,piidx,OversamplingFactor={nf}/{n});",
                complex_literal(x),
                n - 4,
                complex_literal(p),
                n / 2 + 1
            ),
        );
        // For fractional osf use offsets whose products are integers.
        near_complex(&e, "y", y);
        // Offset=0 always exercises phase correction without fractional lengths.
        run(
            &mut e,
            &format!(
                "[z,q]=ofdmdemod(y,{n},cp,0,null,piidx,OversamplingFactor={nf}/{n});assert(max(abs(z(:)-x(:)))<1e-12);assert(max(abs(q(:)-p(:)))<1e-12);"
            ),
        );
    }
}
#[test]
fn ofdm_scalar_vector_prefix_full_allocation_dc_null_and_options() {
    let mut e = Engine::default();
    for n in [8, 9, 12, 64] {
        run(
            &mut e,
            &format!(
                "x=reshape(1:{}, {n},3)+1i;for cp=[0 1 {n}];y=ofdmmod(x,{n},cp);z=ofdmdemod(y,{n},cp);assert(max(abs(z(:)-x(:)))<1e-11);end;",
                3 * n
            ),
        );
    }
    run(
        &mut e,
        "x=reshape(1:12,6,2)+1i;y=ofdmmod(x,8,[16 19],[1;5]);[z,p]=ofdmdemod(y,8,[16 19],[3 9],[1;5]);assert(max(abs(z(:)-x(:)))<1e-12);assert(isempty(p));y=ofdmmod(ones(8,1),8,2);z=ofdmdemod(y,8,2,0);assert(max(abs(z-1))<1e-12);d=ones(7,1);y=ofdmmod(d,8,0,5);f=fft(y);assert(abs(f(1))<1e-14);assert(max(abs(ofdmdemod(y,8,0,[],5)-d))<1e-12);y=ofdmmod(ones(8,1),8,2,OversamplingFactor=2);y1=ofdmmod(ones(8,1),8,2);assert(max(abs(y(1:2:end)-y1))<1e-12);",
    );
}
#[test]
fn ofdm_invalid_lengths_indices_pilot_layout_and_options() {
    let mut e = Engine::default();
    for (s, part) in [
        ("ofdmmod(ones(7,1),8,2)", "data rows"),
        ("ofdmmod(ones(4,1),4,0)", "integer"),
        ("ofdmmod(ones(8,1),8,9)", "cplen"),
        ("ofdmmod(ones(8,2),8,[1 2 3])", "per symbol"),
        ("ofdmmod(ones(6,1),8,2,[1;1])", "duplicate"),
        ("ofdmmod(ones(7,1),8,2,0)", "indices"),
        ("ofdmmod(ones(7,1),8,2,9)", "indices"),
        ("ofdmmod(ones(7,1),8,2,1,1,1)", "disjoint"),
        ("ofdmmod(ones(6,2),8,2,1,2,ones(1,1))", "pilots"),
        ("ofdmmod(ones(8,1),8,2,OversamplingFactor=1.3)", "integers"),
        ("ofdmmod(ones(8,1),8,2,'Bogus',2)", "OversamplingFactor"),
        ("ofdmdemod(ones(9,1),8,2)", "length"),
        ("ofdmdemod(ones(20,1),8,[1 2])", "length"),
        ("ofdmdemod(ones(20,1),8,2,3)", "symOffset"),
        ("ofdmdemod(ones(20,1),8,2,[0 1 2])", "symOffset"),
        ("ofdmmod(ones(8,1),8,2,[],[])", "positional"),
    ] {
        fails(&mut e, s, part);
    }
}
#[test]
fn scatterplot_editable_data_labels_decimation_target_and_hold() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x=[1+2i 3+4i 5+6i 7+8i 9+10i];f=scatterplot(x,2,1,'rx');a=findobj(f,'Type','axes');h=findobj(a,'Type','line');assert(numel(a)==1 && numel(h)==1);assert(isequal(get(h,'XData'),[3 7]));assert(isequal(get(h,'YData'),[4 8]));assert(strcmp(get(get(a,'XLabel'),'String'),'In-Phase'));assert(strcmp(get(get(a,'YLabel'),'String'),'Quadrature'));assert(strcmp(get(get(a,'Title'),'String'),'Scatter plot'));assert(strcmp(get(h,'Marker'),'x'));set(h,'XData',[30 70]);assert(isequal(get(h,'XData'),[30 70]));hold on;g=scatterplot([2;4],1,0,'b.',f);assert(isequal(g,f));assert(numel(findobj(f,'Type','line'))==2);hold off;",
    );
    run(
        &mut e,
        "f=scatterplot([1 10;2 20;3 30]);h=findobj(f,'Type','line');assert(isequal(get(h,'XData'),[1 2 3]));assert(isequal(get(h,'YData'),[10 20 30]));f=scatterplot([1 2 3]);h=findobj(f,'Type','line');assert(isequal(get(h,'YData'),[0 0 0]));scatterplot(1+2i);assert(numel(findobj(gcf,'Type','line'))==1);",
    );
}
#[test]
fn eyediagram_editable_overlapping_traces_alignment_and_iq_axes() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x=(1:9)+1i*(11:19);f=eyediagram(x,4,2,0,'r-');a=findobj(f,'Type','axes');assert(numel(a)==2);for j=1:2;h=findobj(a(j),'Type','line');assert(numel(h)==1);assert(strcmp(get(get(a(j),'XLabel'),'String'),'Time'));assert(strcmp(get(get(a(j),'YLabel'),'String'),'Amplitude'));xt=get(h,'XData');assert(isequal(xt(7:11),[-1 -.5 0 .5 1]));end;",
    );
    let f = e.figure.as_ref().unwrap();
    let axes = f.all_axes().map(|(_, a)| a).collect::<Vec<_>>();
    assert_eq!(axes.len(), 2);
    for a in &axes {
        assert_eq!(a.series.len(), 1);
        let want = if !a.title.contains("Quadrature") {
            [3., 4., 5., 6., 7.]
        } else {
            [13., 14., 15., 16., 17.]
        };
        let got = a.series[0]
            .points
            .iter()
            .skip(6)
            .take(5)
            .map(|p| p[1])
            .collect::<Vec<_>>();
        assert_eq!(got, want);
    }
    run(
        &mut e,
        "f=eyediagram([1;2;3;4;5;6],4);assert(numel(findobj(f,'Type','axes'))==1);f=eyediagram([1 10;2 20;3 30;4 40],4);assert(numel(findobj(f,'Type','axes'))==2);f=eyediagram([1;2;3;4;5],4,1,1);a=findobj(f,'Type','axes');h=findobj(a,'Type','line');yd=get(h,'YData');assert(isequaln(yd(1:5),[NaN 1 2 3 4]));eyediagram([1 2 3 4],4);f=eyediagram(mskmod(mod((1:400)',2),8),16,1,0,'b.');assert(numel(findobj(f,'Type','line'))==2);",
    );
}
#[test]
fn plot_validation_errors() {
    let mut e = Engine::default();
    for (s, part) in [
        ("scatterplot(ones(3,3))", "two-column"),
        ("scatterplot(ones(3,2)+1i)", "two-column"),
        ("scatterplot([1 2],0)", "integer"),
        ("scatterplot([1 2],1,-1)", "integer"),
        ("eyediagram([1 2],1)", "integer"),
        ("eyediagram([1 2],2,0)", "positive"),
        ("eyediagram([1 2],2,1,2)", "integer"),
        ("f=figure;a=gca;scatterplot([1 2],1,0,'b.',a)", "figure"),
    ] {
        fails(&mut e, s, part);
    }
}
#[test]
fn berfit_scipy_erfc_all_four_forms_and_coefficients() {
    let mut e = Engine::default();
    run(
        &mut e,
        &format!(
            "x={};y={};fx={};",
            literal(BER_X),
            literal(BER_Y),
            literal(BER_FX)
        ),
    );
    for kind in ["exp", "exp+const", "polyRatio", "doubleExp+const"] {
        run(
            &mut e,
            &format!(
                "[z,p]=berfit(x,y,fx,[],'{kind}');assert(strcmp(p.fitType,'{kind}'));assert(p.funcCount<=10000);assert(p.iterations<=10000);assert(all(z>0) && all(diff(z)<0));"
            ),
        );
        let z = real(&e, "z");
        assert_eq!(z.len(), BER_FY.len(), "{kind}");
        let error = z
            .iter()
            .zip(BER_FY)
            .map(|(a, b)| (a / b - 1.).abs())
            .fold(0., f64::max);
        assert!(error < 1e-3, "{kind}: max relative error={error}");
        // The public coefficient output must evaluate the same documented form.
        let expression = match kind {
            "exp" => "a(1)*exp(-((t-a(2)).^a(3))/a(4))",
            "exp+const" => "a(1)*exp(-((t-a(2)).^a(3))/a(4))+a(5)",
            "polyRatio" => "(a(1)*t.^2+a(2)*t+a(3))./(t.^3+a(4)*t.^2+a(5)*t+a(6))",
            _ => "a(1)*exp(-((t-a(2)).^a(3))/a(4))+a(5)*exp(-((t-a(6)).^a(7))/a(8))+a(9)",
        };
        run(
            &mut e,
            &format!("a=p.coeffs;t=10.^(fx/10);v={expression};assert(max(abs(v./z-1))<1e-10);"),
        );
    }
    run(
        &mut e,
        "[z,p]=berfit(x,y);assert(isequal(size(z),size(x)));assert(max(abs(z./y-1))<1e-3);zc=berfit(x',y');assert(isequal(size(zc),size(x')));assert(max(abs(zc./y'-1))<1e-3);opts=optimset('Display','off','MaxIter',100,'MaxFunEvals',30,'TolX',1e-8,'TolFun',1e-8);[z,p]=berfit(x,y,x,opts,'exp');assert(p.funcCount<=30);",
    );
}
#[test]
fn berfit_closed_forms_options_no_output_plot_and_invalid_inputs() {
    let mut e = Engine::default();
    run(
        &mut e,
        "x=0:.25:2;t=10.^(x/10);y=.4*exp(-t)+.001;berfit(x,y);f=gcf;a=gca;assert(strcmp(get(a,'YScale'),'log'));h=findobj(f,'Type','line');assert(numel(h)==2);assert(strcmp(get(get(a,'XLabel'),'String'),'Eb/No (dB)'));berfit(x,y,x,[],'all');assert(numel(findobj(gcf,'Type','line'))>=2);[z,p]=berfit(x,y,x,struct('Display','final','MaxFunEvals',100),'exp+const');assert(p.funcCount<=100);",
    );
    for (s, part) in [
        ("berfit([0 1 2],[.2 .1 .05])", "4"),
        ("berfit([0 1 2 3],[.2 .1 .05])", "equal"),
        ("berfit([0 2 1 3],[.2 .1 .05 .01])", "ascending"),
        ("berfit([0 1 2 3],[.2 .1 .05 0])", "(0,1]"),
        ("berfit([0 1 2 3],[.2 .1 .05 .01],[0 1])", "fitEbNo"),
        ("berfit(0:3,[.2 .1 .05 .01],[],[],'bogus')", "fittype"),
        ("berfit(0:3,[.2 .1 .05 .01],[],struct('Bogus',1))", "option"),
        (
            "z=berfit(0:3,[.2 .1 .05 .01],[],[],'all')",
            "without output",
        ),
    ] {
        fails(&mut e, s, part);
    }
}
#[test]
fn limits_cancellation_and_shared_inputs_survive() {
    let mut e = Engine::with_limits(Limits {
        memory_bytes: 64 * 1024,
        ..Limits::default()
    });
    fails(&mut e, "b=rcosdesign(.25,10000,8);", "memory");
    fails(&mut e, "y=fskmod([0 1],2,.1,80000,1);", "memory");
    fails(
        &mut e,
        "y=ofdmmod(ones(8,1),8,2,OversamplingFactor=4096);",
        "memory",
    );
    let mut e = Engine::default();
    run(&mut e, "x=[0;1;2;3];alias=x;");
    let old = e.array("x").unwrap();
    run(&mut e, "y=dpskmod(x,4);z=fskmod(x,4,1,8,8);");
    assert!(old.same_snapshot(&e.array("x").unwrap()));
    assert!(old.same_snapshot(&e.array("alias").unwrap()));
    let cancel = e.cancellation();
    cancel.store(true, Ordering::Relaxed);
    assert!(e.execute("rcosdesign(.25,6,4)").is_err());
    cancel.store(false, Ordering::Relaxed);
    run(&mut e, "assert(isequal(x,alias));b=rcosdesign(.25,6,4);");
}

#[test]
#[ignore = "manual release benchmark; see task notes for hardware and results"]
fn release_benchmark_wave_paths_1_and_8_workers() {
    use std::time::Instant;
    for workers in [1, 8] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap();
        pool.install(|| {
            let mut e=Engine::default();
            run(&mut e,"s=reshape(mod(0:8191,8),4096,2);b=mod(s,2);fm=fskmod(s,8,1,8,16);dm=dpskmod(s,8);mm=mskmod(b,8);ox=reshape(1:32768,256,64,2)+1i;oy=ofdmmod(ox,256,16);bx=0:.25:2;by=.5*erfc(sqrt(10.^(bx/10)));");
            for (name,source) in [
                ("rcosdesign_8001","r=rcosdesign(.25,1000,8);"),
                ("fskmod_8192x8","r=fskmod(s,8,1,8,16);"),
                ("fskdemod_8192x8x8","r=fskdemod(fm,8,1,8,16);"),
                ("dpskmod_8192","r=dpskmod(s,8);"),
                ("dpskdemod_8192","r=dpskdemod(dm,8);"),
                ("mskmod_8192x8","r=mskmod(b,8);"),
                ("mskdemod_8192x8","r=mskdemod(mm,8);"),
                ("ofdmmod_256x64x2","r=ofdmmod(ox,256,16);"),
                ("ofdmdemod_256x64x2","r=ofdmdemod(oy,256,16);"),
                ("berfit_9_best","r=berfit(bx,by);"),
            ] {
                run(&mut e,source);
                let begin=Instant::now();
                for _ in 0..5 {run(&mut e,source);}
                println!("comm_wave benchmark workers={workers} {name} {:.3} ms/call",begin.elapsed().as_secs_f64()*1000./5.);
            }
        });
    }
}
