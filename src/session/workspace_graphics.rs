//! Checkpoint-local identity references; process addresses never reach disk.
use super::*;
use crate::session::figures::{FigureManifest, FigureReference, FigureSession};
use crate::{GraphicsHandle, GraphicsKind, ObjectIdentity, plot::Position};

pub(super) const MAX_HANDLES: usize = 65_536;
const MAX_IDENTITIES: usize = MAX_HANDLES * 3;
pub(super) const HANDLE_BYTES: usize = std::mem::size_of::<GraphicsHandle>()
    + std::mem::size_of::<Arc<GraphicsHandle>>()
    + 8 * std::mem::size_of::<usize>();
// Conservative logical allowance for simultaneously live table, indexing maps,
// restored tokens/handles and per-node reference arrays. This is an allocation
// planning budget, not a claim about allocator overhead or process RSS.
const METADATA_BYTES: usize = 128 * 1024 * 1024;
fn bounded_reserve<T>(values: &mut Vec<T>, limit: usize) -> Result<()> {
    if values.len() >= limit {
        return Err("Graphics metadata count exceeds limit".into());
    }
    if values.len() == values.capacity() {
        let target = values.capacity().saturating_mul(2).max(64).min(limit);
        values
            .try_reserve_exact(target - values.len())
            .map_err(|_| "Cannot allocate graphics recovery metadata")?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Location {
    Figure {},
    Axes {
        position: Position,
    },
    Series {
        position: Position,
        index: usize,
    },
    TextBox {
        index: usize,
    },
    Legend {
        position: Position,
    },
    Baseline {
        position: Position,
        index: usize,
        latent: bool,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Address {
    figure: u32,
    location: Location,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fingerprint {
    file: String,
    bytes: u64,
    crc32: u32,
}
impl From<&FigureReference> for Fingerprint {
    fn from(r: &FigureReference) -> Self {
        Self {
            file: r.file.clone(),
            bytes: r.bytes,
            crc32: r.crc32,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Kind {
    Figure,
    Axes,
    Series,
    TextBox,
    Legend,
    Baseline,
}
impl From<GraphicsKind> for Kind {
    fn from(k: GraphicsKind) -> Self {
        match k {
            GraphicsKind::Figure => Self::Figure,
            GraphicsKind::Axes => Self::Axes,
            GraphicsKind::Series => Self::Series,
            GraphicsKind::TextBox => Self::TextBox,
            GraphicsKind::Legend => Self::Legend,
            GraphicsKind::Baseline => Self::Baseline,
        }
    }
}
impl From<Kind> for GraphicsKind {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Figure => Self::Figure,
            Kind::Axes => Self::Axes,
            Kind::Series => Self::Series,
            Kind::TextBox => Self::TextBox,
            Kind::Legend => Self::Legend,
            Kind::Baseline => Self::Baseline,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Handle {
    kind: Kind,
    figure: usize,
    axes: Option<usize>,
    object: usize,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    // None is a detached token. Repeated references to it retain identity.
    identities: Vec<Option<Address>>,
    handles: Vec<Handle>,
    figures: BTreeMap<u32, Fingerprint>,
}

/// Constructed only by the actual figure recovery reader. Carries fresh model
/// identities, not samples or caller-supplied guesses about figure numbers.
#[derive(Clone, Default)]
pub struct FigureBindings {
    expected: BTreeMap<u32, Fingerprint>,
    available: FigureSession,
    valid: bool,
}
impl FigureBindings {
    /// A failed Engine figure installation cannot make any object live.
    pub fn detach_all(&mut self) {
        self.available = FigureSession::default();
    }
    pub(crate) fn empty() -> Self {
        Self {
            valid: true,
            ..Default::default()
        }
    }
    pub(crate) fn recovered(
        figures: &FigureSession,
        manifest: &FigureManifest,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        let expected = fingerprints(manifest)?;
        check_cancel(cancel)?;
        // Index lazily only if the workspace actually contains Graphics.
        Ok(Self {
            expected,
            available: figures.clone(),
            valid: true,
        })
    }
}

fn fingerprints(manifest: &FigureManifest) -> Result<BTreeMap<u32, Fingerprint>> {
    if manifest.error.is_some() || manifest.version != 1 || manifest.entries.len() > 16 {
        return Err("Invalid figure binding manifest".into());
    }
    let mut result = BTreeMap::new();
    let mut bytes = 0u64;
    for r in &manifest.entries {
        if !(1..=1_000_000).contains(&r.id)
            || r.bytes > 17_000_000
            || !crate::session::figures::owned_name(&r.file)
            || result.insert(r.id, Fingerprint::from(r)).is_some()
        {
            return Err("Invalid or duplicate figure binding reference".into());
        }
        bytes = bytes
            .checked_add(r.bytes)
            .ok_or("Figure binding size overflow")?;
    }
    if bytes > 256 * 1024 * 1024 {
        return Err("Figure binding generation exceeds limit".into());
    }
    Ok(result)
}
// Model traversal is bounded independently of handle-array size. A map lookup
// per identity avoids repeatedly scanning every figure for every handle.
fn index(
    figures: &FigureSession,
    cancel: &AtomicBool,
) -> Result<BTreeMap<usize, (Address, ObjectIdentity)>> {
    // Save reaches here only after SessionStore::stage_figures validates the
    // complete model; restore gets the private reader-created FigureBindings.
    // Do not repeat payload validation (Patch faces / automatic log signs).
    // Defend traversal bounds and semantic address uniqueness independently.
    check_cancel(cancel)?;
    if figures.figures.len() > 16 {
        return Err("Figure recovery exceeds 16 figures".into());
    }
    let mut numbers = BTreeSet::new();
    for doc in &figures.figures {
        checkpoint(cancel)?;
        if !(1..=1_000_000).contains(&doc.id) || !numbers.insert(doc.id) {
            return Err("Invalid or duplicate recovery figure number".into());
        }
        if doc
            .figure
            .layout
            .as_ref()
            .is_some_and(|l| l.others.len() >= 16)
            || doc.figure.textboxes.len() > 64
        {
            return Err("Graphics recovery object count exceeds limit".into());
        }
        let mut positions = BTreeSet::new();
        let mut series = 0usize;
        for (position, axes) in doc.figure.all_axes() {
            checkpoint(cancel)?;
            position.validate()?;
            if !positions.insert(position) || axes.series.len() > 64 || axes.baselines.len() > 64 {
                return Err("Invalid graphics recovery axes or series count".into());
            }
            series += axes.series.len();
            if series > 64 {
                return Err("Graphics recovery series count exceeds limit".into());
            }
        }
    }
    if !numbers.is_empty() && !numbers.contains(&figures.current) {
        return Err("Missing active recovery figure".into());
    }
    let mut result = BTreeMap::new();
    let mut add = |id: &ObjectIdentity, figure, location| -> Result<()> {
        checkpoint(cancel)?;
        if result
            .insert(
                id.recovery_key(),
                (Address { figure, location }, id.clone()),
            )
            .is_some()
        {
            return Err("Ambiguous graphics identity in recovery figures".into());
        }
        Ok(())
    };
    for doc in &figures.figures {
        add(&doc.figure.identity, doc.id, Location::Figure {})?;
        for (position, axes) in doc.figure.all_axes() {
            add(&axes.identity, doc.id, Location::Axes { position })?;
            for (i, series) in axes.series.iter().enumerate() {
                add(
                    &series.identity,
                    doc.id,
                    Location::Series { position, index: i },
                )?;
            }
            for (index, baseline) in axes.baselines.iter().enumerate() {
                add(
                    &baseline.identity,
                    doc.id,
                    Location::Baseline {
                        position,
                        index,
                        latent: false,
                    },
                )?;
            }
            let mut latent_seen = BTreeSet::new();
            for (index, series) in axes.series.iter().enumerate() {
                if matches!(
                    series.kind,
                    crate::plot::PlotKind::Bar | crate::plot::PlotKind::Stem
                ) && axes.baseline_for_series(&series.identity).is_none()
                    && latent_seen.insert(series.baseline_identity.recovery_key())
                {
                    add(
                        &series.baseline_identity,
                        doc.id,
                        Location::Baseline {
                            position,
                            index,
                            latent: true,
                        },
                    )?;
                }
            }
            if let Some(legend) = &axes.legend_state {
                add(&legend.identity, doc.id, Location::Legend { position })?;
            }
        }
        for (i, textbox) in doc.figure.textboxes.iter().enumerate() {
            add(&textbox.identity, doc.id, Location::TextBox { index: i })?;
        }
    }
    Ok(result)
}

pub(super) struct Planner {
    pub table: Table,
    live: BTreeMap<usize, (Address, ObjectIdentity)>,
    tokens: BTreeMap<usize, usize>,
    handles: BTreeMap<Handle, usize>,
    slots: usize,
}
impl Planner {
    pub fn new(
        figures: &FigureSession,
        manifest: &FigureManifest,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        let live = index(figures, cancel)?;
        let expected = fingerprints(manifest)?;
        if figures.figures.len() != expected.len()
            || figures
                .figures
                .iter()
                .any(|f| !expected.contains_key(&f.id))
        {
            return Err("Graphics recovery requires a complete matching figure snapshot".into());
        }
        Ok(Self {
            table: Table {
                figures: expected,
                ..Default::default()
            },
            live,
            tokens: BTreeMap::new(),
            handles: BTreeMap::new(),
            slots: 0,
        })
    }
    pub fn add_slots(&mut self, count: usize) -> Result<()> {
        self.slots = self
            .slots
            .checked_add(count)
            .ok_or("Graphics slot count overflow")?;
        self.table.validate_budget(self.slots)
    }
    fn token(&mut self, id: &ObjectIdentity) -> Result<usize> {
        let key = id.recovery_key();
        if let Some(&slot) = self.tokens.get(&key) {
            return Ok(slot);
        }
        if self.table.identities.len() == MAX_IDENTITIES {
            return Err("Graphics recovery identity limit exceeded".into());
        }
        bounded_reserve(&mut self.table.identities, MAX_IDENTITIES)?;
        let slot = self.table.identities.len();
        self.table
            .identities
            .push(self.live.get(&key).map(|(address, _)| address.clone()));
        self.tokens.insert(key, slot);
        self.table.validate_budget(self.slots)?;
        Ok(slot)
    }
    pub fn handle(&mut self, h: &GraphicsHandle, cancel: &AtomicBool) -> Result<usize> {
        checkpoint(cancel)?;
        let record = Handle {
            kind: h.kind.into(),
            figure: self.token(&h.figure)?,
            axes: h.axes.as_ref().map(|a| self.token(a)).transpose()?,
            object: self.token(&h.object)?,
        };
        if let Some(&slot) = self.handles.get(&record) {
            return Ok(slot);
        }
        if self.table.handles.len() == MAX_HANDLES {
            return Err("Graphics recovery exceeds65536 unique handles".into());
        }
        bounded_reserve(&mut self.table.handles, MAX_HANDLES)?;
        let slot = self.table.handles.len();
        self.table.handles.push(record.clone());
        self.handles.insert(record, slot);
        self.table.validate_budget(self.slots)?;
        Ok(slot)
    }
}

impl Table {
    pub(super) fn uses_baselines(&self) -> bool {
        self.handles
            .iter()
            .take(MAX_HANDLES + 1)
            .any(|h| h.kind == Kind::Baseline)
            || self
                .identities
                .iter()
                .take(MAX_IDENTITIES + 1)
                .flatten()
                .any(|a| matches!(a.location, Location::Baseline { .. }))
    }
    pub fn len(&self) -> usize {
        self.handles.len()
    }
    pub fn validate_budget(&self, slots: usize) -> Result<()> {
        // Bounds are checked before arithmetic; factors include both planner
        // and restore representations and conservative tree-node allowances.
        if slots > LIMIT / HANDLE_BYTES
            || self.identities.len() > MAX_IDENTITIES
            || self.handles.len() > MAX_HANDLES
        {
            return Err("Graphics recovery allocation limit exceeded".into());
        }
        let bytes = self.identities.len() * 384 + self.handles.len() * 384 + slots * 24;
        if bytes > METADATA_BYTES {
            return Err("Graphics recovery metadata exceeds128 MiB budget".into());
        }
        Ok(())
    }
    pub fn validate(&self, cancel: &AtomicBool) -> Result<()> {
        if self.handles.len() > MAX_HANDLES
            || self.identities.len() > MAX_IDENTITIES
            || self.figures.len() > 16
        {
            return Err("Graphics recovery table exceeds metadata limits".into());
        }
        let mut addresses = BTreeSet::new();
        for address in self.identities.iter().flatten() {
            checkpoint(cancel)?;
            if !self.figures.contains_key(&address.figure) || !addresses.insert(address) {
                return Err("Missing or duplicate graphics object binding".into());
            }
            match address.location {
                Location::Axes { position } | Location::Legend { position } => {
                    position.validate()?
                }
                Location::Series { position, index }
                | Location::Baseline {
                    position, index, ..
                } => {
                    position.validate()?;
                    if index >= 64 {
                        return Err("Invalid recovered series index".into());
                    }
                }
                Location::TextBox { index } if index >= 64 => {
                    return Err("Invalid recovered textbox index".into());
                }
                _ => {}
            }
        }
        // Reuse reference checks, including names and aggregate generation size.
        let manifest = FigureManifest {
            version: 1,
            current: 1,
            error: None,
            entries: self
                .figures
                .iter()
                .map(|(&id, f)| FigureReference {
                    window: Default::default(),
                    id,
                    file: f.file.clone(),
                    bytes: f.bytes,
                    crc32: f.crc32,
                    path: None,
                    source_version: None,
                    dirty: false,
                    view: None,
                })
                .collect(),
        };
        fingerprints(&manifest)?;
        let mut used = vec![false; self.identities.len()];
        let mut unique = BTreeSet::new();
        for h in &self.handles {
            checkpoint(cancel)?;
            if !unique.insert(h) {
                return Err("Duplicate graphics handle record".into());
            }
            let mut take = |id: usize| -> Result<Option<&Address>> {
                let value = self
                    .identities
                    .get(id)
                    .ok_or("Invalid graphics identity reference")?;
                used[id] = true;
                Ok(value.as_ref())
            };
            let figure = take(h.figure)?;
            let axes = h.axes.map(&mut take).transpose()?.flatten();
            let object = take(h.object)?;
            if (matches!(
                h.kind,
                Kind::Axes | Kind::Series | Kind::Legend | Kind::Baseline
            )) != h.axes.is_some()
                || (h.kind == Kind::Figure && h.object != h.figure)
                || (h.kind == Kind::Axes && Some(h.object) != h.axes)
            {
                return Err("Invalid recovered handle parent structure".into());
            }
            if figure.is_some_and(|a| a.location != (Location::Figure {})) {
                return Err("Invalid handle figure identity".into());
            }
            if let Some(a) = axes
                && (!matches!(a.location, Location::Axes { .. })
                    || figure.is_none_or(|f| f.figure != a.figure))
            {
                return Err("Invalid handle axes parent binding".into());
            }
            if let Some(a) = object {
                if figure.is_none_or(|f| f.figure != a.figure) {
                    return Err("Invalid handle object parent binding".into());
                }
                let kind = match a.location {
                    Location::Figure {} => Kind::Figure,
                    Location::Axes { .. } => Kind::Axes,
                    Location::Series { .. } => Kind::Series,
                    Location::TextBox { .. } => Kind::TextBox,
                    Location::Legend { .. } => Kind::Legend,
                    Location::Baseline { .. } => Kind::Baseline,
                };
                if kind != h.kind {
                    return Err("Graphics handle kind mismatch".into());
                }
                if let Location::Axes { position }
                | Location::Series { position, .. }
                | Location::Legend { position }
                | Location::Baseline { position, .. } = a.location
                    && axes.is_none_or(|parent| parent.location != (Location::Axes { position }))
                {
                    return Err("Graphics handle axes mismatch".into());
                }
            }
        }
        if used.iter().any(|used| !*used) {
            return Err("Unreferenced graphics identity".into());
        }
        check_cancel(cancel)
    }
    pub fn restore(
        &self,
        bindings: &FigureBindings,
        cancel: &AtomicBool,
    ) -> Result<(Vec<Arc<GraphicsHandle>>, usize)> {
        self.validate(cancel)?;
        if !bindings.valid {
            return Err("Graphics recovery requires a valid figure restoration context".into());
        }
        // All generation references must match, even when an individual file
        // could not be decoded. No binding to a default/reused figure number.
        if self.figures != bindings.expected {
            return Err("Graphics and figure checkpoints do not match".into());
        }
        let available: BTreeMap<_, _> = index(&bindings.available, cancel)?.into_values().collect();
        let mut identities = Vec::new();
        identities
            .try_reserve_exact(self.identities.len())
            .map_err(|_| "Cannot allocate recovered identities")?;
        let mut missing = BTreeSet::new();
        for address in &self.identities {
            checkpoint(cancel)?;
            let token = if let Some(address) = address {
                if let Some(id) = available.get(address) {
                    id.clone()
                } else if available.contains_key(&Address {
                    figure: address.figure,
                    location: Location::Figure {},
                }) {
                    return Err("Recovered figure is missing a referenced graphics object".into());
                } else {
                    missing.insert(address.figure);
                    ObjectIdentity::default()
                }
            } else {
                ObjectIdentity::default()
            };
            identities.push(token);
        }
        let mut handles = Vec::new();
        handles
            .try_reserve_exact(self.handles.len())
            .map_err(|_| "Cannot allocate recovered handles")?;
        for h in &self.handles {
            checkpoint(cancel)?;
            handles.push(Arc::new(GraphicsHandle {
                kind: h.kind.into(),
                figure: identities[h.figure].clone(),
                axes: h.axes.map(|a| identities[a].clone()),
                object: identities[h.object].clone(),
            }));
        }
        Ok((handles, missing.len()))
    }
}

#[cfg(test)]
#[path = "workspace_graphics_tests.rs"]
mod tests;
