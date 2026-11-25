//! Bounded native container of single-axes records. Nested containers
//! are rejected before decoding, and all axes share the figure-wide limits.
use super::*;
use crate::plot::{AxisLink, Layout, Panel, Position, TilePadding, TileSpacing};

pub(super) fn is_container(magic: &[u8]) -> bool {
    matches!(
        magic,
        b"MATRFIG6"
            | b"MATRFIG7"
            | b"MATRFIG8"
            | b"MATRFIGA"
            | b"MATRFIGC"
            | b"MATRFIGE"
            | b"MATRFIGG"
            | b"MATRFIGI"
            | b"MATRFIGK"
            | b"MATRFIGM"
            | b"MATRFIGO"
            | b"MATRFIGQ"
            | b"MATRFIGS"
            | b"MATRFIGU"
            | b"MATRFIGW"
            | b"MATRFIGY"
            | b"MATRFIGa"
            | NATIVE_EXTENDED_SUBPLOTS
    )
}

// Older containers keep their original per-record budgets. The extended
// revision also shares one metadata budget across every child record.
fn charge_metadata(remaining: &mut usize, size: usize) -> Result<()> {
    *remaining = remaining
        .checked_sub(size)
        .ok_or("Figure exceeds combined metadata limit")?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeAxisLink {
    axes: Vec<usize>,
    #[serde(default)]
    x: bool,
    #[serde(default)]
    y: bool,
}

fn is_default_tile_spacing(v: &TileSpacing) -> bool {
    *v == TileSpacing::default()
}

fn is_default_tile_padding(v: &TilePadding) -> bool {
    *v == TilePadding::default()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    appearance: Option<crate::plot::FigureAppearance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invert_hardcopy: Option<bool>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,
    #[serde(default = "white_color", skip_serializing_if = "is_white_color")]
    color: [u8; 3],
    #[serde(
        default = "default_figure_position",
        skip_serializing_if = "is_default_figure_position"
    )]
    position: [f64; 4],
    #[serde(default, skip_serializing_if = "crate::plot::FigureUnits::is_default")]
    units: crate::plot::FigureUnits,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    number_title: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    visible: bool,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    resize: bool,
    #[serde(default, skip_serializing_if = "crate::plot::WindowState::is_default")]
    window_state: crate::plot::WindowState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    textboxes: Vec<crate::plot::TextBox>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    annotations: Vec<crate::plot::Annotation>,
    active: usize,
    positions: Vec<Position>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    slice_viewers: Vec<slice_viewer_file::ViewerMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tiled: Option<crate::plot::TileGrid>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    title: String,
    #[serde(default, skip_serializing_if = "crate::plot::Subtitle::is_default")]
    subtitle: crate::plot::Subtitle,
    #[serde(default, skip_serializing_if = "crate::plot::LabelStyle::is_default")]
    title_style: crate::plot::LabelStyle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label_styles: Option<[crate::plot::LabelStyle; 2]>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    xlabel: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    ylabel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text_interpreters: Option<[crate::plot::text_markup::Interpreter; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text_lines: Option<[Option<crate::plot::text_markup::TextSource>; 3]>,
    #[serde(default, skip_serializing_if = "is_default_tile_spacing")]
    tile_spacing: TileSpacing,
    #[serde(default, skip_serializing_if = "is_default_tile_padding")]
    padding: TilePadding,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    linked_axes: Vec<NativeAxisLink>,
}
pub(super) fn encode_with_limit(figure: &Figure, limit: usize) -> Result<Vec<u8>> {
    let mut axes: Vec<_> = figure.all_axes().collect();
    axes.sort_by_key(|(p, _)| *p);
    let slice_viewers = if figure.slice_viewers.is_empty() && figure.orthoslice_viewers.is_empty() {
        Vec::new()
    } else {
        let panes: Vec<_> = axes.iter().map(|(_, a)| *a).collect();
        slice_viewer_file::capture(figure, &panes)?
    };
    let viewer_payload = slice_viewer_file::payload_size(&slice_viewers, axes.len())?;
    let layout = figure.layout.as_ref();
    let linked_axes: Vec<NativeAxisLink> = layout
        .map(|l| {
            l.linked_axes
                .iter()
                .filter_map(|link| {
                    let indices: Vec<usize> = link
                        .axes
                        .iter()
                        .filter_map(|id| axes.iter().position(|(_, a)| a.identity.same_object(id)))
                        .collect();
                    (indices.len() >= 2 && (link.x || link.y)).then_some(NativeAxisLink {
                        axes: indices,
                        x: link.x,
                        y: link.y,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let metadata = Metadata {
        appearance: figure.appearance,
        invert_hardcopy: figure.invert_hardcopy,
        name: figure.name.clone(),
        color: figure.color,
        position: figure.position,
        units: figure.units,
        number_title: figure.number_title,
        visible: figure.visible,
        resize: figure.resize,
        window_state: figure.window_state,
        textboxes: figure.textboxes.clone(),
        annotations: figure.annotations.clone(),
        active: axes
            .iter()
            .position(|(p, _)| *p == figure.active_position())
            .unwrap_or(0),
        positions: axes.iter().map(|(p, _)| *p).collect(),
        slice_viewers,
        tiled: layout.and_then(|l| l.tiled),
        title: layout.map_or_else(String::new, |l| l.title.clone()),
        subtitle: layout.map_or_else(Default::default, |l| l.subtitle.clone()),
        title_style: layout.map_or_else(Default::default, |l| l.title_style),
        label_styles: layout.and_then(|l| {
            l.label_styles
                .iter()
                .any(|s| !s.is_default())
                .then_some(l.label_styles)
        }),
        xlabel: layout.map_or_else(String::new, |l| l.xlabel.clone()),
        ylabel: layout.map_or_else(String::new, |l| l.ylabel.clone()),
        text_interpreters: layout.and_then(|l| {
            (l.text_interpreters != [crate::plot::text_markup::Interpreter::default(); 3])
                .then_some(l.text_interpreters)
        }),
        text_lines: layout.and_then(|l| {
            l.text_lines
                .iter()
                .any(Option::is_some)
                .then(|| l.text_lines.clone())
        }),
        tile_spacing: layout.map_or_else(TileSpacing::default, |l| l.tile_spacing),
        padding: layout.map_or_else(TilePadding::default, |l| l.padding),
        linked_axes,
    };
    let json = serde_json::to_vec(&metadata).map_err(|e| e.to_string())?;
    if json.len() > NATIVE_METADATA_LIMIT {
        return Err("Figure exceeds metadata save limit".into());
    }
    let mut bytes = Vec::with_capacity((16 * figure.point_count() + 4096).min(limit));
    let extended = !metadata.slice_viewers.is_empty()
        || axes.iter().any(|(_, a)| !a.extras.volumes.is_empty());
    let mut metadata_remaining = NATIVE_SUBPLOT_METADATA_LIMIT;
    let needs_a = !metadata.xlabel.is_empty()
        || !metadata.ylabel.is_empty()
        || !metadata.subtitle.is_default()
        || !metadata.title_style.is_default()
        || metadata.label_styles.is_some()
        || metadata.text_interpreters.is_some()
        || metadata.text_lines.is_some()
        || metadata.tile_spacing != TileSpacing::default()
        || metadata.padding != TilePadding::default()
        || !metadata.linked_axes.is_empty()
        || axes.iter().any(|(_, a)| super::needs_mfigb(a));
    let legend_helpers = axes.iter().any(|(_, a)| super::needs_mfigz(a));
    let limit_error = || {
        super::native_figure_limit_error(
            limit,
            figure.all_axes().any(|(_, a)| !a.extras.volumes.is_empty()),
        )
    };
    if 12 + json.len() + viewer_payload > limit {
        return Err(limit_error());
    }
    bytes.extend_from_slice(if extended {
        charge_metadata(&mut metadata_remaining, json.len())?;
        NATIVE_EXTENDED_SUBPLOTS
    } else if needs_a {
        b"MATRFIGa"
    } else if figure.has_static_style_metadata() || legend_helpers {
        b"MATRFIGY"
    } else if figure.has_palette_metadata() {
        b"MATRFIGW"
    } else if figure.all_axes().any(|(_, a)| a.colors.is_some()) {
        b"MATRFIGU"
    } else if figure.appearance.is_some() {
        b"MATRFIGS"
    } else if axes.iter().any(|(_, a)| {
        baseline_metadata::needs_latent(a)
            || !a.baselines.is_empty()
            || baseline_metadata::needs_bar_visibility(a)
    }) {
        b"MATRFIGQ"
    } else if axes.iter().any(|(_, a)| {
        a.series.iter().any(|series| {
            series
                .points
                .patch()
                .and_then(|p| p.native_descriptor())
                .is_some()
        })
    }) {
        b"MATRFIGO"
    } else if axes
        .iter()
        .any(|(_, a)| a.series.iter().any(crate::plot::Series::has_face_style))
    {
        b"MATRFIGM"
    } else if !figure.name.is_empty()
        || figure.color != [255; 3]
        || !figure.textboxes.is_empty()
        || !figure.annotations.is_empty()
        || axes.iter().any(|(_, a)| {
            a.legend_state.is_some()
                || a.xgrid != a.ygrid
                || a.series
                    .iter()
                    .any(|s| s.patch.is_some() || s.base_value.is_some())
        })
    {
        b"MATRFIGK"
    } else if axes
        .iter()
        .any(|(_, a)| a.placement.is_some() || a.legend_location.requires_extended_layout())
    {
        b"MATRFIGI"
    } else if axes.iter().any(|(_, a)| {
        a.series
            .iter()
            .any(|s| s.pattern == crate::plot::LinePattern::None)
    }) {
        b"MATRFIGG"
    } else if axes.iter().any(|(_, a)| !a.legend_location.is_default()) {
        b"MATRFIGE"
    } else if axes
        .iter()
        .any(|(_, a)| a.series.iter().any(|s| s.has_marker_style()))
    {
        b"MATRFIGC"
    } else if axes
        .iter()
        .any(|(_, a)| a.series.iter().any(|s| s.line_width.is_some()))
    {
        b"MATRFIGA"
    } else if metadata.positions.iter().any(|p| p.custom.is_some()) {
        b"MATRFIG8"
    } else if metadata.tiled.is_some() || !metadata.title.is_empty() {
        b"MATRFIG7"
    } else {
        b"MATRFIG6"
    });
    bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&json);
    for (index, (_, axes)) in axes.into_iter().enumerate() {
        let mut axes = axes.clone();
        slice_viewer_file::prepare_pane(&metadata.slice_viewers, index, &mut axes)?;
        let payload = encode_native_figure_with_limit(
            &Figure {
                identity: crate::ObjectIdentity::default(),
                axes,
                layout: None,
                textboxes: Vec::new(),

                name: String::new(),
                color: [255; 3],
                appearance: None,
                invert_hardcopy: None,
                ..Figure::default()
            },
            limit,
        )?;
        if extended {
            let size = u32::from_le_bytes(payload[8..12].try_into().unwrap()) as usize;
            charge_metadata(&mut metadata_remaining, size)?;
        }
        if bytes.len() + 4 + payload.len() + viewer_payload > limit {
            return Err(limit_error());
        }
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);
    }
    slice_viewer_file::write_payload(&mut bytes, figure);
    Ok(bytes)
}
pub(super) fn decode(bytes: &[u8]) -> Result<Figure> {
    if bytes.len() < 12 || bytes.len() > super::NATIVE_FIGURE_LIMIT {
        return Err("Invalid subplot container length".into());
    }
    let extended = bytes.starts_with(NATIVE_EXTENDED_SUBPLOTS);
    let version: &[u8] = if extended { b"MATRFIGa" } else { &bytes[..8] };
    let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let mut metadata_remaining = NATIVE_SUBPLOT_METADATA_LIMIT;
    if size > NATIVE_METADATA_LIMIT || size > bytes.len() - 12 {
        return Err("Invalid subplot metadata length".into());
    }
    let metadata: Metadata =
        serde_json::from_slice(&bytes[12..12 + size]).map_err(|e| e.to_string())?;
    if metadata.invert_hardcopy.is_some() && !matches!(version, b"MATRFIGY" | b"MATRFIGa") {
        return Err("Subplot print preference requires MFIGY".into());
    }
    if !version.starts_with(b"MATRFIGa")
        && (!metadata.xlabel.is_empty()
            || !metadata.ylabel.is_empty()
            || !metadata.subtitle.is_default()
            || !metadata.title_style.is_default()
            || metadata.label_styles.is_some()
            || metadata.text_interpreters.is_some()
            || metadata.text_lines.is_some()
            || metadata.tile_spacing != TileSpacing::default()
            || metadata.padding != TilePadding::default()
            || !metadata.linked_axes.is_empty())
    {
        return Err("Extended tiled layout and linkaxes metadata require MFIGa".into());
    }
    if metadata
        .appearance
        .is_some_and(|a| a.series_palette.is_some())
        && !matches!(version, b"MATRFIGW" | b"MATRFIGY" | b"MATRFIGa")
    {
        return Err("Subplot automatic palette metadata requires MFIGW".into());
    }
    if metadata.appearance.is_some()
        && !matches!(
            version,
            b"MATRFIGS" | b"MATRFIGU" | b"MATRFIGW" | b"MATRFIGY" | b"MATRFIGa"
        )
    {
        return Err("Subplot appearance intent requires MFIGS".into());
    }
    if !matches!(
        version,
        b"MATRFIGK"
            | b"MATRFIGM"
            | b"MATRFIGO"
            | b"MATRFIGQ"
            | b"MATRFIGS"
            | b"MATRFIGU"
            | b"MATRFIGW"
            | b"MATRFIGY"
            | b"MATRFIGa"
    ) && (!metadata.textboxes.is_empty()
        || !metadata.annotations.is_empty()
        || !metadata.name.is_empty()
        || metadata.color != [255; 3])
    {
        return Err("Advanced subplot metadata requires MFIGK".into());
    }
    if version.starts_with(b"MATRFIG6") && (metadata.tiled.is_some() || !metadata.title.is_empty())
    {
        return Err("MFIG6 does not contain tiled layout metadata".into());
    }
    if !matches!(
        version,
        b"MATRFIG8"
            | b"MATRFIGA"
            | b"MATRFIGC"
            | b"MATRFIGE"
            | b"MATRFIGG"
            | b"MATRFIGI"
            | b"MATRFIGK"
            | b"MATRFIGM"
            | b"MATRFIGO"
            | b"MATRFIGQ"
            | b"MATRFIGS"
            | b"MATRFIGU"
            | b"MATRFIGW"
            | b"MATRFIGY"
            | b"MATRFIGa"
    ) && metadata.positions.iter().any(|p| p.custom.is_some())
    {
        return Err("Custom axes positions require MFIG8".into());
    }
    let empty = metadata.tiled.is_some_and(|g| g.pending_first);
    if (empty && (!metadata.positions.is_empty() || metadata.active != 0))
        || (!empty
            && (metadata.positions.is_empty() || metadata.active >= metadata.positions.len()))
        || metadata.positions.len() > 16
    {
        return Err("Invalid subplot axes count or active position".into());
    }
    if !extended && !metadata.slice_viewers.is_empty() {
        return Err("Slice viewer metadata requires MFIGd".into());
    }
    let viewer_size =
        slice_viewer_file::payload_size(&metadata.slice_viewers, metadata.positions.len())?;
    let viewer_offset = bytes
        .len()
        .checked_sub(viewer_size)
        .filter(|&end| end >= 12 + size)
        .ok_or("Truncated native slice viewer payload")?;
    let (bytes, viewer_payload) = bytes.split_at(viewer_offset);
    let mut figure = Figure {
        textboxes: metadata.textboxes,
        annotations: metadata.annotations,
        name: metadata.name,
        color: metadata.color,
        appearance: metadata.appearance,
        invert_hardcopy: metadata.invert_hardcopy,
        position: metadata.position,
        units: metadata.units,
        number_title: metadata.number_title,
        visible: metadata.visible,
        resize: metadata.resize,
        window_state: metadata.window_state,
        layout: Some(Layout {
            tiled: metadata.tiled,
            title: metadata.title,
            subtitle: metadata.subtitle,
            title_style: metadata.title_style,
            label_styles: metadata.label_styles.unwrap_or_default(),
            xlabel: metadata.xlabel,
            ylabel: metadata.ylabel,
            text_interpreters: metadata.text_interpreters.unwrap_or_default(),
            text_lines: metadata.text_lines.unwrap_or_default(),
            tile_spacing: metadata.tile_spacing,
            padding: metadata.padding,
            ..Layout::plain(if empty {
                let grid = metadata.tiled.unwrap();
                grid.validate()?;
                Position::from_grid(grid.rows, grid.columns, &[1])?
            } else {
                metadata.positions[metadata.active]
            })
        }),
        ..Default::default()
    };
    figure.axes = figure.fresh_axes();
    // Validate geometry before allocating sample buffers.
    for (i, &position) in metadata.positions.iter().enumerate() {
        if i != metadata.active {
            figure.layout.as_mut().unwrap().others.push(Panel {
                position,
                axes: Default::default(),
            });
        }
    }
    figure.validate()?;
    figure.layout.as_mut().unwrap().others.clear();
    if extended {
        charge_metadata(&mut metadata_remaining, size)?;
    }
    // U's complete style ledger is admitted before any child's sample allocation.
    // Old containers retain their existing validation/error precedence.
    if matches!(
        version,
        b"MATRFIGU" | b"MATRFIGW" | b"MATRFIGY" | b"MATRFIGa"
    ) {
        #[derive(Deserialize)]
        struct ColorsOnly {
            #[serde(default)]
            axes_colors: Option<crate::plot::AxesColors>,
            #[serde(default)]
            invert_hardcopy: Option<bool>,
        }
        let mut cursor = 12 + size;
        for _ in &metadata.positions {
            let prefix = bytes
                .get(cursor..cursor + 4)
                .ok_or("Truncated subplot record")?;
            let length = u32::from_le_bytes(prefix.try_into().unwrap()) as usize;
            cursor += 4;
            let child = bytes
                .get(cursor..cursor + length)
                .ok_or("Truncated subplot samples")?;
            cursor += length;
            if child.len() < 12 {
                return Err("Invalid subplot child metadata".into());
            }
            let length = u32::from_le_bytes(child[8..12].try_into().unwrap()) as usize;
            if length > super::NATIVE_AXES_METADATA_LIMIT {
                return Err("Invalid figure metadata size".into());
            }
            if extended {
                charge_metadata(&mut metadata_remaining, length)?;
            }
            let child_version = if child.starts_with(NATIVE_EXTENDED_AXES) {
                if !extended {
                    return Err("Extended axes require MFIGd subplot container".into());
                }
                b"MATRFIGb".as_slice()
            } else {
                &child[..8]
            };
            let json = child
                .get(12..12 + length)
                .ok_or("Invalid figure metadata size")?;
            let metadata: ColorsOnly = serde_json::from_slice(json).map_err(|e| e.to_string())?;
            if metadata.invert_hardcopy.is_some() {
                return Err(
                    "InvertHardcopy belongs to the figure, not an embedded axes record".into(),
                );
            }
            if child.starts_with(b"MATRFIGX") && !matches!(version, b"MATRFIGY" | b"MATRFIGa") {
                return Err("Subplot legend RGB capability requires MFIGY".into());
            }
            if metadata.axes_colors.is_some_and(|c| c.legend.is_some())
                && (!matches!(version, b"MATRFIGY" | b"MATRFIGa")
                    || !matches!(child_version, b"MATRFIGX" | b"MATRFIGZ" | b"MATRFIGb"))
            {
                return Err(
                    "Subplot legend RGB metadata requires MFIGY with MFIGX children".into(),
                );
            }
            if matches!(version, b"MATRFIGW" | b"MATRFIGY" | b"MATRFIGa") {
                if is_container(child_version) {
                    return Err("Nested subplot containers are unsupported".into());
                }
                let palette: FigureMetadata =
                    serde_json::from_slice(json).map_err(|e| e.to_string())?;
                if matches!(version, b"MATRFIGY" | b"MATRFIGa")
                    && (palette.appearance.is_some()
                        || !palette.name.is_empty()
                        || !palette.textboxes.is_empty()
                        || palette.color != [255; 3])
                {
                    return Err("Figure-only properties are not embedded axes metadata".into());
                }
                let has = palette.palette_cycle.is_some()
                    || palette.series.iter().any(|s| s.auto_palette.is_some())
                    || palette
                        .appearance
                        .is_some_and(|a| a.series_palette.is_some());
                if has
                    && !matches!(
                        child_version,
                        b"MATRFIGV" | b"MATRFIGX" | b"MATRFIGZ" | b"MATRFIGb"
                    )
                {
                    return Err("Automatic palette metadata requires MFIGV".into());
                }
                if let Some(cycle) = palette.palette_cycle {
                    cycle.validate()?;
                }
                for series in &palette.series {
                    if let Some(p) = series.auto_palette {
                        p.validate()?;
                        if series.kind == crate::plot::PlotKind::Patch {
                            return Err("Patch colors require explicit RGB".into());
                        }
                    }
                }
            }
            if let Some(colors) = metadata.axes_colors {
                if !matches!(
                    child_version,
                    b"MATRFIGT" | b"MATRFIGV" | b"MATRFIGX" | b"MATRFIGZ" | b"MATRFIGb"
                ) {
                    return Err("Axes RGB metadata requires MFIGT".into());
                }
                colors.validate()?;
            }
        }
    }
    let mut offset = 12 + size;
    let mut decoded_ids = Vec::with_capacity(metadata.positions.len());
    for (i, position) in metadata.positions.into_iter().enumerate() {
        if bytes.len() - offset < 4 {
            return Err("Truncated subplot record".into());
        }
        let size = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        offset += 4;
        if size > bytes.len() - offset {
            return Err("Truncated subplot samples".into());
        }
        let child = &bytes[offset..offset + size];
        if child.get(..8).is_some_and(is_container) {
            return Err("Nested subplot containers are unsupported".into());
        }
        if child.starts_with(NATIVE_EXTENDED_AXES) && !extended {
            return Err("Extended axes require MFIGd subplot container".into());
        }
        if !matches!(
            version,
            b"MATRFIGI"
                | b"MATRFIGK"
                | b"MATRFIGM"
                | b"MATRFIGO"
                | b"MATRFIGQ"
                | b"MATRFIGS"
                | b"MATRFIGU"
                | b"MATRFIGW"
                | b"MATRFIGY"
                | b"MATRFIGa"
        ) && child.starts_with(b"MATRFIGH")
        {
            return Err("Extended subplot legend/axes layout requires MFIGI".into());
        }
        if !matches!(
            version,
            b"MATRFIGG"
                | b"MATRFIGI"
                | b"MATRFIGK"
                | b"MATRFIGM"
                | b"MATRFIGO"
                | b"MATRFIGQ"
                | b"MATRFIGS"
                | b"MATRFIGU"
                | b"MATRFIGW"
                | b"MATRFIGY"
                | b"MATRFIGa"
        ) && child.starts_with(b"MATRFIGF")
        {
            return Err("Subplot LineStyle none requires MFIGG".into());
        }
        if !matches!(
            version,
            b"MATRFIGE"
                | b"MATRFIGG"
                | b"MATRFIGI"
                | b"MATRFIGK"
                | b"MATRFIGM"
                | b"MATRFIGO"
                | b"MATRFIGQ"
                | b"MATRFIGS"
                | b"MATRFIGU"
                | b"MATRFIGW"
                | b"MATRFIGY"
                | b"MATRFIGa"
        ) && child.starts_with(b"MATRFIGD")
        {
            return Err("Subplot legend locations require MFIGE".into());
        }
        if !matches!(
            version,
            b"MATRFIGC"
                | b"MATRFIGE"
                | b"MATRFIGG"
                | b"MATRFIGI"
                | b"MATRFIGK"
                | b"MATRFIGM"
                | b"MATRFIGO"
                | b"MATRFIGQ"
                | b"MATRFIGS"
                | b"MATRFIGU"
                | b"MATRFIGW"
                | b"MATRFIGY"
                | b"MATRFIGa"
        ) && child.starts_with(b"MATRFIGB")
        {
            return Err("Marker style metadata requires MFIGC".into());
        }
        if !matches!(
            version,
            b"MATRFIGA"
                | b"MATRFIGC"
                | b"MATRFIGE"
                | b"MATRFIGG"
                | b"MATRFIGI"
                | b"MATRFIGK"
                | b"MATRFIGM"
                | b"MATRFIGO"
                | b"MATRFIGQ"
                | b"MATRFIGS"
                | b"MATRFIGU"
                | b"MATRFIGW"
                | b"MATRFIGY"
                | b"MATRFIGa"
        ) && child.starts_with(b"MATRFIG9")
        {
            return Err("Explicit subplot line widths require MFIGA".into());
        }
        if !matches!(
            version,
            b"MATRFIGK"
                | b"MATRFIGM"
                | b"MATRFIGO"
                | b"MATRFIGQ"
                | b"MATRFIGS"
                | b"MATRFIGU"
                | b"MATRFIGW"
                | b"MATRFIGY"
                | b"MATRFIGa"
        ) && child.starts_with(b"MATRFIGJ")
        {
            return Err("Advanced subplot metadata requires MFIGK".into());
        }
        if !matches!(
            version,
            b"MATRFIGO"
                | b"MATRFIGQ"
                | b"MATRFIGS"
                | b"MATRFIGU"
                | b"MATRFIGW"
                | b"MATRFIGY"
                | b"MATRFIGa"
        ) && child.starts_with(b"MATRFIGN")
        {
            return Err("Compact subplot coordinates require MFIGO".into());
        }
        if !matches!(
            version,
            b"MATRFIGM"
                | b"MATRFIGO"
                | b"MATRFIGQ"
                | b"MATRFIGS"
                | b"MATRFIGU"
                | b"MATRFIGW"
                | b"MATRFIGY"
                | b"MATRFIGa"
        ) && child.starts_with(b"MATRFIGL")
        {
            return Err("Independent subplot face outlines require MFIGM".into());
        }
        if !matches!(
            version,
            b"MATRFIGQ" | b"MATRFIGS" | b"MATRFIGU" | b"MATRFIGW" | b"MATRFIGY" | b"MATRFIGa"
        ) && child.starts_with(b"MATRFIGP")
        {
            return Err("Independent subplot baselines require MFIGQ".into());
        }
        if child.starts_with(b"MATRFIGT")
            && !matches!(
                version,
                b"MATRFIGU" | b"MATRFIGW" | b"MATRFIGY" | b"MATRFIGa"
            )
        {
            return Err("Subplot axes RGB capability requires MFIGU".into());
        }
        if child.starts_with(b"MATRFIGR")
            && !matches!(
                version,
                b"MATRFIGS" | b"MATRFIGU" | b"MATRFIGW" | b"MATRFIGY" | b"MATRFIGa"
            )
        {
            return Err("Subplot appearance capability requires MFIGS".into());
        }
        if child.starts_with(b"MATRFIGV")
            && !matches!(version, b"MATRFIGW" | b"MATRFIGY" | b"MATRFIGa")
        {
            return Err("Subplot palette capability requires MFIGW".into());
        }
        if child.starts_with(b"MATRFIGX") && !matches!(version, b"MATRFIGY" | b"MATRFIGa") {
            return Err("Subplot legend RGB capability requires MFIGY".into());
        }
        if child.starts_with(b"MATRFIGZ") && !matches!(version, b"MATRFIGY" | b"MATRFIGa") {
            return Err("Subplot legend helper capability requires MFIGY".into());
        }
        if child.starts_with(b"MATRFIGb") && !version.starts_with(b"MATRFIGa") {
            return Err("Subplot text, color order and bar group capability requires MFIGa".into());
        }
        let decoded = decode_native_figure_with_ledger(child, Some(figure.geometry_ledger()?))?;
        if decoded.invert_hardcopy.is_some()
            || decoded.appearance.is_some()
            || !decoded.textboxes.is_empty()
            || !decoded.name.is_empty()
            || decoded.color != [255; 3]
        {
            return Err(
                "Name, Color and textboxes belong to the figure, not an embedded axes record"
                    .into(),
            );
        }
        decoded_ids.push(decoded.axes.identity.clone());
        if i == metadata.active {
            figure.axes = decoded.axes;
        } else {
            figure.layout.as_mut().unwrap().others.push(Panel {
                position,
                axes: decoded.axes,
            });
        }
        figure.validate()?;
        offset += size;
    }
    if offset != bytes.len() {
        return Err("Trailing bytes in subplot container".into());
    }
    let layout = figure.layout.as_mut().unwrap();
    layout.others.sort_by_key(|p| p.position);
    for link in metadata.linked_axes {
        let mut ids = Vec::with_capacity(link.axes.len());
        for idx in link.axes {
            let id = decoded_ids
                .get(idx)
                .ok_or("Invalid linked axes index in subplot container")?;
            if !ids
                .iter()
                .any(|u: &crate::ObjectIdentity| u.same_object(id))
            {
                ids.push(id.clone());
            }
        }
        if ids.len() >= 2 && (link.x || link.y) {
            layout.linked_axes.push(AxisLink {
                axes: ids,
                x: link.x,
                y: link.y,
            });
        }
    }
    slice_viewer_file::restore(
        metadata.slice_viewers,
        viewer_payload,
        &mut figure,
        &decoded_ids,
    )?;
    figure.validate()?;
    Ok(figure)
}
