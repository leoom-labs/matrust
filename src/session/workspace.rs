//! Incremental workspace checkpoints: streamed little-endian payloads, shared
//! aliases and weak writer caches. The session JSON is the commit point.
use crate::{Array, LogicalMatrix, Matrix, Result, dsp::check_cancel, files};
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Weak, atomic::AtomicBool},
};
#[path = "workspace_graph.rs"]
mod graph;
#[path = "workspace_graphics.rs"]
mod graphics;
pub use graphics::FigureBindings;
const LIMIT: usize = 256 * 1024 * 1024;
const VARIABLES: usize = 10000;
const INLINE_BYTES: usize = 64;
type Variables = BTreeMap<String, Array>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<graph::Graph>,
    pub entries: BTreeMap<String, Reference>,
    /// Variables left out of the checkpoint because they hold a struct.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
    #[serde(skip)]
    pub error: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inline: Option<Vec<u8>>,
    pub file: String,
    pub bytes: usize,
    pub crc32: u32,
    pub rows: usize,
    pub cols: usize,
    pub logical: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub complex: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub character: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numeric_class: Option<crate::NumericClass>,
}
fn is_false(value: &bool) -> bool {
    !*value
}
pub(super) fn recover_manifest<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Manifest>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_value(value).unwrap_or_else(|e| {
        Manifest {
            version: 1,
            graph: None,
            entries: BTreeMap::new(),
            skipped: Vec::new(),
            error: Some(format!("Cannot read workspace recovery manifest: {e}")),
        }
    })))
}
fn owned(name: &str) -> bool {
    name.len() == 26
        && name.starts_with("array-")
        && name.ends_with(".bin")
        && name[6..22].bytes().all(|b| b.is_ascii_alphanumeric())
}
pub fn directory(session: &Path) -> Result<PathBuf> {
    let path = std::path::absolute(session).map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .ok_or("Invalid session filename")?
        .to_string_lossy();
    Ok(path.with_file_name(format!("{name}.workspace")))
}
fn checked_directory(session: &Path, create: bool) -> Result<PathBuf> {
    let dir = directory(session)?;
    if create {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    let parent = dir
        .parent()
        .ok_or("Invalid workspace recovery directory")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let actual = dir
        .canonicalize()
        .map_err(|e| format!("Cannot open workspace recovery: {e}"))?;
    if actual != parent.join(dir.file_name().unwrap()) {
        return Err("Workspace recovery directory is redirected".into());
    }
    Ok(actual)
}
fn validate_name(name: &str) -> Result<()> {
    if !files::valid_name(name) || crate::syntax::keyword(name) {
        return Err("Invalid workspace recovery variable name".into());
    }
    Ok(())
}
impl Manifest {
    /// Command-window line naming the variables this checkpoint could not hold.
    pub fn skipped_note(&self) -> Option<String> {
        const SHOWN: usize = 20;
        if self.skipped.is_empty() {
            return None;
        }
        let mut names = self.skipped[..self.skipped.len().min(SHOWN)].join(", ");
        if self.skipped.len() > SHOWN {
            names.push_str(&format!(" and {} more", self.skipped.len() - SHOWN));
        }
        Some(format!(
            "Not recovered (structs are not supported in recovery yet): {names}"
        ))
    }
    fn validate(&self) -> Result<()> {
        self.validate_cancel(&AtomicBool::new(false))
    }
    fn validate_cancel(&self, cancel: &AtomicBool) -> Result<()> {
        check_cancel(cancel)?;
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        if !matches!(self.version, 1..=7)
            || self.entries.len() > graph::MAX_NODES
            || (self.version < 3 && self.entries.len() > VARIABLES)
            || (self.version >= 3) != self.graph.is_some()
            || (self.version < 4 && self.graph.as_ref().is_some_and(|g| g.graphics.is_some()))
            || (self.version == 4 && !self.graph.as_ref().is_some_and(|g| g.graphics.is_some()))
        {
            return Err("Invalid workspace recovery version or variable count".into());
        }
        if self.skipped.len() > VARIABLES {
            return Err("Invalid workspace recovery skipped-variable count".into());
        }
        for name in &self.skipped {
            check_cancel(cancel)?;
            validate_name(name)?;
        }
        let mut total = 0usize;
        let mut identities = BTreeMap::new();
        for (name, r) in &self.entries {
            check_cancel(cancel)?;
            validate_name(name)?;
            if r.character && (self.version < 3 || r.logical || r.complex) {
                return Err("Invalid character workspace recovery type/version".into());
            }
            if r.complex && (r.logical || !matches!(self.version, 2..=7)) {
                return Err("Invalid complex workspace recovery type/version".into());
            }
            if r.numeric_class.is_some()
                && (self.version != 7 || r.character || r.complex || r.logical)
            {
                return Err("Invalid typed workspace recovery type/version".into());
            }
            let len = if r.character {
                graph::character_len(r.rows, r.cols)?
            } else if r.complex {
                crate::ComplexMatrix::checked_len(r.rows, r.cols)?
            } else {
                Matrix::checked_len(r.rows, r.cols)?
            };
            if len
                * if let Some(class) = r.numeric_class {
                    class.size()
                } else if r.character {
                    2
                } else if r.complex {
                    16
                } else if r.logical {
                    1
                } else {
                    8
                }
                != r.bytes
            {
                return Err("Invalid workspace recovery reference".into());
            }
            if let Some(bytes) = &r.inline {
                let mut crc = flate2::Crc::new();
                crc.update(bytes);
                if !r.file.is_empty()
                    || bytes.len() > INLINE_BYTES
                    || bytes.len() != r.bytes
                    || crc.sum() != r.crc32
                {
                    return Err("Invalid inline workspace recovery payload".into());
                }
            } else if !owned(&r.file) {
                return Err("Invalid workspace recovery reference".into());
            }
            total = total
                .checked_add(r.bytes)
                .ok_or("Workspace recovery size overflow")?;
            if total > LIMIT {
                return Err("Workspace recovery exceeds 256 MiB logical storage".into());
            }
            let identity = (
                r.bytes,
                r.crc32,
                r.logical,
                r.complex,
                r.character,
                r.numeric_class,
            );
            if r.inline.is_none()
                && identities
                    .insert(&r.file, identity)
                    .is_some_and(|old| old != identity)
            {
                return Err("Conflicting workspace recovery references".into());
            }
        }
        if let Some(graph) = &self.graph {
            graph.validate(&self.entries, cancel)?;
        }
        let shaped_characters = self
            .entries
            .values()
            .any(|r| r.character && !graph::canonical_character_shape(r.rows, r.cols))
            || match &self.graph {
                Some(g) => g.shaped_characters(
                    |name| self.entries.get(name).is_some_and(|r| r.character),
                    cancel,
                )?,
                None => false,
            };
        let baselines = self
            .graph
            .as_ref()
            .and_then(|g| g.graphics.as_ref())
            .is_some_and(graphics::Table::uses_baselines);
        let typed = self.entries.values().any(|r| r.numeric_class.is_some());
        if (self.version == 7) != typed
            || (self.version < 7
                && ((self.version == 6) != baselines
                    || (!baselines && (self.version == 5) != shaped_characters)))
            || (shaped_characters && self.version < 5)
        {
            return Err(
                "Character shapes require workspace recovery version 5; baseline handles require workspace recovery version 6".into(),
            );
        }
        if self
            .entries
            .values()
            .filter(|r| r.inline.is_none())
            .map(|r| &r.file)
            .collect::<BTreeSet<_>>()
            .len()
            > VARIABLES
        {
            return Err("Workspace recovery exceeds 10000 leaf files".into());
        }
        check_cancel(cancel)
    }
}
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum PayloadKey {
    File(String),
    Inline(u8, Vec<u8>),
}
/// All-or-nothing decode; aliases and reshaped aliases share the recovered buffer.
pub fn restore(path: &Path, manifest: &Manifest, cancel: &AtomicBool) -> Result<Variables> {
    restore_impl(path, manifest, None, cancel).map(|r| r.values)
}
pub struct WorkspaceRestoration {
    pub values: Variables,
    pub warnings: Vec<String>,
}
pub fn restore_with_figures(
    path: &Path,
    manifest: &Manifest,
    bindings: &FigureBindings,
    cancel: &AtomicBool,
) -> Result<WorkspaceRestoration> {
    restore_impl(path, manifest, Some(bindings), cancel)
}
fn restore_impl(
    path: &Path,
    manifest: &Manifest,
    bindings: Option<&FigureBindings>,
    cancel: &AtomicBool,
) -> Result<WorkspaceRestoration> {
    manifest.validate_cancel(cancel)?;
    check_cancel(cancel)?;

    let (handles, missing) =
        if let Some(table) = manifest.graph.as_ref().and_then(|g| g.graphics.as_ref()) {
            table.restore(
                bindings.ok_or("Graphics recovery requires figure restoration context")?,
                cancel,
            )?
        } else {
            (vec![], 0)
        };
    let dir = manifest
        .entries
        .values()
        .any(|r| r.inline.is_none())
        .then(|| checked_directory(path, false))
        .transpose()?;
    let mut decoded: BTreeMap<PayloadKey, Array> = BTreeMap::new();
    let mut result = BTreeMap::new();
    for (name, r) in &manifest.entries {
        check_cancel(cancel)?;
        let key = r.inline.as_ref().map_or_else(
            || PayloadKey::File(r.file.clone()),
            |b| {
                PayloadKey::Inline(
                    if let Some(class) = r.numeric_class {
                        6 + class as u8
                    } else if r.character {
                        3
                    } else if r.complex {
                        2
                    } else {
                        u8::from(r.logical)
                    },
                    b.clone(),
                )
            },
        );
        let value = if let Some(array) = decoded.get(&key) {
            match array {
                Array::Numeric(a) => a.reshape(r.rows, r.cols)?.into(),
                Array::Typed(a) => a.reshaped(r.rows, r.cols)?.into(),
                Array::Logical(a) => a.reshape(r.rows, r.cols)?.into(),
                Array::Complex(a) => a.reshape(r.rows, r.cols, cancel)?.into(),
                Array::Character(a) => a.reshape(r.rows, r.cols, cancel)?.into(),
                _ => return Err("Unsupported or conflicting recovery leaf type".into()),
            }
        } else {
            let mut file: Box<dyn Read + '_> = if let Some(bytes) = &r.inline {
                Box::new(std::io::Cursor::new(bytes.as_slice()))
            } else {
                let path = dir.as_ref().unwrap().join(&r.file);
                let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                if !meta.file_type().is_file() || meta.len() != r.bytes as u64 {
                    return Err("Workspace recovery file type or length mismatch".into());
                }
                Box::new(File::open(path).map_err(|e| e.to_string())?)
            };
            let mut crc = flate2::Crc::new();
            let mut read = |bytes: &mut [u8]| -> Result<()> {
                for chunk in bytes.chunks_mut(65536) {
                    checkpoint(cancel)?;
                    file.read_exact(chunk).map_err(|e| e.to_string())?;
                    crc.update(chunk);
                }
                Ok(())
            };
            let array: Array = if let Some(class) = r.numeric_class {
                crate::TypedArray::from_le_reader(class, r.rows, r.cols, &mut read)?.into()
            } else if r.character {
                let count = graph::character_len(r.rows, r.cols)?;
                let mut units = Vec::new();
                units
                    .try_reserve_exact(count)
                    .map_err(|_| "Cannot allocate character recovery payload")?;
                let mut buffer = [0u8; 65536];
                for start in (0..count).step_by(32768) {
                    let bytes = &mut buffer[..(count - start).min(32768) * 2];
                    read(bytes)?;
                    units.extend(
                        bytes
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|b| u16::from_le_bytes(*b)),
                    );
                }
                crate::script_containers::CharacterRow::from_column_major(
                    r.rows, r.cols, units, cancel,
                )?
                .into()
            } else if r.complex {
                let count = r.bytes / 16;
                let mut values = Vec::new();
                values
                    .try_reserve_exact(count)
                    .map_err(|_| "Cannot allocate complex recovery payload")?;
                let mut buffer = [0u8; 65536];
                for start in (0..count).step_by(4096) {
                    check_cancel(cancel)?;
                    let bytes = &mut buffer[..(count - start).min(4096) * 16];
                    read(bytes)?;
                    for pair in bytes.as_chunks::<16>().0 {
                        values.push(crate::Complex64::new(
                            f64::from_le_bytes(pair[..8].try_into().unwrap()),
                            f64::from_le_bytes(pair[8..].try_into().unwrap()),
                        ));
                    }
                }
                crate::ComplexMatrix::from_column_vec(r.rows, r.cols, values, cancel)?.into()
            } else if r.logical {
                let mut values = vec![0; r.bytes];
                read(&mut values)?;
                LogicalMatrix::from_column_vec(r.rows, r.cols, values, cancel)?.into()
            } else {
                let mut values = vec![0.0f64; r.bytes / 8];
                read(bytemuck::cast_slice_mut(&mut values))?;
                #[cfg(target_endian = "big")]
                for value in &mut values {
                    *value = f64::from_bits(value.to_bits().swap_bytes());
                }
                Matrix::from_column_vec(r.rows, r.cols, values)?.into()
            };
            let mut extra = [0];
            if crc.sum() != r.crc32 || file.read(&mut extra).map_err(|e| e.to_string())? != 0 {
                return Err("Workspace recovery checksum or length mismatch".into());
            }
            decoded.insert(key, array.clone());
            array
        };
        result.insert(name.clone(), value);
    }
    check_cancel(cancel)?;
    let values = if let Some(graph) = &manifest.graph {
        graph.restore(result, &handles, cancel)?
    } else {
        result
    };
    let warnings = if missing == 0 {
        vec![]
    } else {
        vec![format!(
            "Graphics handles for {missing} unavailable recovered figures remain invalid; variables and aliases were retained."
        )]
    };
    Ok(WorkspaceRestoration { values, warnings })
}

enum WeakArray {
    Typed(crate::typed_array::WeakTypedData),
    Numeric(Weak<nalgebra::DMatrix<f64>>),
    Logical(Weak<Vec<u8>>),
    Complex(Weak<nalgebra::DMatrix<crate::Complex64>>),
    Character(Weak<Vec<u16>>),
}
impl WeakArray {
    fn new(a: &Array) -> Self {
        match a {
            Array::Typed(a) => Self::Typed(a.weak_storage()),
            Array::Numeric(a) => Self::Numeric(Arc::downgrade(&a.storage)),
            Array::Logical(a) => Self::Logical(Arc::downgrade(&a.data)),
            Array::Complex(a) => Self::Complex(Arc::downgrade(&a.storage)),
            Array::Character(a) => Self::Character(a.weak_storage()),
            Array::Cell(_) | Array::Graphics(_) | Array::Struct(_) | Array::Handle(_) => {
                unreachable!("stage preflights supported persistence types")
            }
        }
    }
    fn key(&self) -> (u8, usize) {
        match self {
            Self::Typed(a) => a.key(),
            Self::Numeric(a) => (0, a.as_ptr() as usize),
            Self::Logical(a) => (1, a.as_ptr() as usize),
            Self::Complex(a) => (2, a.as_ptr() as usize),
            Self::Character(a) => (3, a.as_ptr() as usize),
        }
    }
}
struct Cached {
    allocation: WeakArray,
    reference: Reference,
}
pub(super) struct Pending {
    pub manifest: Manifest,
    paths: Vec<PathBuf>,
    cache: BTreeMap<String, Cached>,
    committed: bool,
}
impl Drop for Pending {
    fn drop(&mut self) {
        if !self.committed {
            for p in &self.paths {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}
pub(super) struct Store {
    path: PathBuf,
    cache: BTreeMap<String, Cached>,
    committed: BTreeSet<String>,
    previous: BTreeSet<String>,
}
impl Store {
    pub fn new(path: &Path) -> Self {
        #[derive(Deserialize)]
        struct Index {
            #[serde(default, deserialize_with = "recover_manifest")]
            workspace_manifest: Option<Manifest>,
        }
        let committed = files::read_bounded(path, super::MAX_SESSION_BYTES)
            .ok()
            .and_then(|b| serde_json::from_slice::<Index>(&b).ok())
            .and_then(|i| i.workspace_manifest)
            .filter(|m| m.validate().is_ok())
            .map(|m| {
                m.entries
                    .into_values()
                    .filter(|r| r.inline.is_none())
                    .map(|r| r.file)
                    .collect()
            })
            .unwrap_or_default();
        Self {
            path: path.into(),
            cache: BTreeMap::new(),
            committed,
            previous: BTreeSet::new(),
        }
    }
    #[cfg(test)]
    pub fn stage(&self, values: Variables) -> Result<Pending> {
        self.stage_with_cancel(values, &AtomicBool::new(false))
    }
    #[cfg(test)]
    pub fn stage_with_cancel(&self, values: Variables, cancel: &AtomicBool) -> Result<Pending> {
        self.stage_with_figures(values, None, cancel)
    }
    pub fn stage_with_figures(
        &self,
        values: Variables,
        figures: Option<(
            &super::figures::FigureSession,
            &super::figures::FigureManifest,
        )>,
        cancel: &AtomicBool,
    ) -> Result<Pending> {
        check_cancel(cancel)?;
        if values.len() > VARIABLES
            || values
                .values()
                .try_fold(0usize, |total, a| total.checked_add(a.bytes()))
                .is_none_or(|total| total > LIMIT)
        {
            return Err("Workspace recovery exceeds 10000 variables or 256 MiB".into());
        }
        let (values, graph) = graph::plan(values, figures, cancel)?;
        // Planning recursively rejects unsupported leaves before any directory or file is created.
        let external = values.values().filter(|a| a.bytes() > INLINE_BYTES).count();
        if external > VARIABLES {
            return Err("Workspace recovery exceeds 10000 leaf files".into());
        }
        for name in values.keys() {
            validate_name(name)?;
        }
        let dir = checked_directory(&self.path, true)?;
        cleanup_cancel(
            &dir,
            &self.committed.union(&self.previous).cloned().collect(),
            cancel,
        )?;
        let shaped_characters = values.values().any(|a| matches!(a, Array::Character(c) if !graph::canonical_character_shape(c.rows(), c.cols())))
            || match &graph { Some(g) => g.shaped_characters(|name| matches!(values.get(name), Some(Array::Character(_))), cancel)?, None => false };
        let baselines = graph
            .as_ref()
            .and_then(|g| g.graphics.as_ref())
            .is_some_and(graphics::Table::uses_baselines);
        let version = if values.values().any(|a| matches!(a, Array::Typed(_))) {
            7
        } else if baselines {
            6
        } else if shaped_characters {
            5
        } else if graph.as_ref().is_some_and(|g| g.graphics.is_some()) {
            4
        } else if graph.is_some() {
            3
        } else if values.values().any(Array::is_complex) {
            2
        } else {
            1
        };
        let mut pending = Pending {
            manifest: Manifest {
                version,
                graph,
                entries: BTreeMap::new(),
                skipped: Vec::new(),
                error: None,
            },
            paths: vec![],
            cache: BTreeMap::new(),
            committed: false,
        };
        let mut aliases: BTreeMap<(u8, usize), Reference> = BTreeMap::new();
        for (name, array) in values {
            check_cancel(cancel)?;
            let allocation = WeakArray::new(&array);
            let key = allocation.key();
            let cached = self.cache.get(&name).filter(|c| {
                c.allocation.key() == key
                    && (c.reference.inline.is_some()
                        || std::fs::symlink_metadata(dir.join(&c.reference.file)).is_ok_and(|m| {
                            m.file_type().is_file() && m.len() == c.reference.bytes as u64
                        }))
            });
            let mut reference = if let Some(r) =
                aliases.get(&key).or_else(|| cached.map(|c| &c.reference))
            {
                r.clone()
            } else if array.bytes() <= INLINE_BYTES {
                let bytes = match &array {
                    Array::Character(a) => a.units().iter().flat_map(|v| v.to_le_bytes()).collect(),
                    Array::Numeric(a) => a
                        .data()
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                        .collect::<Vec<_>>(),
                    Array::Logical(a) => a.data().to_vec(),
                    Array::Typed(a) => a.to_le_bytes(),
                    Array::Complex(a) => a
                        .data()
                        .iter()
                        .flat_map(|v| v.re.to_le_bytes().into_iter().chain(v.im.to_le_bytes()))
                        .collect(),
                    Array::Cell(_) | Array::Graphics(_) | Array::Struct(_) | Array::Handle(_) => {
                        unreachable!("stage preflight")
                    }
                };
                let mut crc = flate2::Crc::new();
                crc.update(&bytes);
                Reference {
                    inline: Some(bytes),
                    file: String::new(),
                    bytes: array.bytes(),
                    crc32: crc.sum(),
                    rows: array.rows(),
                    cols: array.cols(),
                    logical: array.is_logical(),
                    complex: array.is_complex(),
                    character: matches!(array, Array::Character(_)),
                    numeric_class: match &array {
                        Array::Typed(a) => Some(a.class()),
                        _ => None,
                    },
                }
            } else {
                let mut file = tempfile::Builder::new()
                    .prefix("array-")
                    .suffix(".bin")
                    .rand_bytes(16)
                    .tempfile_in(&dir)
                    .map_err(|e| e.to_string())?;
                let mut crc = flate2::Crc::new();
                let mut write = |bytes: &[u8]| -> Result<()> {
                    for chunk in bytes.chunks(65536) {
                        checkpoint(cancel)?;
                        file.write_all(chunk).map_err(|e| e.to_string())?;
                        crc.update(chunk);
                    }
                    Ok(())
                };
                match &array {
                    Array::Character(a) => {
                        let mut buffer = [0u8; 65536];
                        for chunk in a.units().chunks(32768) {
                            checkpoint(cancel)?;
                            for (unit, pair) in
                                chunk.iter().zip(buffer.as_chunks_mut::<2>().0.iter_mut())
                            {
                                *pair = unit.to_le_bytes();
                            }
                            write(&buffer[..chunk.len() * 2])?;
                        }
                    }
                    Array::Numeric(a) => {
                        #[cfg(target_endian = "little")]
                        write(bytemuck::cast_slice(a.data()))?;
                        #[cfg(target_endian = "big")]
                        for chunk in a.data().chunks(8192) {
                            let bytes: Vec<_> =
                                chunk.iter().flat_map(|v| v.to_le_bytes()).collect();
                            write(&bytes)?;
                        }
                    }
                    Array::Logical(a) => write(a.data())?,
                    Array::Typed(a) => a.write_le_chunks(&mut write)?,
                    Array::Complex(a) => {
                        let mut buffer = [0u8; 65536];
                        for chunk in a.data().chunks(4096) {
                            for (value, bytes) in
                                chunk.iter().zip(buffer.as_chunks_mut::<16>().0.iter_mut())
                            {
                                bytes[..8].copy_from_slice(&value.re.to_le_bytes());
                                bytes[8..].copy_from_slice(&value.im.to_le_bytes());
                            }
                            write(&buffer[..chunk.len() * 16])?;
                        }
                    }
                    Array::Cell(_) | Array::Graphics(_) | Array::Struct(_) | Array::Handle(_) => {
                        unreachable!("stage preflight")
                    }
                }
                check_cancel(cancel)?;
                file.as_file().sync_all().map_err(|e| e.to_string())?;
                let (_, path) = file.keep().map_err(|e| e.to_string())?;
                pending.paths.push(path.clone());
                Reference {
                    inline: None,
                    file: path
                        .file_name()
                        .unwrap()
                        .to_str()
                        .ok_or("Invalid recovery filename")?
                        .into(),
                    bytes: array.bytes(),
                    crc32: crc.sum(),
                    rows: array.rows(),
                    cols: array.cols(),
                    logical: array.is_logical(),
                    complex: array.is_complex(),
                    character: matches!(array, Array::Character(_)),
                    numeric_class: match &array {
                        Array::Typed(a) => Some(a.class()),
                        _ => None,
                    },
                }
            };
            reference.rows = array.rows();
            reference.cols = array.cols();
            aliases.insert(key, reference.clone());
            pending
                .manifest
                .entries
                .insert(name.clone(), reference.clone());
            pending.cache.insert(
                name,
                Cached {
                    allocation,
                    reference,
                },
            );
        }
        pending.manifest.validate_cancel(cancel)?;
        Ok(pending)
    }
    pub fn commit(&mut self, mut pending: Pending) {
        pending.committed = true;
        self.previous = std::mem::replace(
            &mut self.committed,
            pending
                .manifest
                .entries
                .values()
                .filter(|r| r.inline.is_none())
                .map(|r| r.file.clone())
                .collect(),
        );
        self.cache = std::mem::take(&mut pending.cache);
        if let Ok(dir) = checked_directory(&self.path, false) {
            let _ = cleanup(
                &dir,
                &self.committed.union(&self.previous).cloned().collect(),
            );
        }
    }
}
fn cleanup(dir: &Path, keep: &BTreeSet<String>) -> Result<()> {
    cleanup_cancel(dir, keep, &AtomicBool::new(false))
}
fn cleanup_cancel(dir: &Path, keep: &BTreeSet<String>, cancel: &AtomicBool) -> Result<()> {
    let mut retained = 0u64;
    for (count, entry) in std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .enumerate()
    {
        check_cancel(cancel)?;
        if count >= 30000 {
            return Err("Too many files in workspace recovery directory".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|n| owned(n)) else {
            continue;
        };
        if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            continue;
        }
        if !keep.contains(name) && std::fs::remove_file(entry.path()).is_ok() {
            continue;
        }
        retained += entry.metadata().map_err(|e| e.to_string())?.len();
    }
    if retained > (2 * LIMIT) as u64 {
        return Err("Workspace recovery disk budget exceeded; checkpoint preserved".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn committed_writer_cache_does_not_retain_payload_allocations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        let mut store = Store::new(&path);
        let matrix = Matrix::filled(1024, 1024, 1.).unwrap();
        let weak = Arc::downgrade(&matrix.storage);
        let pending = store
            .stage(BTreeMap::from([("A".into(), matrix.into())]))
            .unwrap();
        assert!(weak.upgrade().is_none());
        store.commit(pending);
        assert!(weak.upgrade().is_none());
    }
}

#[cfg(test)]
#[path = "workspace_graph_tests.rs"]
mod graph_tests;

fn checkpoint(cancel: &AtomicBool) -> Result<()> {
    #[cfg(test)]
    STOP_AFTER.with(|slot| {
        if let Some(left) = slot.get() {
            if left == 0 {
                slot.set(None);
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            } else {
                slot.set(Some(left - 1));
            }
        }
    });
    check_cancel(cancel)
}
#[cfg(test)]
thread_local! { static STOP_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) }; }
