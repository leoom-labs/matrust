//! Bounded serial design baseline. Configured worker counts are context controls,
//! not a claim that tiny coefficient design should execute in parallel.
use matrust::dsp::{self, Band};
use std::{hint::black_box, sync::atomic::AtomicBool, time::Instant};
const BATCH: usize = 32;
fn main() {
    if cfg!(debug_assertions) {
        panic!("Release required");
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(
        args.is_empty() || args == ["--reverse"],
        "Only --reverse is supported"
    );
    let mut bands = [
        Band::Lowpass,
        Band::Highpass,
        Band::Bandpass,
        Band::Bandstop,
    ];
    let mut orders = [1usize, 6, 20];
    let mut ripples = [0.5, 3.];
    let mut forms = ["sos", "ba", "zpk"];
    let mut workers = [1usize, 8];
    if !args.is_empty() {
        bands.reverse();
        orders.reverse();
        ripples.reverse();
        forms.reverse();
        workers.reverse();
    }
    println!("band,order,ripple_db,form,workers,sample,batch,total_ns,ns_per_design,output_bytes");
    let cancel = AtomicBool::new(false);
    for worker in workers {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(worker)
            .build()
            .unwrap();
        for band in bands {
            for order in orders {
                for ripple in ripples {
                    for form in forms {
                        let edges: &[f64] = if matches!(band, Band::Bandpass | Band::Bandstop) {
                            &[0.2, 0.65]
                        } else {
                            &[0.4]
                        };
                        let d = dsp::cheby1(order, ripple, edges, band, &cancel).unwrap();
                        let sos = d.sos(&cancel).unwrap();
                        let edge = dsp::sos_response(&sos, &[edges[0]], &cancel).unwrap()[0];
                        assert!((edge[0].hypot(edge[1]) - 10f64.powf(-ripple / 20.)).abs() < 2e-8);
                        let bytes = match form {
                            "sos" => sos.len() * 8,
                            "ba" => {
                                let (b, a) = d.transfer_function(&cancel).unwrap();
                                assert_eq!(b.len(), d.order() + 1);
                                assert_eq!(a.data()[0], 1.);
                                (b.len() + a.len()) * 8
                            }
                            "zpk" => {
                                let (z, p, k) = d.zero_pole_gain(&cancel).unwrap();
                                assert!(p.data().iter().all(|v| v.norm() < 1.));
                                assert!(k > 0.);
                                (z.len() + p.len()) * 16 + 8
                            }
                            _ => unreachable!(),
                        };
                        for iteration in 0..9 {
                            let start = Instant::now();
                            pool.install(|| {
                                for _ in 0..BATCH {
                                    let d =
                                        dsp::cheby1(order, ripple, edges, band, &cancel).unwrap();
                                    match form {
                                        "sos" => {
                                            black_box(d.sos(&cancel).unwrap());
                                        }
                                        "ba" => {
                                            black_box(d.transfer_function(&cancel).unwrap());
                                        }
                                        "zpk" => {
                                            black_box(d.zero_pole_gain(&cancel).unwrap());
                                        }
                                        _ => unreachable!(),
                                    }
                                }
                            });
                            let total = start.elapsed().as_nanos();
                            assert!(
                                total > 0,
                                "Zero batch duration must not be discarded or floored"
                            );
                            if iteration >= 2 {
                                println!(
                                    "{},{order},{ripple},{form},{worker},{},{BATCH},{total},{:.9},{bytes}",
                                    band.name(),
                                    iteration - 2,
                                    total as f64 / BATCH as f64
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
