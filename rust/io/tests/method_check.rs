//! The method check of IP-03 phase 3D (step 1, the former spike S2): what the binding's **safe** API gives for the awkward kinds of Appendix C.
//!
//! The ladder of decision D03-4 is: (1) the typed API, (2) its lower-level calls, (3) calls into `hdf5-metno-sys` in one module with a scoped exception to the
//! ban on unsafe code (question Q3-1). These tests record what steps 1 and 2 can and cannot do on the committed miniatures, on every libhdf5 of the CI legs:
//!
//! * every attribute of every miniature has a datatype that the binding can describe (class, width, sign, character set, array dimensions);
//! * the byte order and the "is it null-padded" question can be answered, but **no more of the padding**: the binding builds fixed-length strings null-padded
//!   and has no call that says how an existing one is padded, so a null-terminated string and a space-padded one look the same;
//! * **a null-terminated fixed-length string is cut at its first zero byte**: the binding converts it into its own null-padded type on the way in, so the bytes
//!   after the first zero are lost, and a number stored as raw bytes (V17 March's `Power_Min`) that has a zero byte inside reads back empty. Reading the bytes
//!   exactly needs a read in the file's own type, which the safe API does not offer: that is step 3 of the ladder.

use std::path::PathBuf;

mod common;

use build_log_testkit::{miniature, miniature_path};
use common::{locations, MINIATURES};
use hdf5::types::{FixedAscii, FixedUnicode, TypeDescriptor, VarLenUnicode};
use hdf5::{Datatype, File, H5Type};

fn is_null_padded_ascii(datatype: &Datatype) -> bool {
    Datatype::from_descriptor(&TypeDescriptor::FixedAscii(datatype.size()))
        .map(|candidate| &candidate == datatype)
        .unwrap_or(false)
}

#[test]
fn every_attribute_of_every_miniature_has_a_datatype_the_binding_can_describe() {
    let mut attributes = 0;
    for name in MINIATURES {
        let file = File::open(miniature_path(name)).expect("a miniature opens");
        for (_path, location) in locations(&file) {
            for attr_name in location.attr_names().expect("attribute names") {
                let attr = location.attr(&attr_name).expect("an attribute opens");
                let datatype = attr.dtype().expect("an attribute has a datatype");
                let descriptor = datatype
                    .to_descriptor()
                    .unwrap_or_else(|e| panic!("{name}: `{attr_name}` has no descriptor: {e}"));
                // one dimension at most: a scalar, or an array of the stored shape
                assert!(
                    attr.shape().len() <= 1,
                    "{name}: `{attr_name}` has shape {:?}",
                    attr.shape()
                );
                // the kinds that Appendix B lists, and no other
                assert!(
                    matches!(
                        descriptor,
                        TypeDescriptor::Integer(_)
                            | TypeDescriptor::Unsigned(_)
                            | TypeDescriptor::Float(_)
                            | TypeDescriptor::FixedAscii(_)
                            | TypeDescriptor::VarLenUnicode
                            | TypeDescriptor::FixedArray(..)
                    ),
                    "{name}: `{attr_name}` is {descriptor:?}"
                );
                attributes += 1;
            }
        }
    }
    // the sweep reached every attribute: the total of Appendix B (1,614 attribute instances in the eight miniatures)
    assert_eq!(attributes, 1_614);
}

#[test]
fn the_binding_says_whether_a_fixed_string_is_null_padded_and_nothing_more_about_its_padding() {
    let special = File::open(miniature_path(miniature::V17_SPECIAL_LAYERS))
        .expect("the special-layers miniature");
    let sensors = special
        .group("/Built_Part/Monitoring_Data/Layer_1966/Laser_1/Synchronous_Digital_Sensors")
        .expect("a group with a null-padded attribute");
    let padded = sensors
        .attr("Metadata")
        .expect("the attribute")
        .dtype()
        .expect("its datatype");
    assert_eq!(
        padded.to_descriptor().expect("a descriptor"),
        TypeDescriptor::FixedAscii(8)
    );
    assert!(
        is_null_padded_ascii(&padded),
        "a null-padded string equals the binding's own fixed string"
    );

    let march = File::open(miniature_path(miniature::V17_MARCH)).expect("the March miniature");
    let light_source = march
        .group("/Built_Part/Calibration_Data/Optical_Train_1/Light_Source")
        .expect("the light source group");
    let terminated = light_source
        .attr("Power_Min")
        .expect("the attribute")
        .dtype()
        .expect("its datatype");
    assert_eq!(
        terminated.to_descriptor().expect("a descriptor"),
        TypeDescriptor::FixedAscii(9)
    );
    assert!(
        !is_null_padded_ascii(&terminated),
        "a null-terminated string is not the binding's fixed string"
    );
}

/// A temporary copy of the March miniature in which the first `Power_Min` (the raw bytes of 101.7, then the terminating zero) is replaced by the raw bytes of
/// 100.0: `00 00 00 00 00 00 59 40`, then the terminating zero. The copy is removed when the value is dropped.
struct PatchedMarch(PathBuf);

impl PatchedMarch {
    fn new() -> Self {
        let mut bytes =
            std::fs::read(miniature_path(miniature::V17_MARCH)).expect("the March miniature");
        let old = [0xcd_u8, 0xcc, 0xcc, 0xcc, 0xcc, 0x6c, 0x59, 0x40, 0x00];
        let new = [0x00_u8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x59, 0x40, 0x00];
        let at = bytes
            .windows(old.len())
            .position(|w| w == old)
            .expect("the value is in the file");
        bytes[at..at + old.len()].copy_from_slice(&new);
        let path = std::env::temp_dir().join(format!(
            "build-log-io-method-check-{}.h5",
            std::process::id()
        ));
        std::fs::write(&path, &bytes).expect("a writable temporary folder");
        PatchedMarch(path)
    }
}

impl Drop for PatchedMarch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn the_safe_binding_cuts_a_null_terminated_string_at_its_first_zero_byte() {
    // as the file has it: no zero byte inside, so the eight bytes come back (the terminating zero is already gone)
    let march = File::open(miniature_path(miniature::V17_MARCH)).expect("the March miniature");
    let light_source = march
        .group("/Built_Part/Calibration_Data/Optical_Train_1/Light_Source")
        .expect("the light source group");
    let value = light_source
        .attr("Power_Min")
        .expect("the attribute")
        .read_scalar::<FixedAscii<9>>()
        .expect("a typed read");
    assert_eq!(
        value.as_bytes(),
        [0xcd, 0xcc, 0xcc, 0xcc, 0xcc, 0x6c, 0x59, 0x40]
    );

    // with zero bytes inside (the bytes of 100.0), the typed read returns nothing: the data is lost, and no error says so
    let patched = PatchedMarch::new();
    let file = File::open(&patched.0).expect("the patched copy opens");
    let group = file
        .group("/Built_Part/Calibration_Data/Optical_Train_1/Light_Source")
        .expect("the light source group");
    let value = group
        .attr("Power_Min")
        .expect("the attribute")
        .read_scalar::<FixedAscii<9>>()
        .expect("a typed read");
    assert!(
        value.as_bytes().is_empty(),
        "if the binding now returns the bytes, the shim of Q3-1 is not needed for this kind: got {:02x?}",
        value.as_bytes()
    );
}

#[test]
fn an_attribute_whose_element_is_an_array_reads_with_the_typed_api() {
    // probe 5 of Appendix C: one element that is an array of 64 `uint8` (the HDF5 image specification's `CLASS`, `IMAGE` and zeros)
    let file =
        File::open(miniature_path(miniature::V17_2026_08_18_IMAGES)).expect("the images miniature");
    let dataset = file
        .dataset("/Built_Part/Calibration_Data/BitsToMillimetersFormula")
        .expect("a picture of the calibration data");
    let attr = dataset.attr("CLASS").expect("the attribute");
    assert_eq!(
        attr.dtype()
            .expect("its datatype")
            .to_descriptor()
            .expect("a descriptor"),
        TypeDescriptor::FixedArray(
            Box::new(TypeDescriptor::Unsigned(hdf5::types::IntSize::U1)),
            64
        )
    );
    assert_eq!(attr.shape(), vec![1]);
    let elements = attr.read_raw::<[u8; 64]>().expect("a typed read");
    assert_eq!(elements.len(), 1);
    let mut expected = [0_u8; 64];
    expected[..5].copy_from_slice(b"IMAGE");
    assert_eq!(elements[0], expected);
}

#[derive(H5Type, Clone, Debug, PartialEq)]
#[repr(C)]
struct EquationConstant {
    name: FixedUnicode<64>,
    value: f64,
}

#[test]
fn a_compound_dataset_with_a_fixed_string_field_reads_with_a_derived_type() {
    // probe 6: a compound of a fixed-length name (64 bytes, null-padded, so the typed read is exact) and a `float64`, in the order stored
    let file = File::open(miniature_path(miniature::V18)).expect("the V18 miniature");
    let dataset = file
        .dataset(
            "/Build/Lasers/Laser_1/Corrections/Power_Calibration/Derivation_Equation_Constants",
        )
        .expect("a compound dataset");
    let rows = dataset
        .read_raw::<EquationConstant>()
        .expect("a typed read");
    let seen: Vec<(String, f64)> = rows
        .iter()
        .map(|r| (r.name.as_str().to_string(), r.value))
        .collect();
    assert_eq!(
        seen,
        vec![("b".to_string(), 50.5), ("a".to_string(), 107.5)]
    );
}

#[test]
fn a_variable_length_string_dataset_reads_with_the_typed_api() {
    // probe 7: the values of an OPC UA series, stored as variable-length strings
    let file =
        File::open(miniature_path(miniature::V17_2026_08_18_IMAGES)).expect("the images miniature");
    let dataset = file
        .dataset("/Built_Part/Monitoring_Data/Layer_1/Global_Sensors/sensor_events/Actuators/optical_axis::drive::node/current_velocity/Value")
        .expect("a series of values");
    let values = dataset.read_raw::<VarLenUnicode>().expect("a typed read");
    assert_eq!(values.len(), 72);
    let first: Vec<&str> = values.iter().take(3).map(|v| v.as_str()).collect();
    assert_eq!(first, ["0", "0.013", "0"]);
}
