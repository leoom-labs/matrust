use super::checkpoint as check_cancel;
use crate::Result;
use quick_xml::{
    Reader,
    events::{BytesStart, Event},
};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

#[derive(Debug)]
pub(super) struct Element {
    pub name: String,
    pub attrs: BTreeMap<String, String>,
    pub text: String,
    pub children: Vec<Element>,
}
impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }
    pub fn required_attr(&self, name: &str) -> Result<&str> {
        self.attr(name)
            .ok_or_else(|| format!("SLX XML {}: missing {name}", self.name))
    }
    pub fn one(&self, name: &str) -> Result<&Self> {
        let mut found = self.children.iter().filter(|n| n.name == name);
        let first = found
            .next()
            .ok_or_else(|| format!("SLX XML {}: missing {name}", self.name))?;
        if found.next().is_some() {
            return Err(format!("SLX XML: duplicate {name}"));
        }
        Ok(first)
    }
    pub fn properties(&self) -> Result<BTreeMap<String, String>> {
        let mut out = BTreeMap::new();
        for p in self.children.iter().filter(|p| p.name == "P") {
            if !p.children.is_empty() {
                return Err("SLX: structured parameter unsupported".into());
            }
            let name = p.required_attr("Name")?;
            if out
                .insert(name.to_owned(), p.text.trim().to_owned())
                .is_some()
            {
                return Err(format!("SLX: duplicate parameter {name}"));
            }
        }
        Ok(out)
    }
}
fn start(e: &BytesStart<'_>, reader: &Reader<&[u8]>) -> Result<Element> {
    let name = std::str::from_utf8(e.name().as_ref())
        .map_err(|_| "SLX: non-UTF8 XML element")?
        .to_owned();
    if name.len() > 128 || name.contains(':') {
        return Err("SLX: unsupported XML element namespace/name".into());
    }
    let mut attrs = BTreeMap::new();
    for attr in e.attributes() {
        if attrs.len() >= 32 {
            return Err("SLX: XML attribute limit".into());
        }
        let a = attr.map_err(|e| e.to_string())?;
        let key = std::str::from_utf8(a.key.as_ref()).map_err(|_| "SLX: non-UTF8 XML attribute")?;
        let value = a
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
            .map_err(|e| e.to_string())?;
        if key.len() > 128
            || value.len() > 4096
            || attrs.insert(key.to_owned(), value.into_owned()).is_some()
        {
            return Err("SLX: duplicate or oversized XML attribute".into());
        }
    }
    Ok(Element {
        name,
        attrs,
        text: String::new(),
        children: Vec::new(),
    })
}
pub(super) fn parse(bytes: &[u8], cancel: &AtomicBool) -> Result<Element> {
    std::str::from_utf8(bytes).map_err(|_| "SLX: UTF8 XML required")?;
    let mut reader = Reader::from_reader(bytes);
    let mut stack: Vec<Element> = Vec::new();
    let mut root = None;
    let mut events = 0usize;
    let mut declaration_seen = false;
    loop {
        if events.is_multiple_of(128) {
            check_cancel(cancel)?;
        }
        events += 1;
        if events > 100_000 {
            return Err("SLX: XML event limit".into());
        }
        let event = reader.read_event().map_err(|e| format!("SLX XML: {e}"))?;
        let closed = match event {
            Event::Start(e) => {
                if stack.len() >= 32 {
                    return Err("SLX: XML depth limit".into());
                }
                stack.push(start(&e, &reader)?);
                None
            }
            Event::Empty(e) => {
                if stack.len() >= 32 {
                    return Err("SLX: XML depth limit".into());
                }
                Some(start(&e, &reader)?)
            }
            Event::End(_) => Some(stack.pop().ok_or("SLX: unmatched XML end")?),
            Event::Text(e) => {
                let value = e.decode().map_err(|e| e.to_string())?;
                if let Some(n) = stack.last_mut() {
                    if n.text.len().saturating_add(value.len()) > 4096 {
                        return Err("SLX: parameter text exceeds 4096 bytes".into());
                    }
                    n.text.push_str(&value);
                } else if !value.trim().is_empty() {
                    return Err("SLX: text outside XML root".into());
                }
                None
            }
            Event::GeneralRef(e) => {
                let raw = e.decode().map_err(|e| e.to_string())?;
                let encoded = format!("&{raw};");
                let value = quick_xml::escape::unescape(&encoded)
                    .map_err(|_| "SLX: custom XML entities unsupported")?;
                let n = stack.last_mut().ok_or("SLX: reference outside XML root")?;
                if n.text.len().saturating_add(value.len()) > 4096 {
                    return Err("SLX: parameter text limit".into());
                }
                n.text.push_str(&value);
                None
            }
            Event::Decl(e) => {
                if declaration_seen || root.is_some() || !stack.is_empty() {
                    return Err("SLX: misplaced or duplicate XML declaration".into());
                }
                declaration_seen = true;
                if e.encoding()
                    .transpose()
                    .map_err(|e| e.to_string())?
                    .is_some_and(|v| !v.eq_ignore_ascii_case(b"utf-8"))
                {
                    return Err("SLX: UTF8 XML declaration required".into());
                }
                None
            }
            Event::Comment(_) => None,
            Event::Eof => break,
            _ => return Err("SLX: DTD, CDATA and processing instructions unsupported".into()),
        };
        if let Some(n) = closed {
            if let Some(parent) = stack.last_mut() {
                parent.children.push(n);
            } else if root.replace(n).is_some() {
                return Err("SLX: multiple XML roots".into());
            }
        }
    }
    if !stack.is_empty() {
        return Err("SLX: truncated XML tree".into());
    }
    check_cancel(cancel)?;
    root.ok_or_else(|| "SLX: empty XML".into())
}
