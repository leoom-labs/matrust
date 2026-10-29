//! Snapshot identity without retaining replaced sample allocations.
use super::Figure;
use super::{SeriesPoints, WeakSeriesPoints};

#[derive(Clone, Debug)]
pub(crate) struct SnapshotStamp {
    metadata: Figure,
    points: Vec<WeakSeriesPoints>,
}
impl SnapshotStamp {
    pub fn new(figure: &Figure) -> Self {
        let points = figure
            .all_axes()
            .flat_map(|(_, a)| a.series.iter())
            .map(|s| s.points.downgrade())
            .collect();
        let mut metadata = figure.clone();
        let empty = SeriesPoints::from(Vec::new());
        for axes in metadata.all_axes_mut() {
            for series in &mut axes.series {
                series.points = empty.clone();
            }
        }
        Self { metadata, points }
    }
    pub fn matches(&self, figure: &Figure) -> bool {
        self.metadata.same_metadata(figure)
            && self
                .points
                .iter()
                .zip(figure.all_axes().flat_map(|(_, a)| a.series.iter()))
                .all(|(old, series)| old.matches(&series.points))
    }
}
