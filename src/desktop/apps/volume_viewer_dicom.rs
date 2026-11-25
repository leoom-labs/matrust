use super::super::dicom_browser;
use matrust::{
    Array, CharacterRow, Engine, Matrix, Result, StructArray, TableArray, dsp::check_cancel,
};
use std::{path::Path, sync::Arc};

const MAX_SERIES_FILES: usize = 4096;

pub(super) struct SeriesList {
    pub(super) rows: Vec<dicom_browser::Series>,
    table: TableArray,
}

pub(super) fn scan(engine: &mut Engine, folder: &Path) -> Result<SeriesList> {
    let folder = folder
        .to_str()
        .ok_or("DICOM folder must be valid Unicode")?;
    let cancel = engine.cancellation();
    set_file(engine, folder)?;
    engine.execute("vv_collection = dicomCollection(vv_path, 'IncludeSubfolders', true);")?;
    let Some(Array::Table(table)) = engine.array("vv_collection") else {
        return Err("DICOM scan returned no series table".into());
    };
    let rows = dicom_browser::series_rows(&table, None, &cancel)?;
    Ok(SeriesList { rows, table })
}

pub(super) fn series_files(collection: &SeriesList, row: usize) -> Result<Vec<Arc<str>>> {
    let table = &collection.table;
    let column = |name: &str| {
        table
            .names()
            .iter()
            .position(|n| n == name)
            .map(|i| table.column(i))
    };
    let Some(Array::Cell(cells)) = column("Filenames") else {
        return Err("DICOM series has no file listing".into());
    };
    let Array::Str(paths) = cells.get_linear(row)?.as_ref() else {
        return Err("DICOM series has an invalid file listing".into());
    };
    if paths.is_empty() || paths.len() > MAX_SERIES_FILES {
        return Err(format!(
            "DICOM series must have 1 to {MAX_SERIES_FILES} files"
        ));
    }
    paths
        .data()
        .iter()
        .map(|item| {
            item.clone()
                .ok_or_else(|| "DICOM series contains a missing filename".into())
        })
        .collect()
}

struct Slice {
    file: Arc<str>,
    instance: Option<f64>,
    position: Option<[f64; 3]>,
    orientation: Option<[f64; 6]>,
    frames: usize,
    spacing: [Option<f64>; 3],
}

fn set_file(engine: &mut Engine, file: &str) -> Result<()> {
    let path = CharacterRow::from_text(file, &engine.cancellation())?;
    engine.insert_array("vv_path", Array::Character(path))
}

fn field<const N: usize>(info: &StructArray, name: &str) -> Option<[f64; N]> {
    let array = info.get(info.field_index(name)?, 0).ok()?;
    let values = match array.as_ref() {
        Array::Numeric(matrix) if matrix.len() == N => std::array::from_fn(|i| matrix.data()[i]),
        Array::Typed(matrix) if matrix.len() == N => std::array::from_fn(|i| matrix.real(i)),
        _ => return None,
    };
    values.iter().all(|v| v.is_finite()).then_some(values)
}

fn pixel_spacing(info: &StructArray) -> [Option<f64>; 3] {
    let [y, x] = field(info, "PixelSpacing").unwrap_or([0.0; 2]);
    let positive = |v| (v > 0.0).then_some(v);
    let z = field(info, "SpacingBetweenSlices")
        .and_then(|[v]| positive(v))
        .or_else(|| field(info, "SliceThickness").and_then(|[v]| positive(v)));
    [positive(x), positive(y), z]
}

fn geometry(slices: &mut [Slice]) -> [f64; 3] {
    // The browser supplies InstanceNumber order with a stable filename tie-break.
    // If the instance numbers are missing or repeated, use a physical stack order.
    let unique_instances = slices.iter().all(|slice| slice.instance.is_some())
        && slices.windows(2).all(|w| w[0].instance != w[1].instance);
    let all_positions = slices.iter().all(|s| s.position.is_some());
    let mut position_spacing = None;
    if all_positions && slices.len() > 1 {
        let normal =
            slices
                .iter()
                .find_map(|slice| slice.orientation)
                .and_then(|[a, b, c, d, e, f]| {
                    let cross = [b * f - c * e, c * d - a * f, a * e - b * d];
                    let length = cross[0].hypot(cross[1]).hypot(cross[2]);
                    (length > 0.0 && length.is_finite()).then(|| cross.map(|v| v / length))
                });
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        if !unique_instances {
            let direction = normal.unwrap_or_else(|| {
                let mut min = [f64::INFINITY; 3];
                let mut max = [f64::NEG_INFINITY; 3];
                for slice in slices.iter() {
                    for (axis, p) in slice.position.unwrap().into_iter().enumerate() {
                        min[axis] = min[axis].min(p);
                        max[axis] = max[axis].max(p);
                    }
                }
                let axis = (0..3)
                    .max_by(|&a, &b| (max[a] - min[a]).total_cmp(&(max[b] - min[b])))
                    .unwrap();
                std::array::from_fn(|i| f64::from(i == axis))
            });
            slices.sort_by(|a, b| {
                dot(a.position.unwrap(), direction).total_cmp(&dot(b.position.unwrap(), direction))
            });
        }
        let mut distances: Vec<_> = slices
            .windows(2)
            .filter_map(|pair| {
                let a = pair[0].position.unwrap();
                let b = pair[1].position.unwrap();
                let delta = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let distance = normal.map_or_else(
                    || delta[0].hypot(delta[1]).hypot(delta[2]),
                    |n| dot(delta, n).abs(),
                ) / pair[0].frames as f64;
                (distance.is_finite() && distance > 1e-9).then_some(distance)
            })
            .collect();
        distances.sort_by(f64::total_cmp);
        position_spacing = distances.get(distances.len() / 2).copied();
    }
    let mut spacing = std::array::from_fn(|axis| {
        slices
            .iter()
            .find_map(|slice| slice.spacing[axis])
            .unwrap_or(1.0)
    });
    if let Some(z) = position_spacing {
        spacing[2] = z;
    }
    spacing
}

pub(super) fn load(
    engine: &mut Engine,
    files: Vec<Arc<str>>,
    [rows, cols, pages]: [usize; 3],
) -> Result<(Array, [f64; 3])> {
    matrust::engine::volume_viewer_app_hooks::validate_dimensions([rows, cols, pages])?;
    if pages < 2 {
        return Err("Select a DICOM series with at least two grayscale slices".into());
    }
    let cancel = engine.cancellation();
    let mut slices = Vec::with_capacity(files.len());
    let mut depth = 0usize;
    for file in files {
        check_cancel(&cancel)?;
        set_file(engine, &file)?;
        engine.execute("vv_info = dicominfo(vv_path);")?;
        let Some(Array::Struct(info)) = engine.array("vv_info") else {
            return Err("No DICOM slice metadata".into());
        };
        engine.remove_array("vv_info");
        if field(&info, "Rows") != Some([rows as f64])
            || field(&info, "Columns") != Some([cols as f64])
            || field(&info, "SamplesPerPixel").unwrap_or([1.0]) != [1.0]
        {
            return Err(
                "DICOM slices must have identical dimensions and one grayscale channel".into(),
            );
        }
        let [frames] = field(&info, "NumberOfFrames").unwrap_or([1.0]);
        if frames.fract() != 0.0 || frames < 1.0 || frames > pages.saturating_sub(depth) as f64 {
            return Err("DICOM frame metadata does not match the selected series".into());
        }
        let frames = frames as usize;
        depth += frames;
        slices.push(Slice {
            file,
            instance: field(&info, "InstanceNumber").map(|[v]| v),
            position: field(&info, "ImagePositionPatient"),
            orientation: field(&info, "ImageOrientationPatient"),
            frames,
            spacing: pixel_spacing(&info),
        });
    }
    if depth != pages {
        return Err("DICOM frame metadata does not match the selected series".into());
    }
    let spacing = geometry(&mut slices);
    let plane = rows * cols;
    let mut voxel_data = Vec::with_capacity(plane * pages);
    for slice in slices {
        check_cancel(&cancel)?;
        set_file(engine, &slice.file)?;
        engine.execute("vv_part = dicomread(vv_path);")?;
        let part = engine
            .array("vv_part")
            .ok_or("DICOM read returned no pixel data")?;
        engine.remove_array("vv_part");
        // dicomread uses rows × cols × 1 × frames for multi-frame grayscale.
        let dims = part.dims();
        if dims.first() != Some(&rows)
            || dims.get(1) != Some(&cols)
            || dims.get(2).copied().unwrap_or(1) != 1
            || dims.get(3).copied().unwrap_or(1) != slice.frames
            || dims.len() > 4
        {
            return Err("DICOM decoded dimensions do not match the selected series".into());
        }
        match &part {
            Array::Typed(values) => {
                for start in (0..values.len()).step_by(4096) {
                    check_cancel(&cancel)?;
                    let end = (start + 4096).min(values.len());
                    voxel_data.extend((start..end).map(|i| values.real(i)));
                }
            }
            Array::Numeric(values) => voxel_data.extend_from_slice(values.data()),
            _ => return Err("DICOM read did not return real grayscale pixels".into()),
        }
    }
    let matrix =
        Matrix::from_column_vec(rows, cols * pages, voxel_data)?.with_dims(&[rows, cols, pages])?;
    Ok((Array::Numeric(matrix), spacing))
}
