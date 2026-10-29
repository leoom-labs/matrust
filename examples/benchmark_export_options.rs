#[cfg(feature = "plot-export")]
fn main() -> Result<(), String> {
    use std::{path::Path, time::Instant};
    let workers: usize = std::env::args()
        .nth(1)
        .unwrap_or("8".into())
        .parse()
        .unwrap();
    assert!([1, 8].contains(&workers));
    rayon::ThreadPoolBuilder::new().num_threads(workers).build().unwrap().install(|| {
        let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
        let mut engine = matrust::Engine::default();
        engine.execute("t=linspace(0,2,1000);x=sin(2*pi*3*t);y=0.7*sin(2*pi*3*t+0.4);plot(t,x,'b-',t,y,'r--');title('Custom export dimensions');xlabel('Time (seconds)');ylabel('Amplitude');legend('Reference','Measured');grid on;")?;
        let figure=engine.figure.as_ref().unwrap();
        println!("format,width,height,workers,points,repetitions,median_ms,min_ms,max_ms,file_bytes");
        for (width,height) in [(1440,840),(2880,1680),(1920,1080),(1600,1600)] {
            for extension in ["png","jpg","bmp","tiff"] {
                let path=directory.path().join(format!("{width}x{height}.{extension}"));
                let mut times=Vec::new();
                for i in 0..14 {
                    let start=Instant::now();
                    matrust::export::export_figure(figure,&path,width,height)?;
                    if i>=3 { times.push(start.elapsed().as_secs_f64()*1000.); }
                }
                let image=image::open(&path).map_err(|e| e.to_string())?.to_rgb8();
                assert_eq!(image.dimensions(),(width,height));
                assert!(image.pixels().any(|p| p.0[2]>p.0[0].saturating_add(40)));
                times.sort_by(f64::total_cmp);
                let bytes=std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
                println!("{extension},{width},{height},{workers},2000,11,{:.6},{:.6},{:.6},{bytes}",times[5],times[0],times[10]);
                if workers==1 && extension=="png" {
                    let destination=Path::new("target/evidence/plots/export-options");
                    std::fs::create_dir_all(destination).map_err(|e| e.to_string())?;
                    std::fs::copy(&path,destination.join(path.file_name().unwrap())).map_err(|e|e.to_string())?;
                }
            }
        }
        let path=directory.path().join("plot.csv");
        let mut times=Vec::new();
        for i in 0..14 {
            let start=Instant::now();
            matrust::files::write_figure_csv(&path,figure)?;
            if i>=3 { times.push(start.elapsed().as_secs_f64()*1000.); }
        }
        times.sort_by(f64::total_cmp);
        let bytes=std::fs::metadata(path).map_err(|e|e.to_string())?.len();
        println!("csv,0,0,{workers},2000,11,{:.6},{:.6},{:.6},{bytes}",times[5],times[0],times[10]);
        Ok(())
    })
}
#[cfg(not(feature = "plot-export"))]
fn main() {
    eprintln!("Use --features plot-export");
}
