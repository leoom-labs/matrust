//! Small reusable preferences, independent of document and figure identities.
use super::DesktopLayout;
use crate::Result;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{IgnoredAny, MapAccess, SeqAccess, Visitor},
};
use std::{fmt, io::Write};

pub const MAX_LAYOUT_PRESETS: usize = 16;
pub const MAX_LAYOUT_PRESET_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedLayout {
    name: String,
    layout: DesktopLayout,
    split_editor: bool,
    compact_figure: bool,
}
impl NamedLayout {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn layout(&self) -> DesktopLayout {
        self.layout
    }
    pub fn split_editor(&self) -> bool {
        self.split_editor
    }
    pub fn compact_figure(&self) -> bool {
        self.compact_figure
    }
    fn valid(&self) -> bool {
        valid_name(&self.name).is_ok_and(|name| name == self.name)
            && self.layout.editor_columns.is_none()
            && self.layout.tools.is_none()
            && {
                let mut normalized = self.layout;
                normalized.normalize();
                normalized == self.layout
            }
    }
}

pub fn valid_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > 256
        || name.chars().count() > 64
        || name.chars().any(char::is_control)
    {
        Err("Use a nonempty layout name of at most 64 characters / 256 UTF-8 bytes, without control characters".into())
    } else {
        Ok(name)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct LayoutPresets(Vec<NamedLayout>);
impl LayoutPresets {
    pub fn entries(&self) -> &[NamedLayout] {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn get(&self, name: &str) -> Option<&NamedLayout> {
        self.0.iter().find(|entry| entry.name == name)
    }
    pub fn save(
        &mut self,
        name: &str,
        mut layout: DesktopLayout,
        split_editor: bool,
        compact_figure: bool,
        replace: bool,
    ) -> Result<()> {
        let name = valid_name(name)?.to_owned();
        layout.editor_columns = None;
        layout.tools = None;
        layout.normalize();
        let position = self.0.iter().position(|entry| entry.name == name);
        if position.is_some() && !replace {
            return Err("A layout with that name already exists; use Replace layout".into());
        }
        if position.is_none() && self.0.len() == MAX_LAYOUT_PRESETS {
            return Err("Save at most 16 named layouts; delete one before adding another".into());
        }
        let mut candidate = self.clone();
        let entry = NamedLayout {
            name,
            layout,
            split_editor,
            compact_figure,
        };
        if let Some(index) = position {
            candidate.0[index] = entry;
        } else {
            candidate.0.push(entry);
        }
        candidate.check()?;
        *self = candidate;
        Ok(())
    }
    pub fn rename(&mut self, old: &str, new: &str) -> Result<()> {
        let new = valid_name(new)?.to_owned();
        let index = self
            .0
            .iter()
            .position(|entry| entry.name == old)
            .ok_or("The saved layout no longer exists")?;
        if old != new && self.get(&new).is_some() {
            return Err("Another layout already uses that name".into());
        }
        let mut candidate = self.clone();
        candidate.0[index].name = new;
        candidate.check()?;
        *self = candidate;
        Ok(())
    }
    pub fn remove(&mut self, name: &str) -> bool {
        if let Some(index) = self.0.iter().position(|entry| entry.name == name) {
            self.0.remove(index);
            true
        } else {
            false
        }
    }
    pub(super) fn normalize(&mut self) {
        if self.check().is_err() {
            self.0.clear();
        }
    }
    fn check(&self) -> Result<()> {
        if self.0.len() > MAX_LAYOUT_PRESETS || self.0.iter().any(|entry| !entry.valid()) {
            return Err("Invalid named layout preferences".into());
        }
        for (index, entry) in self.0.iter().enumerate() {
            if self.0[..index].iter().any(|other| other.name == entry.name) {
                return Err("Duplicate saved layout name".into());
            }
        }
        struct Count(usize);
        impl Write for Count {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self.0.saturating_add(bytes.len());
                if self.0 > MAX_LAYOUT_PRESET_BYTES {
                    return Err(std::io::Error::other("Named layouts exceed 32 KiB"));
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serde_json::to_writer(Count(0), self).map_err(|error| error.to_string())
    }
}

// The entire Session input is already capped at 32 MiB. At most sixteen entry
// values are decoded; excess entries are consumed without retaining them. One
// entry's temporary Value remains subject to that enclosing byte/depth bound.
// Invalid optional preferences never prevent document/workspace recovery.
impl<'de> Deserialize<'de> for LayoutPresets {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct Presets;
        impl<'de> Visitor<'de> for Presets {
            type Value = LayoutPresets;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an optional bounded layout list")
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut result = LayoutPresets::default();
                let mut count = 0;
                while count < MAX_LAYOUT_PRESETS {
                    let Some(value) = seq.next_element::<serde_json::Value>()? else {
                        result.normalize();
                        return Ok(result);
                    };
                    count += 1;
                    if let Ok(entry) = serde_json::from_value::<NamedLayout>(value)
                        && entry.valid()
                        && result.get(&entry.name).is_none()
                    {
                        result.0.push(entry);
                    }
                }
                let mut excess = false;
                while seq.next_element::<IgnoredAny>()?.is_some() {
                    excess = true;
                }
                if excess {
                    result.0.clear();
                } else {
                    result.normalize();
                }
                Ok(result)
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(LayoutPresets::default())
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(LayoutPresets::default())
            }
            fn visit_bool<E: serde::de::Error>(
                self,
                _: bool,
            ) -> std::result::Result<Self::Value, E> {
                Ok(LayoutPresets::default())
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> std::result::Result<Self::Value, E> {
                Ok(LayoutPresets::default())
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> std::result::Result<Self::Value, E> {
                Ok(LayoutPresets::default())
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> std::result::Result<Self::Value, E> {
                Ok(LayoutPresets::default())
            }
            fn visit_str<E: serde::de::Error>(
                self,
                _: &str,
            ) -> std::result::Result<Self::Value, E> {
                Ok(LayoutPresets::default())
            }
        }
        deserializer.deserialize_any(Presets)
    }
}
