//! Read-only, bounded SLX import. No callbacks, parameter evaluation or extraction.
use crate::Result;
use std::{path::Path, sync::atomic::AtomicBool};
#[path = "slx/archive.rs"]
mod archive;
#[path = "slx/graph.rs"]
mod graph;
#[path = "slx/xml.rs"]
mod xml;

/// Source identity is informational; the imported model is an unsaved native document.
#[derive(Debug)]
pub struct ImportedModel {
    pub model: super::Model,
    pub source_blake3: String,
}

fn checkpoint(cancel: &AtomicBool) -> Result<()> {
    #[cfg(test)]
    tests::checkpoint(cancel);
    crate::dsp::check_cancel(cancel)
}

pub fn import(path: &Path, cancel: &AtomicBool) -> Result<ImportedModel> {
    let bytes = archive::read(path, cancel)?;
    let source_blake3 = blake3::hash(&bytes).to_hex().to_string();
    let mut package = archive::Package::new(&bytes, cancel)?;
    let relationships = xml::parse(&package.part("_rels/.rels")?, cancel)?;
    let block_part = relationship(
        &relationships,
        "http://schemas.mathworks.com/simulink/2010/relationships/blockDiagram",
    )?;
    let config_info = relationship(
        &relationships,
        "http://schemas.mathworks.com/simulink/2014/relationships/configSetInfo",
    )?;
    let info = xml::parse(&package.part(&config_info)?, cancel)?;
    if info.name != "ConfigSetInfo" {
        return Err("SLX: unsupported configuration index root".into());
    }
    let active: Vec<_> = info
        .children
        .iter()
        .filter(|n| n.name == "ConfigSet" && n.attr("Active") == Some("true"))
        .collect();
    if active.len() != 1 {
        return Err("SLX: exactly one saved active configuration is required".into());
    }
    let config_part = active[0]
        .required_attr("PartName")?
        .strip_prefix('/')
        .ok_or("SLX: active configuration must be an absolute package part")?;
    archive::part_name(config_part)?;
    let config = xml::parse(&package.part(config_part)?, cancel)?;
    let diagram = xml::parse(&package.part(&block_part)?, cancel)?;
    let model = graph::decode(&diagram, &config, cancel)?;
    checkpoint(cancel)?;
    Ok(ImportedModel {
        model,
        source_blake3,
    })
}

fn relationship(root: &xml::Element, kind: &str) -> Result<String> {
    if root.name != "Relationships" {
        return Err("SLX: invalid root relationships".into());
    }
    let mut found = None;
    let mut ids = std::collections::BTreeSet::new();
    for n in &root.children {
        if n.name != "Relationship" {
            return Err("SLX: unsupported relationship element".into());
        }
        if !ids.insert(n.required_attr("Id")?) {
            return Err("SLX: duplicate relationship identity".into());
        }
        if n.attr("Type") == Some(kind) {
            if found.is_some() || n.attr("TargetMode").is_some_and(|x| x != "Internal") {
                return Err("SLX: ambiguous or external model relationship".into());
            }
            let target = n.required_attr("Target")?;
            archive::part_name(target)?;
            found = Some(target.to_owned());
        }
    }
    found.ok_or_else(|| format!("SLX: missing relationship {kind}"))
}

#[cfg(test)]
#[path = "slx/tests.rs"]
mod tests;
