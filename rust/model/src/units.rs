//! Units and quantities: the `Quantity` of a number with its unit, the controlled vocabulary of units and the as-written spelling (decision D02-3).
//!
//! * **Layer:** the Model: plain types, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** producers (`Quantity`, `Unit`); the enums `Dimension` and `UnitSource` are non-exhaustive, and `Dimension` is open.
//! * **Requirement:** BL-CD-06 (every dimensional quantity carries a unit or says it has none), BL-CD-07 (absent is absent).
//!
//! # What this module claims, and what it does not
//!
//! * **A unit is reported when the file records one, in whatever form its dialect uses** (an attribute such as `_unit` or `_units`, or a unit written
//!   into the value, as in `'10us'`). Otherwise the unit is **unknown**. Nothing is guessed: an empty unit, a spelling that is not in the vocabulary
//!   (`'10ums'`) and a number with no unit at all (a sample period of `5.0`) all stay unknown, and the spelling is kept.
//! * **The vocabulary normalises spellings, never values.** `V` and `Volts` are one unit with two spellings, and the micro sign (U+00B5), the Greek mu
//!   (U+03BC) and ASCII `u` are one prefix. **Nothing here converts a value**: reading `10` microseconds gives `10` microseconds. Converting is a separate,
//!   explicit step that comes later (the processing stages and the version converters), and the dimension and canonical unit recorded here are what it
//!   will use to check a request.
//! * **Where a unit did not come from the file** it says so, in [`UnitSource`]: a format convention that is documented for a dialect is
//!   `DeclaredByAdapter` (the first uses are epoch units and the stored units of raw channels), and a guess is `Inferred`. Adapters do not use these two
//!   until the field inventory (phase 2J) lists which fields they apply to.
//!
//! The vocabulary is a table (`VOCABULARY`): adding a unit is adding a row.

/// The kind of thing a unit measures. A unit of one dimension can be converted to another unit of the same dimension, and never to one of another.
// open-domain: dimensions that files or consumers may bring that nobody has listed yet
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Dimension {
    Time,
    Length,
    Angle,
    ElectricPotential,
    ElectricCurrent,
    Power,
    /// A dimensionless ratio, such as parts per million.
    Ratio,
    /// Counts of a converter or encoder, such as a position in `bits`. This is not an amount of information.
    RawCounts,
    /// An amount of stored information, such as the size of a file in `bytes`.
    Information,
    /// A dimension that the vocabulary does not know, by name.
    Other(String),
}

impl Dimension {
    /// The id of the one canonical unit of this dimension, the unit that values are compared in (BL-CD-06). No value is converted to it on read.
    pub fn canonical_unit_id(&self) -> Option<&'static str> {
        VOCABULARY
            .iter()
            .find(|entry| entry.canonical && entry.dimension == *self)
            .map(|entry| entry.id)
    }
}

/// Where the unit of a quantity came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UnitSource {
    /// The file records the unit.
    FromFile,
    /// The file does not, and the format documents it for this dialect (in the dialect's manifest).
    DeclaredByAdapter,
    /// Neither, and the unit is a reasoned guess.
    Inferred,
    /// There is no unit, or it is empty, or it is not in the vocabulary.
    Unknown,
}

/// A unit: the normalised id from the vocabulary, and the spelling the file used.
///
/// The id drives comparison (`V` and `Volts` have the id `V`); the spelling lets a file written again in the same version keep `Volts` as `Volts`.
// constructed-by: producer
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Unit {
    /// The id in the vocabulary, or `None` when the unit is not known.
    pub id: Option<String>,
    /// The spelling as written (an empty string if the file wrote an empty unit), or `None` when the file wrote none.
    pub spelling: Option<String>,
}

impl Unit {
    /// A unit the file did not write.
    pub fn unknown() -> Self {
        Unit {
            id: None,
            spelling: None,
        }
    }

    /// Reads a unit as a file wrote it. A spelling in the vocabulary gets its id; any other (including an empty one) is unknown, and is kept.
    pub fn parse(text: &str) -> Self {
        let id = entry_for(text).map(|entry| entry.id.to_string());
        Unit {
            id,
            spelling: Some(text.to_string()),
        }
    }

    /// Is the unit in the vocabulary?
    pub fn is_known(&self) -> bool {
        self.id.is_some()
    }

    /// What the unit measures, when it is known.
    pub fn dimension(&self) -> Option<Dimension> {
        let id = self.id.as_deref()?;
        VOCABULARY
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.dimension.clone())
    }

    /// Are both the same known unit, whatever their spellings? An unknown unit is not the same as anything, not even another unknown one.
    pub fn is_same_unit(&self, other: &Unit) -> bool {
        matches!((&self.id, &other.id), (Some(a), Some(b)) if a == b)
    }
}

/// A number with its unit, and where the unit came from.
// constructed-by: producer
#[derive(Debug, Clone, PartialEq)]
pub struct Quantity {
    pub value: f64,
    pub unit: Unit,
    pub unit_source: UnitSource,
}

impl Quantity {
    /// A value as a file gave it: `unit` is the text of the unit attribute, or `None` if the file has none. A unit in the vocabulary is
    /// [`UnitSource::FromFile`]; anything else (none, empty, or a spelling nobody has listed) is [`UnitSource::Unknown`], with the spelling kept.
    /// **The value is never changed.**
    pub fn from_file(value: f64, unit: Option<&str>) -> Self {
        match unit {
            None => Quantity {
                value,
                unit: Unit::unknown(),
                unit_source: UnitSource::Unknown,
            },
            Some(text) => {
                let unit = Unit::parse(text);
                let unit_source = if unit.is_known() {
                    UnitSource::FromFile
                } else {
                    UnitSource::Unknown
                };
                Quantity {
                    value,
                    unit,
                    unit_source,
                }
            }
        }
    }
}

/// Reads a number with its unit written into one string, as in a sample period of `'10us'`. `None` if the text does not start with a number.
///
/// A number alone (`5.0`) has an unknown unit; a unit the vocabulary does not have (`'10ums'`) is unknown too, and its spelling is kept. Neither is
/// guessed to be microseconds or milliseconds. The value is never changed.
pub fn parse_embedded(text: &str) -> Option<Quantity> {
    let trimmed = text.trim();
    let number_len = leading_number_len(trimmed)?;
    let value: f64 = trimmed[..number_len].parse().ok()?;
    let unit_text = trimmed[number_len..].trim();
    if unit_text.is_empty() {
        return Some(Quantity::from_file(value, None));
    }
    Some(Quantity::from_file(value, Some(unit_text)))
}

/// The byte length of the number at the start of `text` (an optional sign, digits with an optional fraction, an optional exponent), if there is one.
fn leading_number_len(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let digits_from = i;
    while bytes.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let mut mantissa_digits = i - digits_from;
    if bytes.get(i) == Some(&b'.') {
        let fraction_from = i + 1;
        let mut j = fraction_from;
        while bytes.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
        if j > fraction_from || mantissa_digits > 0 {
            mantissa_digits += j - fraction_from;
            i = j;
        }
    }
    if mantissa_digits == 0 {
        return None;
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        let exponent_from = j;
        while bytes.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
        if j > exponent_from {
            i = j;
        }
    }
    Some(i)
}

// ---------------------------------------------------------------- the vocabulary

/// One row of the vocabulary.
struct Entry {
    /// The normalised id.
    id: &'static str,
    dimension: Dimension,
    /// Is this the canonical unit of its dimension (exactly one entry of each dimension is)?
    canonical: bool,
    /// Every spelling that means this unit. The micro prefix is written `u` here: U+00B5 and U+03BC are read as `u` before the lookup.
    spellings: &'static [&'static str],
}

/// The controlled vocabulary: the units seen in the fixtures and the SI ones the plan lists (IP-02 question Q2-1). Matching is exact: `mW` is not `MW`.
/// To add a unit, add a row; nothing else changes.
#[rustfmt::skip]
static VOCABULARY: [Entry; 16] = [
    Entry { id: "s", dimension: Dimension::Time, canonical: true, spellings: &["s", "sec", "second", "seconds"] },
    Entry { id: "ms", dimension: Dimension::Time, canonical: false, spellings: &["ms"] },
    Entry { id: "us", dimension: Dimension::Time, canonical: false, spellings: &["us", "usec"] },
    Entry { id: "ns", dimension: Dimension::Time, canonical: false, spellings: &["ns"] },
    Entry { id: "mm", dimension: Dimension::Length, canonical: true, spellings: &["mm"] },
    Entry { id: "um", dimension: Dimension::Length, canonical: false, spellings: &["um"] },
    Entry { id: "nm", dimension: Dimension::Length, canonical: false, spellings: &["nm"] },
    Entry { id: "deg", dimension: Dimension::Angle, canonical: true, spellings: &["deg", "degree", "degrees"] },
    Entry { id: "V", dimension: Dimension::ElectricPotential, canonical: true, spellings: &["V", "Volt", "Volts", "volt", "volts"] },
    Entry { id: "A", dimension: Dimension::ElectricCurrent, canonical: true, spellings: &["A"] },
    Entry { id: "mA", dimension: Dimension::ElectricCurrent, canonical: false, spellings: &["mA"] },
    Entry { id: "W", dimension: Dimension::Power, canonical: true, spellings: &["W", "Watt", "Watts", "watt", "watts"] },
    Entry { id: "mW", dimension: Dimension::Power, canonical: false, spellings: &["mW"] },
    Entry { id: "ppm", dimension: Dimension::Ratio, canonical: true, spellings: &["ppm"] },
    Entry { id: "bits", dimension: Dimension::RawCounts, canonical: true, spellings: &["bits", "bit"] },
    Entry { id: "bytes", dimension: Dimension::Information, canonical: true, spellings: &["bytes", "byte"] },
];

/// The text as it is looked up: trimmed, with the micro sign and the Greek mu written as `u`.
fn lookup_key(text: &str) -> String {
    text.trim()
        .chars()
        .map(|c| {
            if matches!(c, '\u{00B5}' | '\u{03BC}') {
                'u'
            } else {
                c
            }
        })
        .collect()
}

fn entry_for(text: &str) -> Option<&'static Entry> {
    let key = lookup_key(text);
    VOCABULARY
        .iter()
        .find(|entry| entry.spellings.contains(&key.as_str()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    const MICRO_SIGN: &str = "\u{00B5}";
    const GREEK_MU: &str = "\u{03BC}";

    #[test]
    fn volts_and_v_normalise_to_one_unit_and_keep_the_spelling() {
        let volts = Unit::parse("Volts");
        let v = Unit::parse("V");
        assert_eq!(volts.id.as_deref(), Some("V"));
        assert_eq!(v.id.as_deref(), Some("V"));
        assert!(volts.is_same_unit(&v));
        // each keeps what the file wrote, so a file written again in the same version says `Volts` where it said `Volts`
        assert_eq!(volts.spelling.as_deref(), Some("Volts"));
        assert_eq!(v.spelling.as_deref(), Some("V"));
        assert_ne!(volts, v, "the two are the same unit, but not the same text");
        let q = Quantity::from_file(3.3, Some("Volts"));
        assert_eq!(q.unit_source, UnitSource::FromFile);
        assert_eq!(q.unit.spelling.as_deref(), Some("Volts"));
        assert_eq!(q.unit.dimension(), Some(Dimension::ElectricPotential));
        // and watts the same way
        assert!(Unit::parse("Watts").is_same_unit(&Unit::parse("W")));
    }

    #[test]
    fn micro_sign_greek_mu_and_ascii_u_are_one_unit() {
        let spellings = [
            format!("{MICRO_SIGN}m"),
            format!("{GREEK_MU}m"),
            "um".to_string(),
        ];
        let units: Vec<Unit> = spellings.iter().map(|s| Unit::parse(s)).collect();
        for (unit, spelling) in units.iter().zip(&spellings) {
            assert_eq!(unit.id.as_deref(), Some("um"), "{spelling:?}");
            assert_eq!(unit.spelling.as_deref(), Some(spelling.as_str()));
        }
        assert!(units[0].is_same_unit(&units[1]) && units[1].is_same_unit(&units[2]));
        assert_eq!(units[0].dimension(), Some(Dimension::Length));
        // the prefix works for seconds as well, and does not turn an unrelated letter into a unit
        assert_eq!(
            Unit::parse(&format!("{GREEK_MU}s")).id.as_deref(),
            Some("us")
        );
        assert!(!Unit::parse("xm").is_known());
    }

    #[test]
    fn empty_opcua_unit_is_unknown() {
        let q = Quantity::from_file(1.0, Some(""));
        assert_eq!(q.unit.id, None);
        assert_eq!(
            q.unit.spelling.as_deref(),
            Some(""),
            "an empty unit is not the same as no unit: the file wrote it"
        );
        assert_eq!(q.unit_source, UnitSource::Unknown);
        assert_eq!(q.value, 1.0);
        assert!(!Unit::parse("   ").is_known());
        assert!(q.unit.dimension().is_none());
    }

    #[test]
    fn clearbox_sample_period_5_and_1e_5_stay_unknown() {
        for value in [5.0, 1e-5] {
            let q = Quantity::from_file(value, None);
            assert_eq!(q.unit, Unit::unknown());
            assert_eq!(q.unit_source, UnitSource::Unknown);
            assert_eq!(
                q.value.to_bits(),
                value.to_bits(),
                "the number is kept exactly"
            );
        }
        // written into a string, with no unit, they are the same
        for (text, value) in [("5.0", 5.0), ("1e-5", 1e-5)] {
            let q = parse_embedded(text).expect("a number");
            assert_eq!((q.value, q.unit_source), (value, UnitSource::Unknown));
            assert_eq!(q.unit.spelling, None);
        }
    }

    #[test]
    fn embedded_10us_parses_and_10ums_is_unknown_with_raw_kept() {
        let us = parse_embedded("10us").expect("a number and a unit");
        assert_eq!(us.value, 10.0);
        assert_eq!(us.unit.id.as_deref(), Some("us"));
        assert_eq!(us.unit_source, UnitSource::FromFile);
        assert_eq!(
            parse_embedded("10 us"),
            Some(us.clone()),
            "a space between them does not matter"
        );
        assert_eq!(
            parse_embedded(&format!("10{GREEK_MU}s")).map(|q| q.unit.id),
            Some(Some("us".to_string()))
        );

        let ums = parse_embedded("10ums").expect("a number and something");
        assert_eq!(ums.value, 10.0);
        assert_eq!(
            ums.unit.id, None,
            "`ums` is neither microseconds nor milliseconds: it is not guessed"
        );
        assert_eq!(
            ums.unit.spelling.as_deref(),
            Some("ums"),
            "what was written is kept"
        );
        assert_eq!(ums.unit_source, UnitSource::Unknown);

        // text that does not start with a number is not a quantity
        for text in ["", "us", "fast", "-", ".", "e5"] {
            assert_eq!(parse_embedded(text), None, "{text:?}");
        }
        // signs, fractions and exponents are part of the number; an exponent with no digits is not
        assert_eq!(
            parse_embedded("-2.5e3ms").map(|q| (q.value, q.unit.id)),
            Some((-2500.0, Some("ms".to_string())))
        );
        assert_eq!(
            parse_embedded("1e").map(|q| (q.value, q.unit.spelling)),
            Some((1.0, Some("e".to_string())))
        );
    }

    #[test]
    fn unit_conversion_is_never_applied_on_read() {
        // 10 microseconds is read as 10 microseconds, not as 1e-5 seconds
        let q = Quantity::from_file(10.0, Some("us"));
        assert_eq!(q.value, 10.0);
        assert_eq!(q.unit.id.as_deref(), Some("us"));
        // the same for every spelling in the vocabulary, as an attribute and as an embedded unit, bit for bit
        let value = 7.25_f64;
        for entry in &VOCABULARY {
            for spelling in entry.spellings.iter().copied() {
                let from_attribute = Quantity::from_file(value, Some(spelling));
                assert_eq!(
                    from_attribute.value.to_bits(),
                    value.to_bits(),
                    "{spelling}"
                );
                assert_eq!(from_attribute.unit.id.as_deref(), Some(entry.id));
                let embedded = parse_embedded(&format!("7.25{spelling}")).expect("a quantity");
                assert_eq!(embedded.value.to_bits(), value.to_bits(), "7.25{spelling}");
            }
        }
        // the canonical unit is only a statement for comparison: a quantity in milliseconds is not turned into it
        assert_eq!(Dimension::Time.canonical_unit_id(), Some("s"));
        assert_eq!(Quantity::from_file(500.0, Some("ms")).value, 500.0);
    }

    // ---------------------------------------------------------------- the table is checked

    #[test]
    fn no_spelling_and_no_id_is_listed_twice() {
        let mut spellings = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for entry in &VOCABULARY {
            assert!(ids.insert(entry.id), "the id {} is listed twice", entry.id);
            for spelling in entry.spellings {
                assert!(
                    spellings.insert(*spelling),
                    "the spelling {spelling:?} is listed twice"
                );
                assert_eq!(
                    lookup_key(spelling),
                    *spelling,
                    "{spelling:?} must be written in its looked-up form (the micro prefix as `u`)"
                );
            }
        }
    }

    #[test]
    fn every_dimension_in_the_vocabulary_has_exactly_one_canonical_unit_and_it_is_in_it() {
        let dimensions: BTreeSet<String> = VOCABULARY
            .iter()
            .map(|e| format!("{:?}", e.dimension))
            .collect();
        for entry in &VOCABULARY {
            let canonical: Vec<&str> = VOCABULARY
                .iter()
                .filter(|e| e.dimension == entry.dimension && e.canonical)
                .map(|e| e.id)
                .collect();
            assert_eq!(
                canonical.len(),
                1,
                "{:?} has canonical units {canonical:?}",
                entry.dimension
            );
            assert_eq!(entry.dimension.canonical_unit_id(), Some(canonical[0]));
        }
        assert_eq!(dimensions.len(), 9, "nine dimensions today: {dimensions:?}");
        assert_eq!(Dimension::Other("thing".into()).canonical_unit_id(), None);
    }

    #[test]
    fn matching_is_exact_and_an_unknown_unit_is_not_the_same_as_anything() {
        assert!(Unit::parse("mW").is_known());
        assert!(
            !Unit::parse("MW").is_known(),
            "case matters: a megawatt is not a milliwatt"
        );
        assert!(!Unit::parse("kV").is_known());
        let (a, b) = (Unit::unknown(), Unit::unknown());
        assert!(!a.is_same_unit(&b));
        assert!(!a.is_same_unit(&Unit::parse("V")));
        // the units seen in the fixtures are all there
        for seen in [
            "mm", "nm", "degrees", "Volts", "V", "Watts", "mW", "mA", "ppm", "bytes", "bits",
        ] {
            assert!(Unit::parse(seen).is_known(), "{seen}");
        }
        // time units of the same dimension, and a power is not a voltage
        assert_eq!(Unit::parse("ns").dimension(), Some(Dimension::Time));
        assert_ne!(Unit::parse("W").dimension(), Unit::parse("V").dimension());
    }
}
