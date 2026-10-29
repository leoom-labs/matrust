//! Bounded recognition of exact ordinary Bar constructor batches, not source text.
use super::*;
use crate::plot::{Axes, MAX_SERIES};
use std::ops::Range;

pub(super) struct Plan {
    groups: Vec<Group>,
}
impl Plan {
    pub(super) fn new(axes: &Axes, cancel: &AtomicBool) -> crate::Result<Self> {
        Self::new_checked(axes, &|| check(cancel))
    }
    fn new_checked(
        axes: &Axes,
        checkpoint: &impl Fn() -> crate::Result<()>,
    ) -> crate::Result<Self> {
        checkpoint()?;
        if axes.series.len() > MAX_SERIES {
            return Err("Bar replay exceeds series limit".into());
        }
        let mut spacing = [None; MAX_SERIES];
        let mut first_bar = None::<usize>;
        let bars = axes
            .series
            .iter()
            .filter(|s| s.kind == PlotKind::Bar)
            .try_fold(0usize, |total, s| total.checked_add(s.points.len()))
            .ok_or("Bar replay count overflow")?;
        if bars > 10_000 {
            return Err("Bar replay exceeds geometry limit".into());
        }
        let fallback = || Self { groups: Vec::new() };
        for (index, series) in axes.series.iter().enumerate() {
            if series.kind != PlotKind::Bar {
                continue;
            }
            if series.base_value.is_some() {
                return Ok(fallback());
            }
            if let Some(first) = first_bar {
                let first: &crate::plot::Series = &axes.series[first];
                if !first
                    .baseline_identity
                    .same_object(&series.baseline_identity)
                {
                    return Ok(fallback());
                }
            } else {
                first_bar = Some(index);
            }
            let mut previous = None;
            let mut step = None::<f64>;
            for (row, point) in series.points.iter().enumerate() {
                if row.is_multiple_of(1024) {
                    checkpoint()?;
                }
                if !point[0].is_finite() {
                    return Ok(fallback());
                }
                if let Some(x) = previous {
                    if point[0] <= x {
                        return Ok(fallback());
                    }
                    let delta = point[0] - x;
                    step = Some(step.map_or(delta, |old| old.min(delta)));
                }
                previous = Some(point[0]);
            }
            let step = step.unwrap_or(1.);
            if !step.is_finite() || step <= 0. {
                return Ok(fallback());
            }
            spacing[index] = Some(step);
        }
        let mut groups = Vec::new();
        let mut start = 0;
        while start < axes.series.len() {
            checkpoint()?;
            if axes.series[start].kind != PlotKind::Bar {
                start += 1;
                continue;
            }
            let rows = axes.series[start].points.len();
            // Preserve the historical .8/1 recipe and its first matching group.
            // Only if all legacy candidate lengths fail do we try recovered factors.
            let mut lengths = Vec::new();
            let mut found = None;
            for count in 1..=axes.series.len() - start {
                checkpoint()?;
                if count > 1 && rows == 1 {
                    break;
                }
                if axes.series[start..start + count]
                    .iter()
                    .any(|s| s.kind != PlotKind::Bar || s.points.len() != rows)
                {
                    break;
                }
                let range = start..start + count;
                if matches_factors(axes, &spacing, range.clone(), 0.8, 1.) {
                    found = Some(Group {
                        range,
                        group_width: 0.8,
                        bar_width: 1.,
                    });
                    break;
                }
                lengths.push(count);
            }
            if found.is_none() {
                for count in lengths {
                    checkpoint()?;
                    if let Some(group) =
                        relative_factors(axes, &spacing, start..start + count, checkpoint)?
                    {
                        found = Some(group);
                        break;
                    }
                }
            }
            let Some(group) = found else {
                return Ok(fallback());
            };
            start = group.range.end;
            groups.push(group);
        }
        checkpoint()?;
        Ok(Self { groups })
    }
    pub(super) fn member(&self, index: usize) -> Option<(usize, usize)> {
        self.groups
            .iter()
            .find(|g| g.range.contains(&index))
            .map(|g| (index - g.range.start, g.range.len()))
    }
    pub(super) fn starting(&self, index: usize) -> Option<Group> {
        self.groups.iter().find(|g| g.range.start == index).cloned()
    }
    pub(super) fn first(&self) -> Option<usize> {
        self.groups.first().map(|g| g.range.start)
    }
    pub(super) fn canonical_record(&self, axes: &Axes, record: &crate::plot::Baseline) -> bool {
        let bars: Vec<_> = axes
            .series
            .iter()
            .enumerate()
            .filter(|(_, s)| s.kind == PlotKind::Bar)
            .collect();
        !bars.is_empty()
            && record.members.len() == bars.len()
            && bars.iter().all(|(index, s)| {
                self.member(*index).is_some()
                    && record.identity.same_object(&s.baseline_identity)
                    && record.members.iter().any(|id| id.same_object(&s.identity))
            })
    }
}

#[derive(Clone)]
pub(super) struct Group {
    range: Range<usize>,
    group_width: f64,
    bar_width: f64,
}

// Six seeds, each with itself and two adjacent positive floats on either side.
// This is a bounded exact recognizer, not an inverse rounded-arithmetic solver.
struct Candidates {
    values: [f64; 30],
    len: usize,
}
impl Candidates {
    fn new() -> Self {
        Self {
            values: [0.; 30],
            len: 0,
        }
    }
    fn add(&mut self, value: f64) {
        if value.is_finite()
            && value > 0.
            && value <= 1.
            && !self.values[..self.len].contains(&value)
        {
            // At most six calls to around(), each inserting at most five values.
            self.values[self.len] = value;
            self.len += 1;
        }
    }
    fn around(&mut self, value: f64) {
        if !value.is_finite() || value <= 0. {
            return;
        }
        self.add(value);
        let mut lower = value;
        let mut upper = value;
        for _ in 0..2 {
            lower = lower.next_down();
            upper = upper.next_up();
            self.add(lower);
            self.add(upper);
        }
    }
    fn iter(&self) -> impl Iterator<Item = f64> + '_ {
        self.values[..self.len].iter().copied()
    }
}

fn matches_factors(
    axes: &Axes,
    spacing: &[Option<f64>; MAX_SERIES],
    range: Range<usize>,
    group_width: f64,
    bar_width: f64,
) -> bool {
    let count = range.len();
    range.enumerate().all(|(column, index)| {
        let series = &axes.series[index];
        // Identical operation order to PlotProperties::apply_new_batch.
        let pitch = spacing[index].unwrap() * group_width / count as f64;
        let width = pitch * bar_width;
        let offset = (column as f64 - (count as f64 - 1.) / 2.) * pitch;
        width.is_finite()
            && width > 0.
            && offset.is_finite()
            && width.to_bits() == series.bar_width.to_bits()
            && offset.to_bits() == series.bar_offset.to_bits()
    })
}

fn relative_factors(
    axes: &Axes,
    spacing: &[Option<f64>; MAX_SERIES],
    range: Range<usize>,
    checkpoint: &impl Fn() -> crate::Result<()>,
) -> crate::Result<Option<Group>> {
    let count = range.len();
    // Choose widest member for the width quotient; no sample reads or allocations.
    let widest = range
        .clone()
        .max_by(|&a, &b| {
            axes.series[a]
                .bar_width
                .total_cmp(&axes.series[b].bar_width)
        })
        .unwrap();
    let width = axes.series[widest].bar_width;
    let step = spacing[widest].unwrap();
    let mut groups = Candidates::new();
    groups.around(1.);
    groups.around(0.8);
    groups.around((width * count as f64) / step);
    groups.around((width / step) * count as f64);
    // Largest nonzero offset avoids needlessly dividing a tiny centre offset.
    let anchor = range
        .clone()
        .enumerate()
        .filter(|(column, _)| *column as f64 != (count as f64 - 1.) / 2.)
        .max_by(|(_, a), (_, b)| {
            axes.series[*a]
                .bar_offset
                .abs()
                .total_cmp(&axes.series[*b].bar_offset.abs())
        });
    if let Some((column, index)) = anchor {
        let coefficient = column as f64 - (count as f64 - 1.) / 2.;
        let offset = axes.series[index].bar_offset;
        let step = spacing[index].unwrap();
        groups.around(((offset / coefficient) * count as f64) / step);
        groups.around(((offset / step) / coefficient) * count as f64);
    }
    for group_width in groups.iter() {
        checkpoint()?;
        let pitch = step * group_width / count as f64;
        let mut bars = Candidates::new();
        bars.around(1.);
        bars.around(0.8);
        bars.around(width / pitch);
        for bar_width in bars.iter() {
            checkpoint()?;
            if matches_factors(axes, spacing, range.clone(), group_width, bar_width) {
                return Ok(Some(Group {
                    range,
                    group_width,
                    bar_width,
                }));
            }
        }
    }
    Ok(None)
}

// A negative literal consumes two parser expression nodes. Each vector below
// therefore costs at most 1 + 2 * 120 = 241 nodes; the existing Bar planner's
// 10,000-value bound needs at most 84 vectors. The final reshape costs at most
// 1 (call) + 1 (matrix) + 84 (names) + 2 (dimensions) = 88 nodes.
// Concatenate once, rather than emitting transactional indexed writes that
// would repeatedly copy the complete destination during ordinary replay.
const INLINE_BLOCK_VALUES: usize = 120;

fn emit_inline_group_blocks(
    out: &mut impl Write,
    axes: &crate::plot::Axes,
    group: Range<usize>,
    component: usize,
    name: &str,
    cancel: &AtomicBool,
) -> fmt::Result {
    let rows = axes.series[group.start].points.len();
    let count = rows.checked_mul(group.len()).ok_or(fmt::Error)?;
    if count == 0 || count > 10_000 {
        return Err(fmt::Error);
    }
    let blocks = count.div_ceil(INLINE_BLOCK_VALUES);
    for block in 0..blocks {
        if cancel.load(Ordering::Relaxed) {
            return Err(fmt::Error);
        }
        writeln!(out, "mr_bar_values_{} = [", block + 1)?;
        let start = block * INLINE_BLOCK_VALUES;
        for index in start..(start + INLINE_BLOCK_VALUES).min(count) {
            let series = &axes.series[group.start + index / rows];
            let value = series.points.get(index % rows).ok_or(fmt::Error)?[component];
            writeln!(out, "{};", number(value))?;
        }
        writeln!(out, "];")?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(fmt::Error);
    }
    write!(out, "{name} = reshape([")?;
    for block in 1..=blocks {
        write!(out, "mr_bar_values_{block};")?;
    }
    writeln!(out, "],{rows},{});", group.len())?;
    write!(out, "clear")?;
    for block in 1..=blocks {
        write!(out, " mr_bar_values_{block}")?;
    }
    writeln!(out, ";")
}

pub(super) fn emit_group(
    out: &mut impl Write,
    axes: &crate::plot::Axes,
    group: Group,
    axis: usize,
    first_sample: usize,
    bundle: bool,
    cancel: &AtomicBool,
) -> fmt::Result {
    let group_width = group.group_width;
    let bar_width = group.bar_width;
    let group = group.range;
    let rows = axes.series[group.start].points.len();
    for (component, name) in [(0, "mr_group_x"), (1, "mr_group_y")] {
        if rows == 0 {
            writeln!(out, "{name} = zeros(0,{});", group.len())?;
        } else if bundle {
            write!(out, "{name} = [")?;
            for offset in 0..group.len() {
                write!(
                    out,
                    "mr_samples_{}(:,{}) ",
                    first_sample + offset,
                    component + 1
                )?;
            }
            writeln!(out, "];")?;
        } else if rows * group.len() > INLINE_BLOCK_VALUES {
            emit_inline_group_blocks(out, axes, group.clone(), component, name, cancel)?;
        } else {
            writeln!(out, "{name} = [")?;
            for row in 0..rows {
                if row.is_multiple_of(1024) && cancel.load(Ordering::Relaxed) {
                    return Err(fmt::Error);
                }
                for index in group.clone() {
                    write!(
                        out,
                        "{} ",
                        number(axes.series[index].points.get(row).ok_or(fmt::Error)?[component])
                    )?;
                }
                writeln!(out, ";")?;
            }
            writeln!(out, "];")?;
        }
    }
    // Ordinary documented graphics properties used by this replay.
    // The first-choice legacy recipe remains .8/1; other factors were verified
    // forward against every member using the same public ordered arithmetic.
    if group_width.to_bits() == 0.8f64.to_bits() && bar_width.to_bits() == 1f64.to_bits() {
        writeln!(
            out,
            "mr_bars = bar(mr_ax{axis},mr_group_x,mr_group_y,'GroupWidth',0.8,'BarWidth',1,'EdgeColor','none');\nclear mr_group_x mr_group_y;"
        )?;
    } else {
        writeln!(
            out,
            "mr_bars = bar(mr_ax{axis},mr_group_x,mr_group_y,'GroupWidth',{},'BarWidth',{},'EdgeColor','none');\nclear mr_group_x mr_group_y;",
            number(group_width),
            number(bar_width)
        )?;
    }
    if bundle {
        write!(out, "clear")?;
        for offset in 0..group.len() {
            write!(out, " mr_samples_{}", first_sample + offset)?;
        }
        writeln!(out, ";")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[test]
    fn inline_blocks_cancel_between_chunks_without_publishing_and_retry_exactly() {
        struct StopAfterBlock<'a> {
            text: String,
            stop: &'a AtomicBool,
        }
        impl Write for StopAfterBlock<'_> {
            fn write_str(&mut self, value: &str) -> fmt::Result {
                self.text.push_str(value);
                if self.text.ends_with("];\n") {
                    self.stop.store(true, Ordering::Relaxed);
                }
                Ok(())
            }
        }
        let mut engine = crate::Engine::default();
        engine.execute("bar(1:256,-(1:256));").unwrap();
        let figure = engine.figure.unwrap();
        let before = figure.clone();
        let stop = AtomicBool::new(false);
        let mut output = StopAfterBlock {
            text: String::new(),
            stop: &stop,
        };
        assert!(
            emit_inline_group_blocks(&mut output, &figure.axes, 0..1, 1, "mr_group_y", &stop)
                .is_err()
        );
        assert!(output.text.contains("mr_bar_values_1"));
        assert!(!output.text.contains("mr_bar_values_2"));
        assert!(!output.text.contains("mr_group_y ="));
        assert!(figure.same_snapshot(&before));
        assert!(
            figure.series[0]
                .points
                .same_storage(&before.series[0].points)
        );
        stop.store(false, Ordering::Relaxed);
        let mut retry = String::new();
        emit_inline_group_blocks(&mut retry, &figure.axes, 0..1, 1, "mr_group_y", &stop).unwrap();
        let mut replay = crate::Engine::default();
        replay.execute(&retry).unwrap();
        let values = replay.workspace();
        let actual = values["mr_group_y"].as_numeric().unwrap();
        assert_eq!((actual.rows(), actual.cols()), (256, 1));
        for (index, point) in figure.series[0].points.iter().enumerate() {
            assert_eq!(actual.get(index, 0).to_bits(), point[1].to_bits());
        }
        assert!(!values.keys().any(|name| name.starts_with("mr_bar_values_")));
    }

    #[test]
    fn exact_batch_planning_cancels_mid_scan_without_changing_sources_and_retries() {
        let mut engine = crate::Engine::default();
        engine
            .execute("bar(1:5000,[ones(5000,1) 2*ones(5000,1)]);")
            .unwrap();
        let figure = engine.figure.unwrap();
        let before = figure.clone();
        let visits = Cell::new(0usize);
        let result = Plan::new_checked(&figure, &|| {
            visits.set(visits.get() + 1);
            if visits.get() == 4 {
                Err("planned cancellation".into())
            } else {
                Ok(())
            }
        });
        assert!(matches!(result,Err(ref e) if e=="planned cancellation"));
        assert_eq!(visits.get(), 4);
        assert!(figure.same_snapshot(&before));
        for (a, b) in figure.series.iter().zip(&before.series) {
            assert!(a.points.same_storage(&b.points));
        }
        let plan = Plan::new(&figure, &AtomicBool::new(false)).unwrap();
        assert_eq!(plan.groups.len(), 1);
        assert_eq!(plan.groups[0].range, 0..2);
    }
    #[test]
    fn factor_candidates_are_bounded_and_require_exact_forward_bits() {
        let mut engine = crate::Engine::default();
        engine
            .execute("bar([1 3 7],[2 3;4 5;6 7],'GroupWidth',.5,'BarWidth',.25);")
            .unwrap();
        let figure = engine.figure.unwrap();
        let original = figure.clone();
        let visits = Cell::new(0usize);
        let plan = Plan::new_checked(&figure, &|| {
            visits.set(visits.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(plan.groups.len(), 1);
        assert_eq!(plan.groups[0].range, 0..2);
        assert_ne!(plan.groups[0].bar_width.to_bits(), 1f64.to_bits());
        assert!(
            visits.get() > 10,
            "must exercise recovered factors, not just the legacy path"
        );
        // Cancel in the final candidate search, before a plan can be published.
        let failure = visits.get() - 2;
        visits.set(0);
        let result = Plan::new_checked(&figure, &|| {
            visits.set(visits.get() + 1);
            if visits.get() == failure {
                Err("factor cancellation".into())
            } else {
                Ok(())
            }
        });
        assert!(matches!(result, Err(ref error) if error == "factor cancellation"));
        assert_eq!(visits.get(), failure);
        assert!(figure.same_snapshot(&original));
        assert_eq!(
            Plan::new(&figure, &AtomicBool::new(false)).unwrap().groups[0].range,
            0..2
        );

        let mut incompatible = figure.clone();
        incompatible.series[1].bar_offset = incompatible.series[1].bar_offset.next_up();
        // Equal spacings require exactly opposite offsets; one ULP is not tolerated.
        assert!(
            Plan::new(&incompatible, &AtomicBool::new(false))
                .unwrap()
                .groups
                .is_empty()
        );
        assert!(figure.same_snapshot(&original));

        let mut candidates = Candidates::new();
        for seed in [0., f64::from_bits(1), 0.25, 0.5, 1., f64::INFINITY] {
            candidates.around(seed);
        }
        assert!(candidates.len <= 30);
        assert!(
            candidates
                .iter()
                .all(|value| value.is_finite() && value > 0. && value <= 1.)
        );
        assert!(candidates.iter().any(|value| value.to_bits() == 1));
        assert!(!candidates.iter().any(|value| value == 0.));
    }

    #[test]
    fn subnormal_single_bar_width_is_exact_but_nonzero_offset_is_not_invented() {
        let mut engine = crate::Engine::default();
        engine.execute("bar([0 1],[2 3]);").unwrap();
        let mut figure = engine.figure.unwrap();
        figure.series[0].bar_width = f64::from_bits(1);
        let before = figure.clone();
        let plan = Plan::new(&figure, &AtomicBool::new(false)).unwrap();
        assert_eq!(plan.groups.len(), 1);
        let group = &plan.groups[0];
        let spacing = [Some(1.); MAX_SERIES];
        assert!(matches_factors(
            &figure,
            &spacing,
            group.range.clone(),
            group.group_width,
            group.bar_width
        ));
        assert!(figure.same_snapshot(&before));
        figure.series[0].bar_offset = f64::from_bits(1);
        assert!(
            Plan::new(&figure, &AtomicBool::new(false))
                .unwrap()
                .groups
                .is_empty()
        );
        figure.series[0].bar_offset = 0.;
        figure.series[0].bar_width = 2.;
        assert!(
            Plan::new(&figure, &AtomicBool::new(false))
                .unwrap()
                .groups
                .is_empty()
        );
    }
}
