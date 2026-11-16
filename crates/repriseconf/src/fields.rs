//! A program's settings struct against `config.toml`: a table of [`Field`]s, each one key and how
//! its value moves in and out of the struct, so [`load`] and [`save`] read the same list.

use crate::{Key, Lens, SetError, get, remove, remove_table_if_empty, set};
use toml_edit::{Array, DocumentMut, Item, Value};

pub enum Field<S> {
    Text(Key, Lens<S, String>),
    Int(Key, Lens<S, i64>),
    /// An integer that may be left out (`[idle.battery]`'s, which inherit `[idle]`'s).
    OptInt(Key, Lens<S, Option<i64>>),
    Flag(Key, Lens<S, bool>),
    /// A comma-separated string against a TOML array of strings (`fonts.family`).
    List(Key, Lens<S, String>),
    /// A string of words apart by spaces or commas against a TOML array of strings
    /// (`window_snapping.layouts`).
    Words(Key, Lens<S, String>),
    /// An integer in tenths against a TOML float (`mouse.speed`), read back into its range of
    /// tenths.
    Tenths(Key, Lens<S, i64>, (i64, i64)),
    /// The system icon names, an older file's read as the current ones.
    Names(Key, Lens<S, Vec<String>>),
    /// A TOML array of strings kept entry for entry, empty ones included.
    Strings(Key, Lens<S, Vec<String>>),
}

impl<S> Field<S> {
    pub fn key(&self) -> &Key {
        match self {
            Field::Text(k, _)
            | Field::Int(k, _)
            | Field::OptInt(k, _)
            | Field::Flag(k, _)
            | Field::List(k, _)
            | Field::Words(k, _)
            | Field::Tenths(k, _, _)
            | Field::Names(k, _)
            | Field::Strings(k, _) => k,
        }
    }

    /// The value written for `s`; None for an [`Field::OptInt`] left out.
    pub fn value(&self, s: &S) -> Option<Value> {
        Some(match self {
            Field::Text(_, at) => Value::from(at.get(s).as_str()),
            Field::Int(_, at) => Value::from(*at.get(s)),
            Field::OptInt(_, at) => Value::from((*at.get(s))?),
            Field::Flag(_, at) => Value::from(*at.get(s)),
            Field::List(_, at) => {
                Value::Array(text_to_list(at.get(s)).into_iter().collect::<Array>())
            }
            Field::Words(_, at) => Value::Array(words(at.get(s)).collect::<Array>()),
            Field::Tenths(_, at, _) => Value::from(*at.get(s) as f64 / 10.0),
            Field::Names(_, at) | Field::Strings(_, at) => {
                Value::Array(at.get(s).iter().map(String::as_str).collect::<Array>())
            }
        })
    }

    pub fn same(&self, a: &S, b: &S) -> bool {
        match self {
            Field::Text(_, at) | Field::List(_, at) | Field::Words(_, at) => at.get(a) == at.get(b),
            Field::Int(_, at) | Field::Tenths(_, at, _) => at.get(a) == at.get(b),
            Field::OptInt(_, at) => at.get(a) == at.get(b),
            Field::Flag(_, at) => at.get(a) == at.get(b),
            Field::Names(_, at) | Field::Strings(_, at) => at.get(a) == at.get(b),
        }
    }

    pub fn copy(&self, from: &S, to: &mut S) {
        match self {
            Field::Text(_, at) | Field::List(_, at) | Field::Words(_, at) => {
                *at.get_mut(to) = at.get(from).clone()
            }
            Field::Int(_, at) | Field::Tenths(_, at, _) => *at.get_mut(to) = *at.get(from),
            Field::OptInt(_, at) => *at.get_mut(to) = *at.get(from),
            Field::Flag(_, at) => *at.get_mut(to) = *at.get(from),
            Field::Names(_, at) | Field::Strings(_, at) => *at.get_mut(to) = at.get(from).clone(),
        }
    }

    /// Take `item` into `s`; an item of another type leaves `s` as it was.
    pub fn read(&self, item: &Item, s: &mut S) {
        match self {
            Field::Text(_, at) => {
                if let Some(v) = item.as_str() {
                    *at.get_mut(s) = v.to_string();
                }
            }
            Field::Int(_, at) => {
                if let Some(v) = item.as_integer() {
                    *at.get_mut(s) = v;
                }
            }
            Field::OptInt(_, at) => {
                if let Some(v) = item.as_integer() {
                    *at.get_mut(s) = Some(v);
                }
            }
            Field::Flag(_, at) => {
                if let Some(v) = item.as_bool() {
                    *at.get_mut(s) = v;
                }
            }
            Field::List(_, at) => {
                if let Some(v) = list_to_text(item) {
                    *at.get_mut(s) = v;
                }
            }
            Field::Words(_, at) => {
                if let Some(arr) = item.as_array() {
                    let w: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
                    *at.get_mut(s) = w.join(" ");
                }
            }
            Field::Tenths(_, at, (lo, hi)) => {
                // `speed = 1` is a valid float to the compositor too.
                let v = item
                    .as_float()
                    .or_else(|| item.as_integer().map(|i| i as f64));
                if let Some(v) = v {
                    *at.get_mut(s) = ((v * 10.0).round() as i64).clamp(*lo, *hi);
                }
            }
            Field::Strings(_, at) => {
                if let Some(arr) = item.as_array() {
                    *at.get_mut(s) = arr
                        .iter()
                        .map(|v| v.as_str().unwrap_or_default().to_string())
                        .collect();
                }
            }
            Field::Names(_, at) => {
                if let Some(arr) = item.as_array() {
                    *at.get_mut(s) =
                        crate::system_icons::current_list(arr.iter().filter_map(|v| v.as_str()));
                }
            }
        }
    }
}

/// Read into `s` each of `fields` that `doc` holds.
pub fn load<S>(doc: &DocumentMut, fields: &[Field<S>], s: &mut S) {
    for f in fields {
        if let Some(item) = get(doc, f.key()) {
            f.read(item, s);
        }
    }
}

/// Write into `doc` each of `fields` that `want` moved from `base`, unless the file (`have`, with
/// the defaults standing in for the keys it leaves out) already holds it. A value moved back to
/// its default `def`, or left out, is removed, and so is a table that leaves empty (unless a
/// comment sits on or under its header).
pub fn save<S>(
    doc: &mut DocumentMut,
    fields: &[Field<S>],
    want: &S,
    base: &S,
    have: &S,
    def: &S,
) -> Result<(), SetError> {
    let mut shrunk = Vec::new();
    for f in fields {
        if f.same(want, base) || f.same(want, have) {
            continue;
        }
        match f.value(want) {
            Some(v) if !f.same(want, def) => set(doc, f.key(), v)?,
            _ if get(doc, f.key()).is_some() => {
                remove(doc, f.key());
                shrunk.push(f.key().table);
            }
            _ => {}
        }
    }
    for table in shrunk {
        remove_table_if_empty(doc, table);
    }
    Ok(())
}

pub fn list_to_text(item: &Item) -> Option<String> {
    let arr = item.as_array()?;
    Some(
        arr.iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    )
}

pub fn words(text: &str) -> impl Iterator<Item = &str> {
    (text.split([',', ' ', '\t'])).filter(|w| !w.is_empty())
}

pub fn text_to_list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}
