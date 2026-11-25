//! Events, listeners, enumerations and class metadata of user classes:
//! `events`, `addlistener`, `notify`, `enumeration`, `metaclass`.
//!
//! Listeners live in the handle table entry of their source object (they go
//! with it). An enumeration member is a value object of its class built by
//! the constructor with the member's arguments, carrying its member name in a
//! hidden field; two members are equal when class and name match.
use super::handles::is_handle_object;
use super::{ClassInfo, logical, object_class, text};
use crate::engine::user_functions::Program;
use crate::engine::{Engine, Execution, Value};
use crate::{Array, CellArray, CharacterRow, Result, StructArray};
use std::sync::Arc;

/// Hidden field holding the member name of an enumeration value.
pub(in crate::engine) const ENUM_FIELD: &str = "MatRustEnumMember";

/// Member name of an enumeration value.
pub(in crate::engine) fn enum_name(a: &Array) -> Option<String> {
    let Array::Struct(s) = a else { return None };
    if s.len() != 1 {
        return None;
    }
    let Array::Character(c) = s.get(s.field_index(ENUM_FIELD)?, 0).ok()?.as_ref() else {
        return None;
    };
    Some(String::from_utf16_lossy(c.units()))
}

/// isequal of an enumeration member with another member (same class and
/// name) or with text (its name). None when neither side is a member.
pub(in crate::engine) fn enum_equal(a: &Array, b: &Array) -> Option<bool> {
    let name_of = |x: &Array| enum_name(x).map(|n| (object_class(x), n));
    let text_of = |x: &Array| match x {
        Array::Character(c) if c.rows() <= 1 => Some(String::from_utf16_lossy(c.units())),
        _ => None,
    };
    match (name_of(a), name_of(b)) {
        (Some(x), Some(y)) => Some(x == y),
        (Some((_, n)), None) => Some(text_of(b).is_some_and(|t| t == n)),
        (None, Some((_, n))) => Some(text_of(a).is_some_and(|t| t == n)),
        (None, None) => None,
    }
}

fn char_array(text: &str, engine: &Engine) -> Result<Array> {
    Ok(Array::Character(CharacterRow::from_text(
        text,
        &engine.cancel,
    )?))
}

impl Engine {
    /// `Class.Member` for an enumeration member: None when `member` is not one.
    pub(super) fn cls_enum_member(
        &mut self,
        program: &Arc<Program>,
        member: &str,
        out: &mut Execution,
    ) -> Result<Option<Value>> {
        let class = program.class.clone().expect("class program");
        let Some((name, args)) = class.enumeration.iter().find(|(n, _)| n == member).cloned()
        else {
            return Ok(None);
        };
        if class.is_handle {
            return Err("handle enumeration classes are not supported yet".into());
        }
        let values = self.function_arguments(&args, None, out)?;
        let object = self
            .cls_invoke(program, class.constructor, values, 1, None, out)?
            .remove(0)
            .array(&self.cancel)?;
        let Array::Struct(s) = object else {
            return Err(format!(
                "The constructor of {} must return the object.",
                class.name
            ));
        };
        let tagged = s.with_field(ENUM_FIELD, char_array(&name, self)?, &self.cancel)?;
        Ok(Some(Value::Container(Array::Struct(tagged))))
    }

    /// Calls answered for classes with events or enumerations, and
    /// `metaclass`: the object forms (`first` is the object's class) and the
    /// class-name forms. None leaves the call unchanged.
    pub(super) fn cls_event_call(
        &mut self,
        name: &str,
        args: &[Value],
        requested: usize,
        out: &mut Execution,
    ) -> Result<Option<Vec<Value>>> {
        if !matches!(
            name,
            "addlistener" | "notify" | "events" | "enumeration" | "metaclass"
        ) {
            return Ok(None);
        }
        let Some(first) = args.first() else {
            return Err("Not enough input arguments.".into());
        };
        let object = match first {
            Value::Container(a) if object_class(a).is_some() => Some(a.clone()),
            _ => None,
        };
        let class_name = match &object {
            Some(a) => object_class(a).expect("object"),
            None if matches!(name, "events" | "enumeration") => match text(first) {
                Some(n) => n,
                None => return Ok(None),
            },
            None => return Ok(None),
        };
        let Some(program) = self.cls_lookup_in(&class_name, object.is_none(), out)? else {
            return Err(format!(
                "{name}: '{class_name}' is not a class file in the function folder."
            ));
        };
        let class = program.class.clone().expect("class program");
        let value = match name {
            "events" => {
                if args.len() > 1 {
                    return Err("Too many input arguments.".into());
                }
                if requested == 0 {
                    let mut text = format!("Events for class {class_name}:\n\n");
                    for e in &class.events {
                        text.push_str(&format!("    {e}\n"));
                    }
                    self.emit(out, text)?;
                    Value::Void
                } else {
                    self.cls_names_cell(class.events.clone())?
                }
            }
            "enumeration" => {
                if args.len() > 1 {
                    return Err("Too many input arguments.".into());
                }
                let names: Vec<String> = class.enumeration.iter().map(|(n, _)| n.clone()).collect();
                if requested == 0 {
                    let text = if names.is_empty() {
                        format!("No enumeration members for class {class_name}.\n")
                    } else {
                        let mut text = format!("Enumeration members for class '{class_name}':\n\n");
                        for n in &names {
                            text.push_str(&format!("    {n}\n"));
                        }
                        text
                    };
                    self.emit(out, text)?;
                    return Ok(Some(vec![Value::Void]));
                }
                // Object arrays are not supported yet: the members come as a
                // column cell array.
                let mut members = Vec::new();
                for n in &names {
                    let v = self.cls_enum_member(&program, n, out)?.expect("member");
                    members.push(Arc::new(v.array(&self.cancel)?));
                }
                let rows = members.len();
                let cell = Value::Container(Array::Cell(CellArray::from_shared(
                    rows,
                    1,
                    members,
                    &self.cancel,
                )?));
                let mut outs = vec![cell];
                if requested > 1 {
                    outs.push(self.cls_names_cell(names)?);
                }
                return Ok(Some(outs));
            }
            "metaclass" => {
                if args.len() > 1 {
                    return Err("Too many input arguments.".into());
                }
                self.cls_metaclass(&class)?
            }
            "addlistener" => {
                let object = object.expect("object");
                if args.len() != 3 {
                    return Err(if args.len() < 3 {
                        "Not enough input arguments."
                    } else {
                        "addlistener supports addlistener(obj, EventName, callback) only."
                    }
                    .into());
                }
                if !is_handle_object(&object) {
                    return Err(format!(
                        "addlistener: class '{class_name}' is not a handle class; listeners need classdef {class_name} < handle."
                    ));
                }
                let event = text(&args[1]).ok_or("addlistener: the event name must be text.")?;
                if !class.events.contains(&event) {
                    return Err(format!(
                        "Class '{class_name}' has no event named '{event}'."
                    ));
                }
                let Value::Container(Array::Handle(callback)) = &args[2] else {
                    return Err("addlistener: the callback must be a function handle.".into());
                };
                self.cls_add_listener(&object, &event, callback.clone())?;
                // A plain struct stands for the event.listener object.
                let names = vec![
                    "Source".to_owned(),
                    "EventName".to_owned(),
                    "Callback".to_owned(),
                    "Enabled".to_owned(),
                ];
                let slots = vec![
                    Arc::new(object.clone()),
                    Arc::new(char_array(&event, self)?),
                    Arc::new(Array::Handle(callback.clone())),
                    Arc::new(logical(true).array(&self.cancel)?),
                ];
                Value::Container(Array::Struct(StructArray::from_slots(
                    1,
                    1,
                    names,
                    slots,
                    &self.cancel,
                )?))
            }
            "notify" => {
                let object = object.expect("object");
                if !(2..=3).contains(&args.len()) {
                    return Err(if args.len() < 2 {
                        "Not enough input arguments."
                    } else {
                        "Too many input arguments."
                    }
                    .into());
                }
                if !is_handle_object(&object) {
                    return Err(format!(
                        "notify: class '{class_name}' is not a handle class."
                    ));
                }
                let event = text(&args[1]).ok_or("notify: the event name must be text.")?;
                if !class.events.contains(&event) {
                    return Err(format!(
                        "Class '{class_name}' has no event named '{event}'."
                    ));
                }
                let data = self.cls_event_data(&object, &event, args.get(2))?;
                for callback in self.cls_listeners(&object, &event)? {
                    let result = self.call_handle(
                        &callback,
                        vec![Value::Container(object.clone()), data.clone()],
                        0,
                        None,
                        out,
                    );
                    // As documented: an error in a listener callback becomes a
                    // warning, and the other listeners and the caller go on.
                    if let Err(message) = result {
                        self.check_cancel()?;
                        self.emit(
                            out,
                            format!(
                                "Warning: Error occurred while executing the listener callback for event {event} defined for class {class_name}:\n{message}"
                            ),
                        )?;
                    }
                }
                Value::Void
            }
            _ => unreachable!(),
        };
        Ok(Some(vec![value]))
    }

    /// The event data a listener receives: Source and EventName, plus the
    /// fields of a struct passed to notify.
    fn cls_event_data(&self, source: &Array, event: &str, extra: Option<&Value>) -> Result<Value> {
        let mut s = StructArray::from_slots(
            1,
            1,
            vec!["Source".into(), "EventName".into()],
            vec![Arc::new(source.clone()), Arc::new(char_array(event, self)?)],
            &self.cancel,
        )?;
        match extra {
            None => {}
            Some(Value::Container(Array::Struct(d)))
                if d.len() == 1 && object_class(&Array::Struct(d.clone())).is_none() =>
            {
                for name in d.field_names() {
                    if name == "Source" || name == "EventName" {
                        continue;
                    }
                    let v = d
                        .get(d.field_index(name).expect("field"), 0)?
                        .as_ref()
                        .clone();
                    s = s.with_field(name, v, &self.cancel)?;
                }
            }
            Some(_) => {
                return Err("notify: the event data must be a scalar struct.".into());
            }
        }
        Ok(Value::Container(Array::Struct(s)))
    }

    /// `metaclass(obj)`: a struct standing for the meta.class object.
    fn cls_metaclass(&self, class: &ClassInfo) -> Result<Value> {
        let named = |names: Vec<String>| -> Result<Arc<Array>> {
            let slots = names
                .iter()
                .map(|n| Ok(Arc::new(char_array(n, self)?)))
                .collect::<Result<Vec<_>>>()?;
            let rows = slots.len();
            Ok(Arc::new(Array::Struct(StructArray::from_slots(
                rows,
                1,
                vec!["Name".into()],
                slots,
                &self.cancel,
            )?)))
        };
        let properties = class.properties.iter().map(|p| p.name.clone()).collect();
        let methods = class.methods.keys().cloned().collect();
        let names = vec![
            "Name".to_owned(),
            "Sealed".to_owned(),
            "HandleCompatible".to_owned(),
            "Enumeration".to_owned(),
            "PropertyList".to_owned(),
            "MethodList".to_owned(),
            "EventList".to_owned(),
            "EnumerationMemberList".to_owned(),
            "SuperclassList".to_owned(),
        ];
        let flag =
            |b: bool| -> Result<Arc<Array>> { Ok(Arc::new(logical(b).array(&self.cancel)?)) };
        let slots = vec![
            Arc::new(char_array(&class.name, self)?),
            flag(class.sealed)?,
            flag(class.is_handle)?,
            flag(!class.enumeration.is_empty())?,
            named(properties)?,
            named(methods)?,
            named(class.events.clone())?,
            named(class.enumeration.iter().map(|(n, _)| n.clone()).collect())?,
            named(class.supers.clone())?,
        ];
        Ok(Value::Container(Array::Struct(StructArray::from_slots(
            1,
            1,
            names,
            slots,
            &self.cancel,
        )?)))
    }
}
