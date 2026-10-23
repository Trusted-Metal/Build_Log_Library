//! Keeping what an adapter does not type (IP-13 phases 13B and 13D; requirement BL-CD-08): an attribute becomes an entry of the Model's `Extra`, with the form it
//! was stored in, and a group or dataset that the adapter does not type becomes an unmapped node, by its path.
//!
//! * **Layout-free.** The helpers here know no name: an adapter says which attributes and children it types, and everything else is kept.
//! * **Nothing is narrowed or decided.** A number keeps its width and sign, a scalar stays apart from an array of one, and a fixed-length string keeps its
//!   length and padding. Bytes that are valid text are kept as text (they give the same bytes back); bytes that are not stay bytes. Whether eight bytes are a
//!   number is the adapter's knowledge and is not decided here.
//! * **An attribute that cannot be read is still kept**, as an entry that says why, so that the file shows what it holds. A real failure (a truncated file,
//!   an I/O error) is an error and is not kept.
//! * **A node is described, never read.** A group is kept with its own attributes and with everything below it, each child as a node of its own; a dataset is
//!   kept with its extents, its element type and its own attributes, and its samples are not read. The paths are relative to the group the adapter starts from.
//!   The walk stops at [`MAX_DEPTH`] levels, which no file we hold comes near, so a file whose links form a loop cannot make it run for ever.

use build_log_model::{
    AttrShape, AttrValue, Dims, Error, ErrorKind, Extra, ExtraAttribute, NodeKind, StoredAs,
    StoredClass, TextCharset, TextPadding, UnknownNode, MAX_ATTRIBUTE_BYTES,
};
use hdf5::{Group, Location};

use super::attr::read_attribute;
use super::dataset::describe_dataset;
use super::error::from_binding;
use super::value::{Charset, Class, Padding, Shape, Stored, TextLength, Value};

/// How many levels of groups the walk follows below the group it starts from.
pub const MAX_DEPTH: usize = 64;

fn charset(charset: Charset) -> Option<TextCharset> {
    match charset {
        Charset::Ascii => Some(TextCharset::Ascii),
        Charset::Utf8 => Some(TextCharset::Utf8),
    }
}

fn padding(padding: Padding) -> TextPadding {
    match padding {
        Padding::NullTerminated => TextPadding::NullTerminated,
        Padding::NullPadded => TextPadding::NullPadded,
        Padding::SpacePadded => TextPadding::SpacePadded,
        Padding::Unknown => TextPadding::Unknown,
    }
}

/// The class of one element: for an element that is itself an array, the class of what it is an array of.
fn element_class(class: &Class) -> &Class {
    match class {
        Class::Array { base, .. } => element_class(base),
        other => other,
    }
}

fn shape_of(class: &Class, shape: &Shape) -> AttrShape {
    let outer = match shape {
        Shape::Array(dims) => format!("an array of {dims:?}"),
        _ => "a scalar".to_string(),
    };
    match (class, shape) {
        (Class::Array { dims, .. }, _) => AttrShape::Other(format!(
            "each element is an array of {dims:?}; the attribute is {outer}"
        )),
        (_, Shape::Scalar) => AttrShape::Scalar,
        (_, Shape::Array(dims)) if dims.len() == 1 => AttrShape::Array {
            elements: dims[0] as u64,
        },
        _ => AttrShape::Other(outer),
    }
}

/// The elements as the Model holds them, and whether they came from a fixed-length string whose bytes are text.
fn value_of(value: &Value) -> (AttrValue, bool) {
    match value {
        Value::I8(v) => (
            AttrValue::Signed(v.iter().map(|&x| i64::from(x)).collect()),
            false,
        ),
        Value::I16(v) => (
            AttrValue::Signed(v.iter().map(|&x| i64::from(x)).collect()),
            false,
        ),
        Value::I32(v) => (
            AttrValue::Signed(v.iter().map(|&x| i64::from(x)).collect()),
            false,
        ),
        Value::I64(v) => (AttrValue::Signed(v.clone()), false),
        Value::U8(v) => (
            AttrValue::Unsigned(v.iter().map(|&x| u64::from(x)).collect()),
            false,
        ),
        Value::U16(v) => (
            AttrValue::Unsigned(v.iter().map(|&x| u64::from(x)).collect()),
            false,
        ),
        Value::U32(v) => (
            AttrValue::Unsigned(v.iter().map(|&x| u64::from(x)).collect()),
            false,
        ),
        Value::U64(v) => (AttrValue::Unsigned(v.clone()), false),
        Value::F32(v) => (
            AttrValue::Float(v.iter().map(|&x| f64::from(x)).collect()),
            false,
        ),
        Value::F64(v) => (AttrValue::Float(v.clone()), false),
        Value::Text(v) => (AttrValue::Text(v.clone()), false),
        Value::Bytes(v) => {
            let text: Option<Vec<String>> = v
                .iter()
                .map(|bytes| String::from_utf8(bytes.clone()).ok())
                .collect();
            match text {
                Some(strings) => (AttrValue::Text(strings), true),
                None => (AttrValue::Bytes(v.clone()), false),
            }
        }
    }
}

/// How one element of this class is stored, in the Model's words. The shape is not set here.
fn form_of(class: &Class, bytes_are_text: bool) -> StoredAs {
    match element_class(class) {
        Class::Integer { bits, signed, .. } => {
            StoredAs::number(StoredClass::Integer, u16::from(*bits), Some(*signed))
        }
        Class::Float { bits, .. } => StoredAs::number(StoredClass::Float, u16::from(*bits), None),
        Class::Text(text) => match text.length {
            TextLength::Variable => StoredAs::variable_text(charset(text.charset)),
            TextLength::Fixed {
                bytes,
                padding: fill,
            } => StoredAs::fixed_text(
                bytes as u64,
                padding(fill),
                charset(text.charset),
                if bytes_are_text {
                    StoredClass::Text
                } else {
                    StoredClass::Bytes
                },
            ),
        },
        other => StoredAs::new(StoredClass::Other(format!("{other:?}"))),
    }
}

/// An attribute that was read, as an entry of `extra`: its elements and the form it was stored in.
pub fn extra_attribute(stored: &Stored) -> ExtraAttribute {
    let (value, bytes_are_text) = value_of(&stored.value);
    let mut form = form_of(&stored.encoding.class, bytes_are_text);
    form.shape = shape_of(&stored.encoding.class, &stored.encoding.shape);
    ExtraAttribute::new(value, form)
}

/// An attribute that could not be read, kept as an empty entry that says why.
pub fn unread_attribute(error: &Error) -> ExtraAttribute {
    ExtraAttribute::new(
        AttrValue::Bytes(Vec::new()),
        StoredAs::new(StoredClass::Other("not read".to_string())),
    )
    .with_issue(format!("the attribute was not read: {error}"))
}

/// An attribute that is too large to be metadata, kept as an empty entry that says how large it is.
fn too_large(size: usize) -> ExtraAttribute {
    ExtraAttribute::new(
        AttrValue::Bytes(Vec::new()),
        StoredAs::new(StoredClass::Other("not kept".to_string())),
    )
    .with_issue(format!(
        "the attribute holds {size} bytes, more than the {MAX_ATTRIBUTE_BYTES} that are kept as metadata"
    ))
}

/// Every attribute of `location` whose name is not in `typed`, read, in the order the file lists them.
fn other_attributes(
    location: &Location,
    typed: &[&str],
) -> Result<Vec<(String, ExtraAttribute)>, Error> {
    let names = location
        .attr_names()
        .map_err(|e| from_binding("listing the attributes of an object", e))?;
    let mut found = Vec::new();
    for name in names.iter().filter(|name| !typed.contains(&name.as_str())) {
        let mut attribute = match read_attribute(location, name) {
            Ok(stored) => extra_attribute(&stored),
            Err(error) if matches!(error.kind(), ErrorKind::UnsupportedInVersion { .. }) => {
                unread_attribute(&error)
            }
            Err(error) => return Err(error),
        };
        let size = attribute.value.size_bytes();
        if size > MAX_ATTRIBUTE_BYTES {
            attribute = too_large(size);
        }
        found.push((name.clone(), attribute));
    }
    Ok(found)
}

/// Keeps every attribute of `location` whose name is not in `typed`, in `extra`.
///
/// The attributes that the adapter types are named in `typed` and are not kept twice. An attribute of a class that the primitives do not read, and one that
/// is too large, are kept as entries that say so; any other failure is returned.
pub fn keep_other_attributes(
    location: &Location,
    typed: &[&str],
    extra: &mut Extra,
) -> Result<(), Error> {
    for (name, attribute) in other_attributes(location, typed)? {
        extra.set_attribute(name, attribute)?;
    }
    Ok(())
}

// ---------------------------------------------------------------- nodes

fn path_of(base: &[String], name: &str) -> Vec<String> {
    let mut path = base.to_vec();
    path.push(name.to_string());
    path
}

fn describe_group(
    group: &Group,
    path: Vec<String>,
    typed_attributes: &[&str],
) -> Result<UnknownNode, Error> {
    let mut node = UnknownNode::group(path);
    for (name, attribute) in other_attributes(group, typed_attributes)? {
        node.set_attribute(name, attribute)?;
    }
    Ok(node)
}

fn describe_dataset_node(
    dataset: &hdf5::Dataset,
    path: Vec<String>,
    typed_attributes: &[&str],
) -> Result<UnknownNode, Error> {
    let descriptor = describe_dataset(dataset)?;
    let extents: Vec<u64> = descriptor.shape.iter().map(|&n| n as u64).collect();
    let mut node = UnknownNode::dataset(
        path,
        Dims::new(&extents)?,
        form_of(&descriptor.class, false),
    );
    for (name, attribute) in other_attributes(dataset, typed_attributes)? {
        node.set_attribute(name, attribute)?;
    }
    Ok(node)
}

/// Keeps one child of `parent`, by `name`, as a node at `base` and below, with its attributes and (for a group) everything under it.
pub fn keep_node(
    parent: &Group,
    name: &str,
    base: &[String],
    extra: &mut Extra,
) -> Result<(), Error> {
    keep_node_at(parent, name, base, extra, 0)
}

fn keep_node_at(
    parent: &Group,
    name: &str,
    base: &[String],
    extra: &mut Extra,
    depth: usize,
) -> Result<(), Error> {
    let path = path_of(base, name);
    if depth >= MAX_DEPTH {
        extra.add_node(UnknownNode::new(
            path,
            NodeKind::Other(format!(
                "nested deeper than {MAX_DEPTH} levels: not followed"
            )),
        ));
        return Ok(());
    }
    if let Ok(group) = parent.group(name) {
        extra.add_node(describe_group(&group, path.clone(), &[])?);
        keep_children(&group, &[], &path, extra, depth + 1)
    } else if let Ok(dataset) = parent.dataset(name) {
        extra.add_node(describe_dataset_node(&dataset, path, &[])?);
        Ok(())
    } else {
        extra.add_node(UnknownNode::new(
            path,
            NodeKind::Other(
                "neither a group nor a dataset (a link that does not lead to one, or a named type)"
                    .to_string(),
            ),
        ));
        Ok(())
    }
}

fn keep_children(
    group: &Group,
    typed: &[&str],
    base: &[String],
    extra: &mut Extra,
    depth: usize,
) -> Result<(), Error> {
    let names = group
        .member_names()
        .map_err(|e| from_binding("listing the members of a group", e))?;
    for name in names.iter().filter(|name| !typed.contains(&name.as_str())) {
        keep_node_at(group, name, base, extra, depth)?;
    }
    Ok(())
}

/// Keeps every child of `group` whose name is not in `typed`, each as a node at `base` and below. The attributes of `group` itself are not touched: they are
/// the caller's (see [`keep_other_attributes`] and [`keep_group_remainder`]).
pub fn keep_other_nodes(
    group: &Group,
    typed: &[&str],
    base: &[String],
    extra: &mut Extra,
) -> Result<(), Error> {
    keep_children(group, typed, base, extra, 0)
}

/// Keeps what an adapter did not type of a group that it **does** type as a node of its own (a group of channels, say): the attributes of the group that are
/// not in `typed_attributes`, as a node at `base`, and the children that are not in `typed_children`.
pub fn keep_group_remainder(
    group: &Group,
    typed_children: &[&str],
    typed_attributes: &[&str],
    base: &[String],
    extra: &mut Extra,
) -> Result<(), Error> {
    let node = describe_group(group, base.to_vec(), typed_attributes)?;
    if node.attributes().next().is_some() {
        extra.add_node(node);
    }
    keep_children(group, typed_children, base, extra, 1)
}

/// Keeps the attributes that an adapter did not type of a dataset that it **does** type (a channel's samples): if there are any, as a node at `base` with its
/// extents and element type. A dataset with nothing untyped adds nothing.
pub fn keep_dataset_remainder(
    dataset: &hdf5::Dataset,
    typed_attributes: &[&str],
    base: &[String],
    extra: &mut Extra,
) -> Result<(), Error> {
    let node = describe_dataset_node(dataset, base.to_vec(), typed_attributes)?;
    if node.attributes().next().is_some() {
        extra.add_node(node);
    }
    Ok(())
}
