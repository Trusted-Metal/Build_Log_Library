//! The `extra` passthrough: unknown attributes and groups kept so that they round-trip verbatim.
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** library (`Extra`, `ExtraAttribute`, `UnknownNode`, `StoredAs`, `Dims`); the enums `AttrValue`, `StoredClass`, `AttrShape`,
//!   `NodeKind`, `TextPadding` and `TextCharset`.
//! * **Requirement:** BL-CD-08.
//!
//! # What this module claims
//!
//! * **Nothing a file holds is dropped.** An adapter types what the Model has a field for and puts everything else here: an attribute with its name,
//!   its value and **how it was stored** (width, signedness, shape, text form), and a group or dataset by its path, its kind, its extents and its element
//!   type. A phase that adds a typed field takes the matching entry out of `extra` in the same change.
//! * **It holds descriptions and small values, never samples.** A dataset is described, never read; an attribute of more than [`MAX_ATTRIBUTE_BYTES`]
//!   is refused, and so are more than [`MAX_DIMS`] extents. The two types that hold sequences of numbers say `// bulk-container:` with this reason; no
//!   other type here can.
//! * **It says what was stored, not what was meant.** The Model does not decide how any version writes a value; the stored form is kept so that the
//!   adapter of the version can give the value back as it was.
//! * **A value that could not be read in full says so**: [`ExtraAttribute::issue`] holds the reason, and the value is what could be read.
//!
//! Events are not here yet (IP-02 phase 2P, the events half). The policy guards in `tests/policy_guards.rs` scan this file: every enum is
//! `#[non_exhaustive]` or says `// closed: <reason>`, every public struct says `// constructed-by: producer|library|caller-options` and follows that
//! rule, and none holds bulk data unless it says why.

use std::collections::BTreeMap;

use crate::error::{Error, ErrorKind, Violation};

/// The most bytes that one attribute may hold here. An attribute is metadata; anything larger is not, and the adapter reports it and does not keep it.
/// (A number counts as eight bytes, text and raw bytes as their length.)
pub const MAX_ATTRIBUTE_BYTES: usize = 65_536;

/// The most extents that a dataset's shape may have.
pub const MAX_DIMS: usize = 32;

/// The values of one attribute, as read: the elements in order, in the family that the file stored them in.
///
/// A scalar is one element. The width and signedness that the file stored are in [`StoredAs`], not here.
// bulk-container: the elements of one attribute, which is metadata; `Extra` and `UnknownNode` refuse more than `MAX_ATTRIBUTE_BYTES`, so no sample can be held here
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum AttrValue {
    /// Integers that the file stored as signed.
    Signed(Vec<i64>),
    /// Integers that the file stored as unsigned.
    Unsigned(Vec<u64>),
    /// Floating-point numbers, of either width.
    Float(Vec<f64>),
    /// Text that could be read as text.
    Text(Vec<String>),
    /// Raw bytes: a fixed-length string that is not text, kept as it was read.
    Bytes(Vec<Vec<u8>>),
}

impl AttrValue {
    /// How many elements the attribute has.
    pub fn len(&self) -> usize {
        match self {
            AttrValue::Signed(v) => v.len(),
            AttrValue::Unsigned(v) => v.len(),
            AttrValue::Float(v) => v.len(),
            AttrValue::Text(v) => v.len(),
            AttrValue::Bytes(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The size that counts against [`MAX_ATTRIBUTE_BYTES`]: eight bytes for each number, the length for text and raw bytes.
    pub fn size_bytes(&self) -> usize {
        match self {
            AttrValue::Signed(v) => v.len() * 8,
            AttrValue::Unsigned(v) => v.len() * 8,
            AttrValue::Float(v) => v.len() * 8,
            AttrValue::Text(v) => v.iter().map(String::len).sum(),
            AttrValue::Bytes(v) => v.iter().map(Vec::len).sum(),
        }
    }
}

/// The class of a stored value.
// open-domain: classes of stored value that a container can hold and nobody has listed yet
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StoredClass {
    Integer,
    Float,
    Text,
    /// Raw bytes that are not text.
    Bytes,
    /// Another class, by name.
    Other(String),
}

/// The shape of an attribute.
// open-domain: shapes of a container that nobody has listed yet (an array of arrays, more than one dimension)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AttrShape {
    Scalar,
    /// One dimension of this many elements. An array of one element is not the same as a scalar, and files hold both.
    Array {
        elements: u64,
    },
    /// Another shape, described in words.
    Other(String),
}

/// How a fixed-length string is padded, as far as the reader can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextPadding {
    NullTerminated,
    NullPadded,
    SpacePadded,
    /// The reader could not tell (a binding that shows only "null-padded or not").
    Unknown,
}

/// The character set a string was stored in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TextCharset {
    Ascii,
    Utf8,
}

/// How a value was stored, in words that name no file layout: its class, width, signedness and shape, and for text its length, padding and
/// character set. It is what an adapter needs to give the value back as it was.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct StoredAs {
    pub class: StoredClass,
    /// The width of a number, in bits.
    pub bits: Option<u16>,
    /// Whether an integer was signed.
    pub signed: Option<bool>,
    pub shape: AttrShape,
    /// For fixed-length text, the length in bytes; `None` for variable-length text and for numbers.
    pub fixed_bytes: Option<u64>,
    pub padding: Option<TextPadding>,
    pub charset: Option<TextCharset>,
}

impl StoredAs {
    /// A scalar of this class, with nothing else known.
    pub fn new(class: StoredClass) -> Self {
        StoredAs {
            class,
            bits: None,
            signed: None,
            shape: AttrShape::Scalar,
            fixed_bytes: None,
            padding: None,
            charset: None,
        }
    }

    /// A number: an integer or a float of this width.
    pub fn number(class: StoredClass, bits: u16, signed: Option<bool>) -> Self {
        StoredAs {
            bits: Some(bits),
            signed,
            ..StoredAs::new(class)
        }
    }

    /// Text of variable length.
    pub fn variable_text(charset: Option<TextCharset>) -> Self {
        StoredAs {
            charset,
            ..StoredAs::new(StoredClass::Text)
        }
    }

    /// Text of a fixed length in bytes.
    pub fn fixed_text(
        bytes: u64,
        padding: TextPadding,
        charset: Option<TextCharset>,
        class: StoredClass,
    ) -> Self {
        StoredAs {
            fixed_bytes: Some(bytes),
            padding: Some(padding),
            charset,
            ..StoredAs::new(class)
        }
    }

    pub fn with_shape(mut self, shape: AttrShape) -> Self {
        self.shape = shape;
        self
    }
}

/// One attribute that the adapter did not type, as read: its value, how it was stored, and what is wrong with it if anything is.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ExtraAttribute {
    pub value: AttrValue,
    pub stored_as: StoredAs,
    /// Why the value is not the whole of what the file holds, when it is not (for example a fixed-length string that the reader could not read in
    /// full). `None` when the value is what the file holds.
    pub issue: Option<String>,
}

impl ExtraAttribute {
    pub fn new(value: AttrValue, stored_as: StoredAs) -> Self {
        ExtraAttribute {
            value,
            stored_as,
            issue: None,
        }
    }

    pub fn with_issue(mut self, issue: impl Into<String>) -> Self {
        self.issue = Some(issue.into());
        self
    }
}

/// The extents of a dataset, one for each dimension. A scalar dataset has none.
// constructed-by: library
// bulk-container: the extents of one dataset, one number for each dimension and at most `MAX_DIMS`; the data of the dataset is never held
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub struct Dims {
    extents: Vec<u64>,
}

impl Dims {
    /// No extents: a scalar.
    pub fn scalar() -> Self {
        Dims::default()
    }

    /// The extents of a dataset. More than [`MAX_DIMS`] of them is not a shape: it is a `ValidationError`.
    pub fn new(extents: &[u64]) -> Result<Self, Error> {
        if extents.len() > MAX_DIMS {
            return Err(Error::new(ErrorKind::ValidationError {
                violations: vec![Violation::new(format!(
                    "a shape of {} extents is not kept: the most is {MAX_DIMS}",
                    extents.len()
                ))],
            }));
        }
        Ok(Dims {
            extents: extents.to_vec(),
        })
    }

    pub fn as_slice(&self) -> &[u64] {
        &self.extents
    }

    pub fn rank(&self) -> usize {
        self.extents.len()
    }
}

/// The kind of an unmapped node.
// open-domain: kinds of node that a container can hold and nobody has listed yet (a link, a named datatype)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NodeKind {
    Group,
    Dataset,
    /// Another kind, by name.
    Other(String),
}

/// A group or dataset that the adapter did not type, kept by where it is (a path of whole names, below the mount), what it is, its extents and, for a
/// dataset, what its elements are. **Its samples are never held**; it keeps its own attributes.
// constructed-by: library
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct UnknownNode {
    pub path: Vec<String>,
    pub kind: NodeKind,
    pub dims: Dims,
    /// What a dataset's elements are; `None` for a group.
    pub element: Option<StoredAs>,
    attributes: BTreeMap<String, ExtraAttribute>,
}

impl UnknownNode {
    /// A node of this kind at `path`, with no extents, no element type and no attributes.
    pub fn new(path: Vec<String>, kind: NodeKind) -> Self {
        UnknownNode {
            path,
            kind,
            dims: Dims::scalar(),
            element: None,
            attributes: BTreeMap::new(),
        }
    }

    /// A group at `path`.
    pub fn group(path: Vec<String>) -> Self {
        UnknownNode::new(path, NodeKind::Group)
    }

    /// A dataset at `path` with these extents and this element type.
    pub fn dataset(path: Vec<String>, dims: Dims, element: StoredAs) -> Self {
        UnknownNode {
            dims,
            element: Some(element),
            ..UnknownNode::new(path, NodeKind::Dataset)
        }
    }

    /// Keeps an attribute of this node, and returns the one it replaced. An attribute larger than [`MAX_ATTRIBUTE_BYTES`] is a `ValidationError`.
    pub fn set_attribute(
        &mut self,
        name: impl Into<String>,
        attribute: ExtraAttribute,
    ) -> Result<Option<ExtraAttribute>, Error> {
        insert_attribute(&mut self.attributes, name.into(), attribute)
    }

    pub fn attribute(&self, name: &str) -> Option<&ExtraAttribute> {
        self.attributes.get(name)
    }

    /// The attributes in name order.
    pub fn attributes(&self) -> impl Iterator<Item = (&str, &ExtraAttribute)> {
        self.attributes.iter().map(|(name, a)| (name.as_str(), a))
    }
}

/// What a node of the build (the build, a layer, a laser, a laser in a layer) holds that the adapter did not type: unknown attributes and unmapped nodes.
/// It is empty for a node that the adapter typed completely.
// constructed-by: library
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct Extra {
    attributes: BTreeMap<String, ExtraAttribute>,
    nodes: BTreeMap<Vec<String>, UnknownNode>,
}

impl Extra {
    pub fn new() -> Self {
        Extra::default()
    }

    /// Nothing is kept here.
    pub fn is_empty(&self) -> bool {
        self.attributes.is_empty() && self.nodes.is_empty()
    }

    /// How many attributes and nodes are kept (the attributes of an unmapped node are not counted).
    pub fn len(&self) -> usize {
        self.attributes.len() + self.nodes.len()
    }

    /// Keeps an attribute, and returns the one it replaced. An attribute larger than [`MAX_ATTRIBUTE_BYTES`] is a `ValidationError`.
    pub fn set_attribute(
        &mut self,
        name: impl Into<String>,
        attribute: ExtraAttribute,
    ) -> Result<Option<ExtraAttribute>, Error> {
        insert_attribute(&mut self.attributes, name.into(), attribute)
    }

    pub fn attribute(&self, name: &str) -> Option<&ExtraAttribute> {
        self.attributes.get(name)
    }

    /// The attributes in name order.
    pub fn attributes(&self) -> impl Iterator<Item = (&str, &ExtraAttribute)> {
        self.attributes.iter().map(|(name, a)| (name.as_str(), a))
    }

    /// Keeps an unmapped node, and returns the one at the same path that it replaced.
    pub fn add_node(&mut self, node: UnknownNode) -> Option<UnknownNode> {
        self.nodes.insert(node.path.clone(), node)
    }

    pub fn node(&self, path: &[String]) -> Option<&UnknownNode> {
        self.nodes.get(path)
    }

    /// The unmapped nodes in path order.
    pub fn nodes(&self) -> impl Iterator<Item = &UnknownNode> {
        self.nodes.values()
    }
}

/// The one place an attribute enters a map: its size is checked, so that nothing bulky is kept as metadata.
fn insert_attribute(
    map: &mut BTreeMap<String, ExtraAttribute>,
    name: String,
    attribute: ExtraAttribute,
) -> Result<Option<ExtraAttribute>, Error> {
    let size = attribute.value.size_bytes();
    if size > MAX_ATTRIBUTE_BYTES {
        return Err(Error::new(ErrorKind::ValidationError {
            violations: vec![Violation::new(format!(
                "the attribute `{name}` holds {size} bytes: an attribute of more than {MAX_ATTRIBUTE_BYTES} bytes is not metadata and is not kept"
            ))],
        }));
    }
    Ok(map.insert(name, attribute))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int32_array_of_one(value: i64) -> ExtraAttribute {
        ExtraAttribute::new(
            AttrValue::Signed(vec![value]),
            StoredAs::number(StoredClass::Integer, 32, Some(true))
                .with_shape(AttrShape::Array { elements: 1 }),
        )
    }

    fn text(value: &str) -> ExtraAttribute {
        ExtraAttribute::new(
            AttrValue::Text(vec![value.to_string()]),
            StoredAs::variable_text(Some(TextCharset::Utf8)),
        )
    }

    fn path(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn unknown_attributes_are_kept_with_their_encoding_and_value() {
        let mut extra = Extra::new();
        assert!(extra.is_empty());
        // a number stored as an array of one element, text of variable length, and a fixed-length string that could not be read in full
        extra
            .set_attribute("a_number", int32_array_of_one(7))
            .unwrap();
        extra.set_attribute("a_text", text("as written")).unwrap();
        let cut = ExtraAttribute::new(
            AttrValue::Bytes(vec![Vec::new()]),
            StoredAs::fixed_text(
                9,
                TextPadding::Unknown,
                Some(TextCharset::Ascii),
                StoredClass::Bytes,
            ),
        )
        .with_issue("the content could not be read in full: the binding returns a fixed-length string only up to its first zero byte");
        extra.set_attribute("a_cut_string", cut).unwrap();

        let number = extra.attribute("a_number").unwrap();
        assert_eq!(number.value, AttrValue::Signed(vec![7]));
        assert_eq!(number.stored_as.bits, Some(32));
        assert_eq!(number.stored_as.signed, Some(true));
        assert_eq!(number.stored_as.shape, AttrShape::Array { elements: 1 });
        assert_eq!(number.issue, None);
        // the same value stored as a scalar is a different stored form, and the Model keeps the difference
        let scalar = StoredAs::number(StoredClass::Integer, 32, Some(true));
        assert_ne!(number.stored_as, scalar);

        let cut = extra.attribute("a_cut_string").unwrap();
        assert_eq!(cut.stored_as.fixed_bytes, Some(9));
        assert_eq!(cut.stored_as.padding, Some(TextPadding::Unknown));
        assert!(cut
            .issue
            .as_deref()
            .unwrap()
            .contains("could not be read in full"));

        // listed in name order, and a name that is set again replaces the old attribute and returns it
        let names: Vec<&str> = extra.attributes().map(|(name, _)| name).collect();
        assert_eq!(names, ["a_cut_string", "a_number", "a_text"]);
        let replaced = extra
            .set_attribute("a_number", int32_array_of_one(8))
            .unwrap();
        assert_eq!(replaced.unwrap().value, AttrValue::Signed(vec![7]));
        assert_eq!(extra.len(), 3);
        assert_eq!(extra.attribute("missing"), None);
    }

    #[test]
    fn an_unmapped_node_is_kept_by_path_with_its_shape_and_type() {
        let mut extra = Extra::new();
        // a group that holds nothing the Model types, with an attribute of its own
        let mut group = UnknownNode::group(path(&["unmapped", "group"]));
        group.set_attribute("note", text("kept")).unwrap();
        extra.add_node(group);
        // a dataset: its extents and element type are kept, never its data
        let dataset = UnknownNode::dataset(
            path(&["unmapped", "table"]),
            Dims::new(&[257, 257, 2]).unwrap(),
            StoredAs::number(StoredClass::Float, 64, None),
        );
        extra.add_node(dataset);

        let group = extra.node(&path(&["unmapped", "group"])).unwrap();
        assert_eq!(group.kind, NodeKind::Group);
        assert_eq!(group.element, None);
        assert_eq!(group.dims.rank(), 0);
        assert_eq!(
            group.attribute("note").unwrap().value,
            AttrValue::Text(vec!["kept".into()])
        );

        let dataset = extra.node(&path(&["unmapped", "table"])).unwrap();
        assert_eq!(dataset.kind, NodeKind::Dataset);
        assert_eq!(dataset.dims.as_slice(), [257, 257, 2]);
        assert_eq!(dataset.element.as_ref().unwrap().bits, Some(64));

        // in path order; a node at the same path replaces the old one
        let paths: Vec<String> = extra.nodes().map(|n| n.path.join("/")).collect();
        assert_eq!(paths, ["unmapped/group", "unmapped/table"]);
        let again = UnknownNode::group(path(&["unmapped", "table"]));
        assert_eq!(extra.add_node(again).unwrap().kind, NodeKind::Dataset);
        assert_eq!(extra.len(), 2);
        assert_eq!(extra.node(&path(&["nothing"])), None);
    }

    #[test]
    fn extra_never_holds_bulk_data() {
        let mut extra = Extra::new();
        // an attribute at the limit is kept; one byte over is not, and it is a validation error that names the attribute
        let at_limit = ExtraAttribute::new(
            AttrValue::Bytes(vec![vec![0u8; MAX_ATTRIBUTE_BYTES]]),
            StoredAs::new(StoredClass::Bytes),
        );
        assert!(extra.set_attribute("at_the_limit", at_limit).is_ok());
        let over = ExtraAttribute::new(
            AttrValue::Bytes(vec![vec![0u8; MAX_ATTRIBUTE_BYTES + 1]]),
            StoredAs::new(StoredClass::Bytes),
        );
        let error = extra.set_attribute("too_big", over).unwrap_err();
        match error.kind() {
            ErrorKind::ValidationError { violations } => {
                assert!(violations[0].message.contains("too_big"));
            }
            other => panic!("expected a validation error, got {other:?}"),
        }
        assert!(extra.attribute("too_big").is_none());
        // numbers count as eight bytes each: more than the limit's worth of numbers is refused too
        let many = ExtraAttribute::new(
            AttrValue::Float(vec![0.0; MAX_ATTRIBUTE_BYTES / 8 + 1]),
            StoredAs::number(StoredClass::Float, 64, None),
        );
        assert!(extra.set_attribute("many_numbers", many).is_err());
        // the same limit holds for the attributes of an unmapped node
        let mut node = UnknownNode::group(path(&["g"]));
        let huge = ExtraAttribute::new(
            AttrValue::Text(vec!["x".repeat(MAX_ATTRIBUTE_BYTES + 1)]),
            StoredAs::variable_text(None),
        );
        assert!(node.set_attribute("huge", huge).is_err());
        // a shape has at most MAX_DIMS extents, and a dataset's data has no place in a node at all
        assert!(Dims::new(&vec![1; MAX_DIMS]).is_ok());
        assert!(Dims::new(&vec![1; MAX_DIMS + 1]).is_err());
    }

    #[test]
    fn extra_compares_equal_after_a_clone() {
        let mut extra = Extra::new();
        extra.set_attribute("a", int32_array_of_one(1)).unwrap();
        let mut node = UnknownNode::dataset(
            path(&["d"]),
            Dims::new(&[3]).unwrap(),
            StoredAs::number(StoredClass::Integer, 8, Some(false)),
        );
        node.set_attribute("b", text("x")).unwrap();
        extra.add_node(node);
        let copy = extra.clone();
        assert_eq!(copy, extra);
        // and a difference anywhere is seen: the stored form, the value, an issue, a node's extent
        let mut other = extra.clone();
        other
            .set_attribute("a", int32_array_of_one(1).with_issue("changed"))
            .unwrap();
        assert_ne!(other, extra);
        let mut other = extra.clone();
        other.add_node(UnknownNode::dataset(
            path(&["d"]),
            Dims::new(&[4]).unwrap(),
            StoredAs::number(StoredClass::Integer, 8, Some(false)),
        ));
        assert_ne!(other, extra);
        assert_eq!(Extra::new(), Extra::default());
    }
}
