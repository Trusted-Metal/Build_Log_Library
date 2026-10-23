//! Keeping what an adapter does not type (IP-13 phase 13B; requirement BL-CD-08): an attribute becomes an entry of the Model's `Extra` with the form it was
//! stored in, an attribute of a class the primitives do not read is kept with the reason, one too large to be metadata is kept by its size, and the
//! attributes that the adapter types are not kept twice.

mod common;

use build_log_io::hdf5::keep_other_attributes;
use build_log_model::{
    AttrShape, AttrValue, Extra, StoredClass, TextCharset, TextPadding, MAX_ATTRIBUTE_BYTES,
};
use build_log_testkit::{miniature, miniature_path};
use common::{locate, put_array, put_scalar, put_text, temp_h5};
use hdf5::types::FixedAscii;

fn kept(label: &str, typed: &[&str], write: impl FnOnce(&hdf5::Group)) -> Extra {
    let on_disk = temp_h5(label, write);
    let file = hdf5::File::open(&on_disk.0).expect("the file opens");
    let mount = file.group("Mount").expect("the mount");
    let mut extra = Extra::new();
    keep_other_attributes(&mount, typed, &mut extra).expect("the attributes are kept");
    extra
}

#[test]
fn a_stored_value_keeps_its_encoding_in_extra() {
    let extra = kept("extra-forms", &[], |g| {
        put_array(g, "i32_one", &[7i32]);
        put_scalar(g, "u8_scalar", 200u8);
        put_scalar(g, "f32_scalar", 0.5f32);
        put_text(g, "variable", "as written");
        put_scalar(g, "fixed", FixedAscii::<9>::from_ascii(b"IN718").unwrap());
    });
    assert_eq!(extra.len(), 5);

    // a number keeps its width and sign, and an array of one stays apart from a scalar
    let one = extra.attribute("i32_one").unwrap();
    assert_eq!(one.value, AttrValue::Signed(vec![7]));
    assert_eq!(
        (
            one.stored_as.class.clone(),
            one.stored_as.bits,
            one.stored_as.signed
        ),
        (StoredClass::Integer, Some(32), Some(true))
    );
    assert_eq!(one.stored_as.shape, AttrShape::Array { elements: 1 });
    let byte = extra.attribute("u8_scalar").unwrap();
    assert_eq!(byte.value, AttrValue::Unsigned(vec![200]));
    assert_eq!(
        (byte.stored_as.bits, byte.stored_as.signed),
        (Some(8), Some(false))
    );
    assert_eq!(byte.stored_as.shape, AttrShape::Scalar);
    let float = extra.attribute("f32_scalar").unwrap();
    assert_eq!(float.value, AttrValue::Float(vec![0.5]));
    assert_eq!(
        (float.stored_as.class.clone(), float.stored_as.bits),
        (StoredClass::Float, Some(32))
    );

    // text keeps whether it had a length of its own, and a fixed one keeps its length and padding
    let variable = extra.attribute("variable").unwrap();
    assert_eq!(variable.value, AttrValue::Text(vec!["as written".into()]));
    assert_eq!(variable.stored_as.fixed_bytes, None);
    let fixed = extra.attribute("fixed").unwrap();
    assert_eq!(fixed.value, AttrValue::Text(vec!["IN718".into()]));
    assert_eq!(fixed.stored_as.fixed_bytes, Some(9));
    assert_eq!(fixed.stored_as.charset, Some(TextCharset::Ascii));
    assert!(matches!(
        fixed.stored_as.padding,
        Some(TextPadding::NullPadded | TextPadding::NullTerminated | TextPadding::Unknown)
    ));
    // nothing here is the whole of the matter that an issue would describe
    assert!(extra.attributes().all(|(_, a)| a.issue.is_none()));
}

#[test]
fn the_attributes_the_adapter_types_are_not_kept_twice() {
    let extra = kept("extra-typed", &["typed_a", "typed_b", "absent"], |g| {
        put_scalar(g, "typed_a", 1i32);
        put_scalar(g, "typed_b", 2i32);
        put_scalar(g, "other", 3i32);
    });
    let names: Vec<&str> = extra.attributes().map(|(name, _)| name).collect();
    assert_eq!(names, ["other"]);
}

#[test]
fn an_attribute_of_a_class_that_is_not_read_is_kept_with_the_reason() {
    // a boolean is an enumeration to HDF5, a class the primitives do not read
    let extra = kept("extra-unread", &[], |g| {
        put_scalar(g, "flag", true);
        put_scalar(g, "number", 5i32);
    });
    let flag = extra.attribute("flag").expect("the attribute is kept");
    assert_eq!(flag.value, AttrValue::Bytes(Vec::new()));
    assert_eq!(flag.stored_as.class, StoredClass::Other("not read".into()));
    let issue = flag.issue.as_deref().expect("it says why");
    assert!(issue.contains("not read"), "{issue}");
    // the others are not affected
    assert_eq!(
        extra.attribute("number").unwrap().value,
        AttrValue::Signed(vec![5])
    );
}

#[test]
fn an_attribute_too_large_to_be_metadata_is_kept_by_its_size_and_not_its_content() {
    // a number counts as eight bytes in the Model, so this is more than the limit although it is 9,000 bytes on disk
    let elements = MAX_ATTRIBUTE_BYTES / 8 + 1000;
    let extra = kept("extra-large", &[], |g| {
        put_array(g, "large", &vec![0u8; elements]);
        put_scalar(g, "small", 1u8);
    });
    let large = extra.attribute("large").expect("it is kept as an entry");
    assert_eq!(large.value, AttrValue::Bytes(Vec::new()));
    let issue = large.issue.as_deref().expect("it says why");
    assert!(
        issue.contains(&format!("{} bytes", elements * 8)),
        "{issue}"
    );
    assert!(issue.contains(&MAX_ATTRIBUTE_BYTES.to_string()), "{issue}");
    assert_eq!(
        extra.attribute("small").unwrap().value,
        AttrValue::Unsigned(vec![1])
    );
}

#[test]
fn the_image_specification_attributes_of_a_real_file_are_kept() {
    // the formula image of the gate file holds the attribute of the image specification: an element that is an array of 64 `uint8`
    let file = hdf5::File::open(miniature_path(miniature::V17_TRIGGERED_2026_10_19))
        .expect("the miniature");
    let image = locate(
        &file,
        "/Built_Part/Calibration_Data/BitsToMillimetersFormula",
    );
    let mut extra = Extra::new();
    keep_other_attributes(&image, &[], &mut extra).expect("the attributes are kept");
    let class = extra.attribute("CLASS").expect("the attribute CLASS");
    match &class.value {
        AttrValue::Unsigned(v) => assert_eq!(v.len(), 64),
        other => panic!("expected 64 unsigned numbers, got {other:?}"),
    }
    assert_eq!(
        (class.stored_as.class.clone(), class.stored_as.bits),
        (StoredClass::Integer, Some(8))
    );
    match &class.stored_as.shape {
        AttrShape::Other(words) => assert!(words.contains("64"), "{words}"),
        other => panic!("the shape of an array element must be said in words, got {other:?}"),
    }
    assert!(class.issue.is_none());
    // the attribute that is a plain number is kept as one
    let white = extra
        .attribute("IMAGE_WHITE_IS_ZERO")
        .expect("the attribute IMAGE_WHITE_IS_ZERO");
    assert_eq!(white.stored_as.bits, Some(8));
}
