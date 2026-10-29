//! Bounded MFIG6/7/8/A/C/E/G/I container of single-axes records. Nested containers
//! are rejected before decoding, and all axes share the figure-wide limits.
use super::*;
use crate::plot::{Layout, Panel, Position};

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    textboxes: Vec<crate::plot::TextBox>,
    active: usize,
    positions: Vec<Position>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tiled: Option<crate::plot::TileGrid>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    title: String,
}
pub(super) fn encode(figure: &Figure) -> Result<Vec<u8>> {
    let mut axes: Vec<_> = figure.all_axes().collect();
    axes.sort_by_key(|(p, _)| *p);
    let metadata = Metadata {
        appearance: figure.appearance,
        invert_hardcopy: figure.invert_hardcopy,
        name: figure.name.clone(),
        color: figure.color,
        textboxes: figure.textboxes.clone(),
        active: axes
            .iter()
            .position(|(p, _)| *p == figure.active_position())
            .unwrap_or(0),
        positions: axes.iter().map(|(p, _)| *p).collect(),
        tiled: figure.layout.as_ref().and_then(|l| l.tiled),
        title: figure
            .layout
            .as_ref()
            .map_or_else(String::new, |l| l.title.clone()),
    };
    let json = serde_json::to_vec(&metadata).map_err(|e| e.to_string())?;
    if json.len() > 65536 {
        return Err("Figure exceeds metadata save limit".into());
    }
    let mut bytes = Vec::with_capacity(16 * figure.point_count() + 4096);
    bytes.extend_from_slice(if figure.has_static_style_metadata() {
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
    for (_, axes) in axes {
        let payload = encode_native_figure(&Figure {
            identity: crate::ObjectIdentity::default(),
            axes: axes.clone(),
            layout: None,
            textboxes: Vec::new(),

            name: String::new(),
            color: [255; 3],
            appearance: None,
            invert_hardcopy: None,
        })?;
        if bytes.len() + 4 + payload.len() > 17_000_000 {
            return Err("Figure exceeds 17 MB save limit".into());
        }
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&payload);
    }
    Ok(bytes)
}
pub(super) fn decode(bytes: &[u8]) -> Result<Figure> {
    if bytes.len() < 12 || bytes.len() > 17_000_000 {
        return Err("Invalid subplot container length".into());
    }
    let size = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    if size > 65536 || size > bytes.len() - 12 {
        return Err("Invalid subplot metadata length".into());
    }
    let metadata: Metadata =
        serde_json::from_slice(&bytes[12..12 + size]).map_err(|e| e.to_string())?;
    if metadata.invert_hardcopy.is_some() && !bytes.starts_with(b"MATRFIGY") {
        return Err("Subplot print preference requires MFIGY".into());
    }
    if metadata
        .appearance
        .is_some_and(|a| a.series_palette.is_some())
        && !matches!(&bytes[..8], b"MATRFIGW" | b"MATRFIGY")
    {
        return Err("Subplot automatic palette metadata requires MFIGW".into());
    }
    if metadata.appearance.is_some()
        && !matches!(
            &bytes[..8],
            b"MATRFIGS" | b"MATRFIGU" | b"MATRFIGW" | b"MATRFIGY"
        )
    {
        return Err("Subplot appearance intent requires MFIGS".into());
    }
    if !matches!(
        &bytes[..8],
        b"MATRFIGK"
            | b"MATRFIGM"
            | b"MATRFIGO"
            | b"MATRFIGQ"
            | b"MATRFIGS"
            | b"MATRFIGU"
            | b"MATRFIGW"
            | b"MATRFIGY"
    ) && (!metadata.textboxes.is_empty()
        || !metadata.name.is_empty()
        || metadata.color != [255; 3])
    {
        return Err("Advanced subplot metadata requires MFIGK".into());
    }
    if bytes.starts_with(b"MATRFIG6") && (metadata.tiled.is_some() || !metadata.title.is_empty()) {
        return Err("MFIG6 does not contain tiled layout metadata".into());
    }
    if !matches!(
        &bytes[..8],
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
    let mut figure = Figure {
        textboxes: metadata.textboxes,
        name: metadata.name,
        color: metadata.color,
        appearance: metadata.appearance,
        invert_hardcopy: metadata.invert_hardcopy,
        layout: Some(Layout {
            active: if empty {
                let grid = metadata.tiled.unwrap();
                grid.validate()?;
                Position::from_grid(grid.rows, grid.columns, &[1])?
            } else {
                metadata.positions[metadata.active]
            },
            others: Vec::new(),
            tiled: metadata.tiled,
            title: metadata.title,
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
    // U's complete style ledger is admitted before any child's sample allocation.
    // Old containers retain their existing validation/error precedence.
    if matches!(&bytes[..8], b"MATRFIGU" | b"MATRFIGW" | b"MATRFIGY") {
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
            if length > 65536 + crate::plot::MAX_SERIES * 4096 {
                return Err("Invalid figure metadata size".into());
            }
            let json = child
                .get(12..12 + length)
                .ok_or("Invalid figure metadata size")?;
            let metadata: ColorsOnly = serde_json::from_slice(json).map_err(|e| e.to_string())?;
            if metadata.invert_hardcopy.is_some() {
                return Err(
                    "InvertHardcopy belongs to the figure, not an embedded axes record".into(),
                );
            }
            if child.starts_with(b"MATRFIGX") && !bytes.starts_with(b"MATRFIGY") {
                return Err("Subplot legend RGB capability requires MFIGY".into());
            }
            if metadata.axes_colors.is_some_and(|c| c.legend.is_some())
                && (!bytes.starts_with(b"MATRFIGY") || !child.starts_with(b"MATRFIGX"))
            {
                return Err(
                    "Subplot legend RGB metadata requires MFIGY with MFIGX children".into(),
                );
            }
            if matches!(&bytes[..8], b"MATRFIGW" | b"MATRFIGY") {
                if matches!(
                    &child[..8],
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
                ) {
                    return Err("Nested subplot containers are unsupported".into());
                }
                let palette: FigureMetadata =
                    serde_json::from_slice(json).map_err(|e| e.to_string())?;
                if bytes.starts_with(b"MATRFIGY")
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
                if has && !matches!(&child[..8], b"MATRFIGV" | b"MATRFIGX") {
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
                if !matches!(&child[..8], b"MATRFIGT" | b"MATRFIGV" | b"MATRFIGX") {
                    return Err("Axes RGB metadata requires MFIGT".into());
                }
                colors.validate()?;
            }
        }
    }
    let mut offset = 12 + size;
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
        if child.starts_with(b"MATRFIG6")
            || child.starts_with(b"MATRFIG7")
            || child.starts_with(b"MATRFIG8")
            || child.starts_with(b"MATRFIGA")
            || child.starts_with(b"MATRFIGC")
            || child.starts_with(b"MATRFIGE")
            || child.starts_with(b"MATRFIGG")
            || child.starts_with(b"MATRFIGI")
            || child.starts_with(b"MATRFIGK")
            || child.starts_with(b"MATRFIGM")
            || child.starts_with(b"MATRFIGO")
            || child.starts_with(b"MATRFIGQ")
            || child.starts_with(b"MATRFIGS")
            || child.starts_with(b"MATRFIGU")
            || child.starts_with(b"MATRFIGW")
            || child.starts_with(b"MATRFIGY")
        {
            return Err("Nested subplot containers are unsupported".into());
        }
        if !matches!(
            &bytes[..8],
            b"MATRFIGI"
                | b"MATRFIGK"
                | b"MATRFIGM"
                | b"MATRFIGO"
                | b"MATRFIGQ"
                | b"MATRFIGS"
                | b"MATRFIGU"
                | b"MATRFIGW"
                | b"MATRFIGY"
        ) && child.starts_with(b"MATRFIGH")
        {
            return Err("Extended subplot legend/axes layout requires MFIGI".into());
        }
        if !matches!(
            &bytes[..8],
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
        ) && child.starts_with(b"MATRFIGF")
        {
            return Err("Subplot LineStyle none requires MFIGG".into());
        }
        if !matches!(
            &bytes[..8],
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
        ) && child.starts_with(b"MATRFIGD")
        {
            return Err("Subplot legend locations require MFIGE".into());
        }
        if !matches!(
            &bytes[..8],
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
        ) && child.starts_with(b"MATRFIGB")
        {
            return Err("Marker style metadata requires MFIGC".into());
        }
        if !matches!(
            &bytes[..8],
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
        ) && child.starts_with(b"MATRFIG9")
        {
            return Err("Explicit subplot line widths require MFIGA".into());
        }
        if !matches!(
            &bytes[..8],
            b"MATRFIGK"
                | b"MATRFIGM"
                | b"MATRFIGO"
                | b"MATRFIGQ"
                | b"MATRFIGS"
                | b"MATRFIGU"
                | b"MATRFIGW"
                | b"MATRFIGY"
        ) && child.starts_with(b"MATRFIGJ")
        {
            return Err("Advanced subplot metadata requires MFIGK".into());
        }
        if !matches!(
            &bytes[..8],
            b"MATRFIGO" | b"MATRFIGQ" | b"MATRFIGS" | b"MATRFIGU" | b"MATRFIGW" | b"MATRFIGY"
        ) && child.starts_with(b"MATRFIGN")
        {
            return Err("Compact subplot coordinates require MFIGO".into());
        }
        if !matches!(
            &bytes[..8],
            b"MATRFIGM"
                | b"MATRFIGO"
                | b"MATRFIGQ"
                | b"MATRFIGS"
                | b"MATRFIGU"
                | b"MATRFIGW"
                | b"MATRFIGY"
        ) && child.starts_with(b"MATRFIGL")
        {
            return Err("Independent subplot face outlines require MFIGM".into());
        }
        if !matches!(
            &bytes[..8],
            b"MATRFIGQ" | b"MATRFIGS" | b"MATRFIGU" | b"MATRFIGW" | b"MATRFIGY"
        ) && child.starts_with(b"MATRFIGP")
        {
            return Err("Independent subplot baselines require MFIGQ".into());
        }
        if child.starts_with(b"MATRFIGT")
            && !matches!(&bytes[..8], b"MATRFIGU" | b"MATRFIGW" | b"MATRFIGY")
        {
            return Err("Subplot axes RGB capability requires MFIGU".into());
        }
        if child.starts_with(b"MATRFIGR")
            && !matches!(
                &bytes[..8],
                b"MATRFIGS" | b"MATRFIGU" | b"MATRFIGW" | b"MATRFIGY"
            )
        {
            return Err("Subplot appearance capability requires MFIGS".into());
        }
        if child.starts_with(b"MATRFIGV") && !matches!(&bytes[..8], b"MATRFIGW" | b"MATRFIGY") {
            return Err("Subplot palette capability requires MFIGW".into());
        }
        if child.starts_with(b"MATRFIGX") && !bytes.starts_with(b"MATRFIGY") {
            return Err("Subplot legend RGB capability requires MFIGY".into());
        }
        let decoded = decode_native_figure_with_ledger(child, figure.geometry_ledger()?)?;
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
    figure
        .layout
        .as_mut()
        .unwrap()
        .others
        .sort_by_key(|p| p.position);
    Ok(figure)
}
