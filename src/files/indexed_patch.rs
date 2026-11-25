//! Native indexed patches store the original column-major tables. The expanded
//! render stream is rebuilt from their one-based indices on load.
use super::{Matrix, Result, SeriesMetadata, take_native_f64, write_native_f64};
use crate::plot::{
    IndexedColor, IndexedPatch, MAX_PATCH_FACES, PlotKind, Series, coordinates::GeometryCost,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Metadata {
    vertices: [usize; 2],
    colors: bool,
    normals: bool,
    face_color: IndexedColor,
    color_range: Option<[f64; 2]>,
}

impl From<&IndexedPatch> for Metadata {
    fn from(mesh: &IndexedPatch) -> Self {
        Self {
            vertices: [mesh.vertices.rows(), mesh.vertices.cols()],
            colors: mesh.colors.is_some(),
            normals: mesh.normals.is_some(),
            face_color: mesh.face_color,
            color_range: mesh.color_range,
        }
    }
}

impl Metadata {
    pub(super) fn preflight(&self, series: &SeriesMetadata) -> Result<(GeometryCost, usize)> {
        let patch = series.patch.ok_or("Missing indexed patch topology")?;
        let faces = patch.face_count(series.count)?;
        let [rows, cols] = self.vertices;
        if series.kind != PlotKind::Patch
            || series.coordinates.is_some()
            || !matches!(cols, 2 | 3)
            || rows > 3 * MAX_PATCH_FACES
            || (faces != 0 && rows == 0)
            || series.z != (cols == 3 && faces != 0)
            || self
                .color_range
                .is_some_and(|[lo, hi]| !lo.is_finite() || !hi.is_finite() || lo > hi)
        {
            return Err("Invalid native indexed patch dimensions or attributes".into());
        }
        // Both table dimensions are bounded before the payload or render buffers are allocated.
        let columns = cols + usize::from(self.colors) + 3 * usize::from(self.normals);
        let table_bytes = 8 * (series.count + rows * columns);
        let z_bytes = if series.z { 8 * series.count } else { 0 };
        let mapped_bytes = if self.colors && self.face_color.mapped() {
            3 * faces
        } else {
            0
        };
        let cost = GeometryCost::dense_face_allocation(series.count, patch.vertices_per_face())?
            .with_additional_storage(table_bytes + z_bytes + mapped_bytes)?;
        Ok((cost, table_bytes))
    }

    pub(super) fn decode(
        &self,
        series: &SeriesMetadata,
        input: &mut &[u8],
    ) -> Result<IndexedPatch> {
        let width = series
            .patch
            .ok_or("Missing indexed patch topology")?
            .vertices_per_face();
        let [rows, cols] = self.vertices;
        let mut matrix =
            |r, c| Matrix::from_column_vec(r, c, take_native_f64(input, r * c).collect());
        Ok(IndexedPatch {
            faces: matrix(series.count / width, width)?,
            vertices: matrix(rows, cols)?,
            colors: self.colors.then(|| matrix(rows, 1)).transpose()?,
            normals: self.normals.then(|| matrix(rows, 3)).transpose()?,
            face_color: self.face_color,
            color_range: self.color_range,
            mapped_with: None,
        })
    }
}

pub(super) fn write(bytes: &mut Vec<u8>, series: &Series, mesh: &IndexedPatch) -> Result<()> {
    let n = mesh.vertices.rows();
    if series.patch.map(|p| p.vertices_per_face()) != Some(mesh.faces.cols())
        || mesh.faces.len() != series.points.len()
        || mesh
            .colors
            .as_ref()
            .is_some_and(|m| (m.rows(), m.cols()) != (n, 1))
        || mesh
            .normals
            .as_ref()
            .is_some_and(|m| (m.rows(), m.cols()) != (n, 3))
    {
        return Err("Invalid indexed patch table dimensions".into());
    }
    for matrix in [
        Some(&mesh.faces),
        Some(&mesh.vertices),
        mesh.colors.as_ref(),
        mesh.normals.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        write_native_f64(bytes, matrix.data().iter());
    }
    Ok(())
}
