//! The read selection (IP-27 phase 27C): which samples of a laser a read keeps, and the native index of each one it kept.
//!
//! * **Layer:** the Model: plain types and one pure rule, no I/O, no HDF5, and nothing that names a group or path of a file. **Group (decision G1):** shared.
//! * **Constructed by:** caller-options (`ReadSelection`); library (`SelectedColumn`, `LaserRows`); the enums `StateRows` and `NativeIndex`.
//! * **Requirement:** BL-FA-02, BL-FA-08, BL-CD-09.
//!
//! # What a selection is
//!
//! A selection is **a stride on the native grid, then a filter on the strided rows by the laser's state**, applied to every channel of the laser alike:
//!
//! 1. **The stride.** The samples kept are the ones at native indices `0, s, 2s, ...`. A stride of `0` is a stride of `1` (a caller that computes a stride may pass zero).
//!    A stride beyond the length keeps the one sample at index `0`.
//! 2. **The filter.** [`StateRows::OnOnly`] keeps the strided rows whose state is `1`; [`StateRows::OffOnly`] keeps the others. **A state above `1` counts as off.**
//!    The filter looks at the *strided* rows, not the native ones: with a stride of 2 a laser-on row at an odd native index is never seen.
//! 3. **Every channel of a laser is cut the same way,** so the channels of one laser stay the same length, and **each kept value comes with its native sample index**
//!    ([`SelectedColumn::native_index`]), which is the row it had in the file: the sample's place in time is `native index x the laser's sample period`.
//!
//! **The default selection reads everything** ([`ReadSelection::default`]), and a request that names none is the read that [`crate::ColumnSource::read`] already does.
//!
//! # The answers at the edges
//!
//! | Case | Answer |
//! |---|---|
//! | no rows in the column | no rows kept |
//! | a stride beyond the length | one row, at native index `0` |
//! | a filter that no strided row passes | no rows kept (not an error: the caller decides that a laser with no row is skipped) |
//! | a filter, and the laser has no state channel in the layer | `NotPresent`, naming the state |
//! | a filter, and the state channel is empty | no rows kept |
//! | the state is shorter than the column | the rows past the state's end have no state and are not kept |
//! | the state is longer than the column | its extra rows are ignored |
//! | the state is stored as text, a time or a segment table | `Corrupt` |
//! | a request for a range, a time range, or a build-level channel | `InvalidIndex`: a selection is about the whole stream of a laser |
//!
//! # One channel, or a whole laser
//!
//! * **One channel:** [`crate::BuildSource::read_selected`] chooses the rows and cuts the column in one call.
//! * **A whole laser (decision D27-17):** the rows are a property of the laser, not of a channel, so a consumer that reads several channels chooses them **once**
//!   with [`crate::BuildSource::laser_rows`] (it reads the state, once) and cuts each channel with [`crate::BuildSource::read_rows`]. The rows are shared, not copied: every
//!   channel's [`NativeIndex`] points into the one list. **A channel shorter than the state gets the rows that exist in it**, which is a prefix of the list, so the channels of
//!   a laser stay aligned with their native indices. [`LaserRows`] remembers the layer and the laser it was chosen for, and refuses a request for another, and it holds the state
//!   it read, so that a request for the state channel itself does not read it again.
//!
//! # Cost
//!
//! A selection is **read as a whole column and then cut** (the provided methods are functions of [`crate::ColumnSource::read`] and [`crate::BuildSource::layer_meta`], so every
//! source has them and no adapter changed): a file-backed source decodes the chunks that hold the column either way, and the cut keeps only the rows that were asked for, so a coarse
//! stride returns a small column from a large read. **What a filter adds is one read of the state, one pass over it, and one copy of the kept values of each channel:** with
//! `read_selected` the first two are paid for every channel, with `laser_rows` once for the laser (measured in the 27C record of IP-27).

use crate::channel::ChannelKind;
use crate::column::{
    BuildSource, ColumnData, ColumnRange, ColumnRequest, Precision, ReadContext, Shared,
};
use crate::error::{Error, ErrorKind, Location};
use crate::identity::{ChannelKey, LaserKey, LayerId};

/// Which of the strided rows of a laser a read keeps, by the laser's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum StateRows {
    /// Every strided row. Needs no state.
    #[default]
    All,
    /// The rows whose state is `1`.
    OnOnly,
    /// The rows whose state is not `1` (a state above `1` is off).
    OffOnly,
}

/// What a read keeps of a laser: a stride and a filter by state (see the module documentation). The default keeps every sample.
// constructed-by: caller-options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ReadSelection {
    stride: u64,
    rows: StateRows,
}

impl Default for ReadSelection {
    fn default() -> Self {
        ReadSelection::new()
    }
}

impl ReadSelection {
    /// The selection that keeps everything: a stride of 1 and every row.
    pub fn new() -> Self {
        ReadSelection {
            stride: 1,
            rows: StateRows::All,
        }
    }

    /// Keep the samples at native indices `0, stride, 2*stride, ...`. A stride of `0` is `1`.
    pub fn with_stride(mut self, stride: u64) -> Self {
        self.stride = stride.max(1);
        self
    }

    /// Keep only the strided rows that the state selects.
    pub fn with_rows(mut self, rows: StateRows) -> Self {
        self.rows = rows;
        self
    }

    /// The stride, never below 1.
    pub fn stride(&self) -> u64 {
        self.stride
    }

    pub fn rows(&self) -> StateRows {
        self.rows
    }
}

/// The native sample index of each row that a selection kept, in order. A selection that filters nothing is a stride, which needs no list.
// bulk-container: the native index of each row that a selection kept (a run, or the list when a state filter chose them)
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum NativeIndex {
    /// The rows `0, step, 2*step, ...`: `len` of them.
    Strided { step: u64, len: u64 },
    /// The indices that a filter on the state kept, ascending.
    Listed(Shared<u64>),
}

impl NativeIndex {
    /// How many rows were kept.
    pub fn len(&self) -> usize {
        match self {
            NativeIndex::Strided { len, .. } => usize::try_from(*len).unwrap_or(usize::MAX),
            NativeIndex::Listed(list) => list.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The native index of the `i`-th kept row, or `None` past the last.
    pub fn get(&self, i: usize) -> Option<u64> {
        match self {
            NativeIndex::Strided { step, len } => {
                let i = i as u64;
                (i < *len).then(|| i * step)
            }
            NativeIndex::Listed(list) => list.as_slice().get(i).copied(),
        }
    }

    /// The native indices, in order.
    pub fn iter(&self) -> impl Iterator<Item = u64> + '_ {
        (0..self.len()).filter_map(|i| self.get(i))
    }

    /// The native indices as a list (a copy).
    pub fn to_vec(&self) -> Vec<u64> {
        self.iter().collect()
    }
}

/// The values a selection kept of one column, with the native index of each.
// constructed-by: library
// bulk-container: the values of one column that a selection kept, with their native indices
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SelectedColumn {
    /// The kept values, in the precision that the request asked for.
    pub values: ColumnData,
    /// Where each kept value was in the file: one index for each value.
    pub native_index: NativeIndex,
}

impl SelectedColumn {
    pub fn new(values: ColumnData, native_index: NativeIndex) -> Self {
        SelectedColumn {
            values,
            native_index,
        }
    }

    /// How many values were kept.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

fn invalid(what: &str) -> Error {
    Error::new(ErrorKind::InvalidIndex {
        what: what.to_string(),
    })
}

/// Whether row `i` of a state column says the laser is on (the value `1`; a flag that is set).
fn on_at(state: &ColumnData) -> Result<Box<dyn Fn(usize) -> bool + '_>, Error> {
    macro_rules! whole_numbers {
        ($($variant:ident),+) => {
            match state {
                $(ColumnData::$variant(v) => return Ok(Box::new(move |i| v.as_slice()[i] == 1)),)+
                ColumnData::Bool(v) => return Ok(Box::new(move |i| v.as_slice()[i])),
                _ => {}
            }
        };
    }
    whole_numbers!(U8, I8, U16, I16, U32, I32, U64, I64);
    Err(Error::new(ErrorKind::Corrupt {
        detail: "the laser state is not stored as whole numbers or flags".to_string(),
    }))
}

/// How many of the native indices `0, stride, 2*stride, ...` lie below `rows`.
fn strided_len(rows: usize, stride: usize) -> usize {
    if rows == 0 {
        0
    } else {
        (rows - 1) / stride + 1
    }
}

/// The strided rows, as `u64`, that a filter keeps: the ones whose state is on (`OnOnly`) or not on (`OffOnly`). A row past the end of the state, or of the column of `len`
/// native rows, has nothing to compare with and is not kept.
fn filtered_rows(
    selection: &ReadSelection,
    len: usize,
    state: &ColumnData,
) -> Result<Shared<u64>, Error> {
    let wanted_on = match selection.rows() {
        StateRows::OnOnly => true,
        StateRows::OffOnly => false,
        StateRows::All => return Err(invalid("a filter was asked for with every row kept")),
    };
    let stride = usize::try_from(selection.stride()).unwrap_or(usize::MAX);
    let on = on_at(state)?;
    let rows = len.min(state.len());
    let kept = (0..strided_len(rows, stride))
        .map(|j| j * stride)
        .filter(|native| on(*native) == wanted_on)
        .map(|native| native as u64)
        .collect();
    Ok(Shared::new(kept))
}

fn state_needed() -> Error {
    Error::new(ErrorKind::NotPresent {
        what: "the laser state, which a selection of only laser-on or only laser-off rows needs"
            .to_string(),
    })
}

/// The rows that `selection` keeps of a column of `len` native rows, given the laser's state (`None` when it has none).
///
/// This is the whole rule, with no source and no I/O (see the module documentation for the answers at the edges): the strided rows, then the filter on them.
/// A filter without a state is `NotPresent`; a state that is not whole numbers or flags is `Corrupt`.
pub fn select_rows(
    selection: &ReadSelection,
    len: usize,
    state: Option<&ColumnData>,
) -> Result<NativeIndex, Error> {
    if selection.rows() == StateRows::All {
        let stride = usize::try_from(selection.stride()).unwrap_or(usize::MAX);
        return Ok(NativeIndex::Strided {
            step: selection.stride(),
            len: strided_len(len, stride) as u64,
        });
    }
    let state = state.ok_or_else(state_needed)?;
    Ok(NativeIndex::Listed(filtered_rows(selection, len, state)?))
}

/// The rows that a selection keeps of one laser in one layer, **chosen once** so that each channel of the laser is cut with the same rows (decision D27-17). It is made by
/// [`BuildSource::laser_rows`] and used by [`BuildSource::read_rows`]; see the module documentation.
// constructed-by: library
// bulk-container: the native indices of the rows that a state filter kept, and the state they were chosen from
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LaserRows {
    layer: LayerId,
    laser: LaserKey,
    selection: ReadSelection,
    /// The rows that a state filter kept; `None` for a stride alone, whose rows follow from the length of each column.
    listed: Option<Shared<u64>>,
    /// The state that the rows were chosen from, so that a request for the state channel does not read it again.
    state: Option<(ChannelKey, ColumnData)>,
}

impl LaserRows {
    /// The rows of `selection` in `laser` of `layer`: `listed` when a filter chose them (ascending native indices), `state` the channel they were chosen from and what it held.
    /// [`BuildSource::laser_rows`] makes it; a source that overrides that method makes it with this.
    pub fn new(
        layer: LayerId,
        laser: LaserKey,
        selection: ReadSelection,
        listed: Option<Shared<u64>>,
        state: Option<(ChannelKey, ColumnData)>,
    ) -> Self {
        LaserRows {
            layer,
            laser,
            selection,
            listed,
            state,
        }
    }

    pub fn layer(&self) -> LayerId {
        self.layer
    }

    pub fn laser(&self) -> &LaserKey {
        &self.laser
    }

    pub fn selection(&self) -> &ReadSelection {
        &self.selection
    }

    /// The rows kept of a column of `len` native rows: the strided rows below `len`, or the filter's rows that are below it (a prefix of the list, which shares the list's buffer).
    pub fn native_index(&self, len: usize) -> NativeIndex {
        match &self.listed {
            None => {
                let stride = usize::try_from(self.selection.stride()).unwrap_or(usize::MAX);
                NativeIndex::Strided {
                    step: self.selection.stride(),
                    len: strided_len(len, stride) as u64,
                }
            }
            Some(list) => {
                let below = list
                    .as_slice()
                    .partition_point(|native| *native < len as u64);
                NativeIndex::Listed(
                    list.slice(0..below)
                        .unwrap_or_else(|| Shared::new(Vec::new())),
                )
            }
        }
    }
}

/// The state channel of `laser` in `layer`, and what it holds: found in the metadata of that one layer, and read as stored.
fn read_state<S: BuildSource + ?Sized>(
    source: &S,
    layer: LayerId,
    laser: &LaserKey,
    context: &ReadContext,
    place: &Location,
) -> Result<(ChannelKey, ColumnData), Error> {
    let detail = source.layer_meta(layer)?;
    let not_present = |what: &str| {
        Error::new(ErrorKind::NotPresent {
            what: what.to_string(),
        })
        .at(place.clone())
    };
    let entry = detail
        .laser(laser)
        .ok_or_else(|| not_present("the laser in this layer"))?;
    let state = entry
        .channels
        .iter()
        .find(|channel| channel.kind == ChannelKind::LaserState)
        .ok_or_else(|| {
            not_present(
                "the laser state, which a selection of only laser-on or only laser-off rows needs",
            )
        })?;
    let column = source.read(
        &ColumnRequest::new(layer, Some(laser.clone()), state.key.clone()),
        context,
    )?;
    Ok((state.key.clone(), column))
}

/// Chooses the rows of a laser (the work of [`BuildSource::laser_rows`] and of the first half of [`BuildSource::read_selected`]); `place` is where an error says it was.
fn choose_rows<S: BuildSource + ?Sized>(
    source: &S,
    layer: LayerId,
    laser: &LaserKey,
    selection: &ReadSelection,
    context: &ReadContext,
    place: &Location,
) -> Result<LaserRows, Error> {
    let located = |error: Error| error.at(place.clone());
    context.check_cancelled().map_err(located)?;
    if selection.rows() == StateRows::All {
        // a stride needs nothing from the file: its rows follow from the length of each column
        return Ok(LaserRows::new(layer, laser.clone(), *selection, None, None));
    }
    let (key, state) = read_state(source, layer, laser, context, place)?;
    context.check_cancelled().map_err(located)?;
    let listed = filtered_rows(selection, state.len(), &state).map_err(located)?;
    Ok(LaserRows::new(
        layer,
        laser.clone(),
        *selection,
        Some(listed),
        Some((key, state)),
    ))
}

/// A request that a selection can answer: it names a laser, and the whole stream of it.
fn check_request(request: &ColumnRequest, place: &Location) -> Result<(), Error> {
    if request.laser.is_none() {
        return Err(invalid(
            "a selection is about the rows of a laser, and the request names none",
        )
        .at(place.clone()));
    }
    if request.range != ColumnRange::All {
        return Err(invalid(
            "a selection applies to the whole stream of a laser, and the request names a range of it",
        )
        .at(place.clone()));
    }
    Ok(())
}

/// Cuts the column of `request` with `rows` (the work of [`BuildSource::read_rows`] and of the second half of [`BuildSource::read_selected`]).
fn cut_column<S: BuildSource + ?Sized>(
    source: &S,
    request: &ColumnRequest,
    rows: &LaserRows,
    context: &ReadContext,
    operation: &str,
) -> Result<SelectedColumn, Error> {
    let place = request.location().with_operation(operation);
    let located = |error: Error| error.at(place.clone());
    context.check_cancelled().map_err(located)?;
    check_request(request, &place)?;
    if request.layer != rows.layer || request.laser.as_ref() != Some(&rows.laser) {
        return Err(located(invalid(
            "the rows were chosen for another laser or layer than the request names",
        )));
    }
    if *rows.selection() == ReadSelection::default() {
        // nothing is dropped: the read as it is, with its own precision
        let values = source.read(request, context)?;
        let len = values.len() as u64;
        return Ok(SelectedColumn::new(
            values,
            NativeIndex::Strided { step: 1, len },
        ));
    }
    // the cut is made on the stored values, and only the kept ones are converted; the state is not read again if the rows were chosen from it
    let stored = match &rows.state {
        Some((key, state)) if *key == request.channel => state.clone(),
        _ => source.read(&request.clone().with_precision(Precision::Native), context)?,
    };
    let native_index = rows.native_index(stored.len());
    let values = stored
        .pick(&native_index)
        .ok_or_else(|| located(invalid("a kept row is outside the column")))?
        .with_precision(request.precision)
        .map_err(located)?;
    Ok(SelectedColumn::new(values, native_index))
}

/// The provided [`BuildSource::laser_rows`].
pub(crate) fn laser_rows<S: BuildSource + ?Sized>(
    source: &S,
    layer: LayerId,
    laser: &LaserKey,
    selection: &ReadSelection,
    context: &ReadContext,
) -> Result<LaserRows, Error> {
    let place = Location::new()
        .with_layer(layer)
        .with_laser(laser.clone())
        .with_operation("laser_rows");
    choose_rows(source, layer, laser, selection, context, &place)
}

/// The provided [`BuildSource::read_rows`].
pub(crate) fn read_rows<S: BuildSource + ?Sized>(
    source: &S,
    request: &ColumnRequest,
    rows: &LaserRows,
    context: &ReadContext,
) -> Result<SelectedColumn, Error> {
    cut_column(source, request, rows, context, "read_rows")
}

/// The provided [`BuildSource::read_selected`]: the rows chosen and the column cut in one call.
pub(crate) fn read_selected<S: BuildSource + ?Sized>(
    source: &S,
    request: &ColumnRequest,
    selection: &ReadSelection,
    context: &ReadContext,
) -> Result<SelectedColumn, Error> {
    let place = request.location().with_operation("read_selected");
    check_request(request, &place)?;
    let laser = request
        .laser
        .as_ref()
        .ok_or_else(|| invalid("no laser").at(place.clone()))?;
    let rows = choose_rows(source, request.layer, laser, selection, context, &place)?;
    cut_column(source, request, &rows, context, "read_selected")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::{ChannelDType, ChannelDescriptor, ClockClass, EnumLabels, NumericType};
    use crate::column::{ColumnSource, InMemorySource};
    use crate::error::ErrorClass;
    use crate::identity::{ChannelKey, LayerId, Presence};
    use crate::meta::{BuildIdentity, BuildMeta, LaserLayerMeta, LaserMeta};

    const LAYER: LayerId = LayerId(1);

    fn laser() -> LaserKey {
        LaserKey::new("1", 1)
    }

    fn key(name: &str) -> ChannelKey {
        ChannelKey::new([name])
    }

    fn u8s(values: &[u8]) -> ColumnData {
        ColumnData::U8(Shared::new(values.to_vec()))
    }

    fn rows(selection: &ReadSelection, len: usize, state: &[u8]) -> Vec<u64> {
        select_rows(selection, len, Some(&u8s(state)))
            .expect("a rule that holds")
            .to_vec()
    }

    #[test]
    fn the_default_selection_keeps_everything() {
        let everything = ReadSelection::default();
        assert_eq!(everything, ReadSelection::new());
        assert_eq!(
            (everything.stride(), everything.rows()),
            (1, StateRows::All)
        );
        let all = select_rows(&everything, 5, None).unwrap();
        assert_eq!(all.to_vec(), vec![0, 1, 2, 3, 4]);
        // and it is a run, not a list: 7.7 million rows cost no memory
        assert_eq!(all, NativeIndex::Strided { step: 1, len: 5 });
    }

    #[test]
    fn a_stride_keeps_indices_zero_s_two_s_and_a_stride_of_zero_is_one() {
        let three = ReadSelection::new().with_stride(3);
        assert_eq!(
            select_rows(&three, 12, None).unwrap().to_vec(),
            [0, 3, 6, 9]
        );
        // the last row is kept when it falls on the grid
        assert_eq!(
            select_rows(&three, 10, None).unwrap().to_vec(),
            [0, 3, 6, 9]
        );
        assert_eq!(select_rows(&three, 9, None).unwrap().to_vec(), [0, 3, 6]);
        let zero = ReadSelection::new().with_stride(0);
        assert_eq!(zero.stride(), 1);
        assert_eq!(zero, ReadSelection::default());
    }

    #[test]
    fn a_stride_beyond_the_length_keeps_one_row_and_an_empty_column_keeps_none() {
        let long = ReadSelection::new().with_stride(10);
        assert_eq!(select_rows(&long, 3, None).unwrap().to_vec(), [0]);
        assert_eq!(
            select_rows(&long, 0, None).unwrap().to_vec(),
            Vec::<u64>::new()
        );
        let huge = ReadSelection::new().with_stride(u64::MAX);
        assert_eq!(select_rows(&huge, 3, None).unwrap().to_vec(), [0]);
        // a filter over an empty column keeps none, and needs no row to exist
        let on = ReadSelection::new().with_rows(StateRows::OnOnly);
        assert!(select_rows(&on, 0, Some(&u8s(&[]))).unwrap().is_empty());
    }

    #[test]
    fn the_filter_looks_at_the_strided_rows_and_a_state_above_one_is_off() {
        let state = [1, 0, 0, 1, 1, 0, 1, 1, 0, 2, 255, 1];
        let on = ReadSelection::new().with_rows(StateRows::OnOnly);
        let off = ReadSelection::new().with_rows(StateRows::OffOnly);
        assert_eq!(rows(&on, 12, &state), [0, 3, 4, 6, 7, 11]);
        // 2 and 255 are off
        assert_eq!(rows(&off, 12, &state), [1, 2, 5, 8, 9, 10]);
        // with a stride of 2 the strided rows are 0, 2, 4, 6, 8, 10: their states are 1, 0, 1, 1, 0, 255
        assert_eq!(rows(&on.with_stride(2), 12, &state), [0, 4, 6]);
        assert_eq!(rows(&off.with_stride(2), 12, &state), [2, 8, 10]);
        // the laser-on row at native index 3 is never seen at a stride of 2
        assert!(!rows(&on.with_stride(2), 12, &state).contains(&3));
    }

    #[test]
    fn a_filter_that_keeps_no_row_is_an_empty_selection_not_an_error() {
        let on = ReadSelection::new().with_rows(StateRows::OnOnly);
        assert!(rows(&on, 5, &[0, 0, 2, 255, 0]).is_empty());
        // every strided row is off, though the native rows have laser-on ones
        assert!(rows(&on.with_stride(2), 6, &[0, 1, 0, 1, 0, 1]).is_empty());
        assert!(rows(
            &ReadSelection::new().with_rows(StateRows::OffOnly),
            3,
            &[1, 1, 1]
        )
        .is_empty());
    }

    #[test]
    fn a_filter_needs_a_state_and_the_other_selections_do_not() {
        for wanted in [StateRows::OnOnly, StateRows::OffOnly] {
            let error = select_rows(&ReadSelection::new().with_rows(wanted), 3, None).unwrap_err();
            assert!(
                matches!(error.kind(), ErrorKind::NotPresent { .. }),
                "{error}"
            );
            assert!(error.to_string().contains("laser state"), "{error}");
        }
        assert!(select_rows(&ReadSelection::new().with_stride(2), 3, None).is_ok());
    }

    #[test]
    fn a_state_of_another_length_than_the_column_keeps_only_the_rows_both_have() {
        let on = ReadSelection::new().with_rows(StateRows::OnOnly);
        // the state is shorter: the rows past its end have no state
        assert_eq!(rows(&on, 8, &[1, 1, 0, 1]), [0, 1, 3]);
        // the state is longer: its extra rows are ignored
        assert_eq!(rows(&on, 2, &[1, 1, 1, 1]), [0, 1]);
    }

    #[test]
    fn a_state_may_be_any_whole_number_or_a_flag_and_nothing_else() {
        let on = ReadSelection::new().with_rows(StateRows::OnOnly);
        let same = |state: ColumnData| select_rows(&on, 4, Some(&state)).unwrap().to_vec();
        assert_eq!(same(ColumnData::I8(Shared::new(vec![1, -1, 0, 1]))), [0, 3]);
        assert_eq!(same(ColumnData::U16(Shared::new(vec![1, 0, 1, 7]))), [0, 2]);
        assert_eq!(
            same(ColumnData::I32(Shared::new(vec![0, 1, 1, 1]))),
            [1, 2, 3]
        );
        assert_eq!(same(ColumnData::U64(Shared::new(vec![1, 1, 0, 0]))), [0, 1]);
        assert_eq!(
            same(ColumnData::Bool(Shared::new(vec![
                true, false, true, false
            ]))),
            [0, 2]
        );
        for not_a_state in [
            ColumnData::F32(Shared::new(vec![1.0; 4])),
            ColumnData::Text(Shared::new(vec!["1".to_string(); 4])),
        ] {
            let error = select_rows(&on, 4, Some(&not_a_state)).unwrap_err();
            assert!(matches!(error.kind(), ErrorKind::Corrupt { .. }), "{error}");
        }
    }

    /// A small deterministic generator, so that the properties below run over many shapes and the same ones every time (the Model has no dependencies, so no
    /// property-test crate).
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self, below: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) % below
        }
    }

    #[test]
    fn properties_of_the_rule_hold_over_many_shapes() {
        let mut random = Lcg(7);
        for _ in 0..2_000 {
            let len = random.next(60) as usize;
            let stride = random.next(12);
            let state: Vec<u8> = (0..len)
                .map(|_| [1, 1, 0, 0, 2, 255][random.next(6) as usize])
                .collect();
            let base = ReadSelection::new().with_stride(stride);
            let step = base.stride() as usize;

            // 1. with every row kept, selected row i is native row i x stride, and none is missed
            let all = select_rows(&base, len, None).unwrap().to_vec();
            let expected: Vec<u64> = (0..len).step_by(step).map(|n| n as u64).collect();
            assert_eq!(all, expected, "len {len}, stride {stride}");

            // 2. the filter keeps exactly the strided rows whose state matches, in order, and the two filters together are the strided rows
            let on = rows(&base.with_rows(StateRows::OnOnly), len, &state);
            let off = rows(&base.with_rows(StateRows::OffOnly), len, &state);
            let by_hand = |wanted: bool| -> Vec<u64> {
                expected
                    .iter()
                    .copied()
                    .filter(|n| (state[*n as usize] == 1) == wanted)
                    .collect()
            };
            assert_eq!(on, by_hand(true));
            assert_eq!(off, by_hand(false));
            let mut both: Vec<u64> = on.iter().chain(off.iter()).copied().collect();
            both.sort_unstable();
            assert_eq!(both, expected);

            // 3. every kept index is on the grid and inside the column, ascending
            for kept in [&on, &off, &all] {
                assert!(kept
                    .iter()
                    .all(|n| n % step as u64 == 0 && (*n as usize) < len));
                assert!(kept.windows(2).all(|pair| pair[0] < pair[1]));
            }
        }
    }

    // ------------------------------------------------------------ a read through a source

    /// A build with one laser in layer 1 that has a state channel and three other channels, and the columns of them in memory: `n` rows each, the value of row `i`
    /// being `i`, `10 i` or the state `[1, 0, 0, 1, 1, 0, 1, 1, 0, 2, 255, 1]` repeated.
    fn source_with_state(n: usize) -> InMemorySource {
        let mut build = BuildMeta::new(BuildIdentity {
            build_id: Some("b".into()),
            date: None,
            username: None,
            alloy: None,
            part_count: None,
            project_id: None,
            source_mcf_name: None,
            source_mcf_hash: None,
        });
        let mut meta = LaserMeta::new(laser());
        meta.layers
            .insert(LAYER, LaserLayerMeta::new(Presence::from_count(n as u64)));
        let mut channel = |name: &str, kind, dtype| {
            let mut descriptor =
                ChannelDescriptor::new(key(name), kind, ClockClass::LaserSynchronous, dtype);
            descriptor
                .presence
                .insert(LAYER, Presence::from_count(n as u64));
            meta.channels.push(descriptor);
        };
        channel(
            "x",
            ChannelKind::Sensor,
            ChannelDType::Numeric(NumericType::I32),
        );
        channel(
            "y",
            ChannelKind::Sensor,
            ChannelDType::Numeric(NumericType::I32),
        );
        channel(
            "state",
            ChannelKind::LaserState,
            ChannelDType::Enum {
                labels: EnumLabels::Unknown,
            },
        );
        channel(
            "power",
            ChannelKind::LaserPower,
            ChannelDType::Numeric(NumericType::F32),
        );
        build.add_laser(meta);
        build.synthesise_missing_layers();
        let mut source = InMemorySource::new();
        source.set_build(build);
        let state: Vec<u8> = (0..n)
            .map(|i| [1, 0, 0, 1, 1, 0, 1, 1, 0, 2, 255, 1][i % 12])
            .collect();
        let ints =
            |scale: i32| ColumnData::I32(Shared::new((0..n as i32).map(|i| i * scale).collect()));
        source.insert(LAYER, Some(laser()), key("x"), ints(1));
        source.insert(LAYER, Some(laser()), key("y"), ints(10));
        source.insert(LAYER, Some(laser()), key("state"), u8s(&state));
        source.insert(
            LAYER,
            Some(laser()),
            key("power"),
            ColumnData::F32(Shared::new((0..n).map(|i| i as f32 * 0.5).collect())),
        );
        source
    }

    fn request(name: &str) -> ColumnRequest {
        ColumnRequest::new(LAYER, Some(laser()), key(name))
    }

    fn select(
        source: &InMemorySource,
        name: &str,
        selection: &ReadSelection,
    ) -> Result<SelectedColumn, Error> {
        source.read_selected(&request(name), selection, &ReadContext::new())
    }

    #[test]
    fn the_default_selection_is_the_read_that_already_exists() {
        let source = source_with_state(12);
        let plain = source.read(&request("x"), &ReadContext::new()).unwrap();
        let selected = select(&source, "x", &ReadSelection::default()).unwrap();
        assert_eq!(selected.values, plain);
        assert_eq!(
            selected.native_index,
            NativeIndex::Strided { step: 1, len: 12 }
        );
        // the request's precision is still the request's
        let wide = source
            .read_selected(
                &request("x").with_precision(Precision::F64),
                &ReadSelection::default(),
                &ReadContext::new(),
            )
            .unwrap();
        assert_eq!(wide.values.as_f64().map(<[f64]>::len), Some(12));
    }

    #[test]
    fn every_channel_of_a_laser_is_cut_the_same_way_and_each_value_has_its_native_index() {
        let source = source_with_state(24);
        let selection = ReadSelection::new()
            .with_stride(3)
            .with_rows(StateRows::OnOnly);
        let x = select(&source, "x", &selection).unwrap();
        let y = select(&source, "y", &selection).unwrap();
        let power = select(&source, "power", &selection).unwrap();
        let state = select(&source, "state", &selection).unwrap();
        // strided rows 0, 3, 6, 9, 12, 15, 18, 21 have the states 1, 1, 1, 2, 1, 1, 1, 2
        let kept = vec![0_u64, 3, 6, 12, 15, 18];
        assert_eq!(x.native_index.to_vec(), kept);
        assert_eq!(y.native_index, x.native_index);
        assert_eq!(power.native_index, x.native_index);
        assert_eq!(state.native_index, x.native_index);
        // the value of row i is i (x), 10 i (y), i / 2 (power): selected row k is native row kept[k]
        assert_eq!(
            x.values.as_i32().unwrap(),
            kept.iter().map(|n| *n as i32).collect::<Vec<_>>()
        );
        assert_eq!(
            y.values.as_i32().unwrap(),
            kept.iter().map(|n| *n as i32 * 10).collect::<Vec<_>>()
        );
        assert_eq!(
            power.values.as_f32().unwrap(),
            kept.iter().map(|n| *n as f32 * 0.5).collect::<Vec<_>>()
        );
        // the state of every kept row is on, and the lengths agree
        assert!(matches!(&state.values, ColumnData::U8(v) if v.as_slice().iter().all(|s| *s == 1)));
        assert!([&x, &y, &power, &state]
            .iter()
            .all(|c| c.len() == kept.len()));
    }

    #[test]
    fn a_stride_alone_needs_no_metadata_and_keeps_the_native_index() {
        // an in-memory source with columns and no build metadata: only a filter needs the metadata, to find the state
        let mut source = InMemorySource::new();
        source.insert(
            LAYER,
            Some(laser()),
            key("x"),
            ColumnData::I32(Shared::new((0..10).collect())),
        );
        let selected = select(&source, "x", &ReadSelection::new().with_stride(4)).unwrap();
        assert_eq!(selected.values.as_i32().unwrap(), [0, 4, 8]);
        assert_eq!(selected.native_index.to_vec(), [0, 4, 8]);
        assert_eq!(
            selected.native_index,
            NativeIndex::Strided { step: 4, len: 3 }
        );
        let error = select(
            &source,
            "x",
            &ReadSelection::new().with_rows(StateRows::OnOnly),
        )
        .unwrap_err();
        assert!(
            matches!(error.kind(), ErrorKind::NotPresent { .. }),
            "{error}"
        );
    }

    #[test]
    fn the_precision_applies_to_the_kept_values() {
        let source = source_with_state(12);
        let selected = source
            .read_selected(
                &request("x").with_precision(Precision::F64),
                &ReadSelection::new().with_stride(5),
                &ReadContext::new(),
            )
            .unwrap();
        assert_eq!(selected.values.as_f64().unwrap(), [0.0, 5.0, 10.0]);
        assert_eq!(selected.native_index.to_vec(), [0, 5, 10]);
    }

    #[test]
    fn a_laser_without_a_state_channel_cannot_be_filtered() {
        let source = source_with_state(12);
        // the same column, in a layer whose laser has no state channel in the build's metadata
        let mut other = LaserMeta::new(LaserKey::new("2", 2));
        other
            .layers
            .insert(LAYER, LaserLayerMeta::new(Presence::from_count(12)));
        let mut build = (*source.build().unwrap()).clone();
        build.add_laser(other);
        let mut source = source;
        source.set_build(build);
        source.insert(
            LAYER,
            Some(LaserKey::new("2", 2)),
            key("x"),
            ColumnData::I32(Shared::new((0..12).collect())),
        );
        let request = ColumnRequest::new(LAYER, Some(LaserKey::new("2", 2)), key("x"));
        let filtered = source.read_selected(
            &request,
            &ReadSelection::new().with_rows(StateRows::OffOnly),
            &ReadContext::new(),
        );
        let error = filtered.unwrap_err();
        assert!(
            matches!(error.kind(), ErrorKind::NotPresent { .. }),
            "{error}"
        );
        assert_eq!(error.location().laser, Some(LaserKey::new("2", 2)));
        assert_eq!(error.location().operation.as_deref(), Some("read_selected"));
        // a stride of the same laser is fine
        assert!(source
            .read_selected(
                &request,
                &ReadSelection::new().with_stride(2),
                &ReadContext::new()
            )
            .is_ok());
    }

    #[test]
    fn a_request_a_selection_cannot_answer_is_refused_with_the_place() {
        let source = source_with_state(12);
        let selection = ReadSelection::new().with_stride(2);
        let refused = |request: ColumnRequest| {
            let error = source
                .read_selected(&request, &selection, &ReadContext::new())
                .unwrap_err();
            assert!(
                matches!(error.kind(), ErrorKind::InvalidIndex { .. }),
                "{error}"
            );
            assert_eq!(error.kind().class(), ErrorClass::Caller);
            assert_eq!(error.location().operation.as_deref(), Some("read_selected"));
            error
        };
        // a range of the column
        refused(request("x").with_range(ColumnRange::Indices(2..6)));
        // no laser: a build-level channel has no rows of a laser
        refused(ColumnRequest::new(LAYER, None, key("x")));
        // a column the source does not have is the source's own error, with its place
        let missing = source
            .read_selected(&request("nothing"), &selection, &ReadContext::new())
            .unwrap_err();
        assert!(
            matches!(missing.kind(), ErrorKind::NotPresent { .. }),
            "{missing}"
        );
    }

    #[test]
    fn a_cancelled_token_stops_a_selected_read() {
        let source = source_with_state(12);
        let token = crate::column::CancelToken::new();
        token.cancel();
        let error = source
            .read_selected(
                &request("x"),
                &ReadSelection::new().with_stride(2),
                &ReadContext::new().with_cancel(token),
            )
            .unwrap_err();
        assert!(matches!(error.kind(), ErrorKind::Cancelled), "{error}");
    }

    #[test]
    fn a_selection_is_a_value_that_can_be_copied_compared_and_hashed() {
        fn assert_traits<
            T: Copy + Clone + PartialEq + Eq + std::hash::Hash + Default + Send + Sync,
        >() {
        }
        assert_traits::<ReadSelection>();
        assert_traits::<StateRows>();
        // and the Model's read surface stays object safe with the new method on it
        let boxed: Box<dyn BuildSource> = Box::new(source_with_state(12));
        let selected = boxed
            .read_selected(
                &request("x"),
                &ReadSelection::new().with_stride(6),
                &ReadContext::new(),
            )
            .unwrap();
        assert_eq!(selected.native_index.to_vec(), [0, 6]);
    }

    // ------------------------------------------------------------ the rows of a laser, chosen once (decision D27-17)

    fn channels_of_the_laser() -> [&'static str; 4] {
        ["x", "y", "state", "power"]
    }

    #[test]
    fn rows_chosen_once_cut_every_channel_as_the_one_shot_read_does_and_share_one_list() {
        let source = source_with_state(24);
        for selection in [
            ReadSelection::new().with_rows(StateRows::OnOnly),
            ReadSelection::new().with_rows(StateRows::OffOnly),
            ReadSelection::new()
                .with_stride(3)
                .with_rows(StateRows::OnOnly),
            ReadSelection::new().with_stride(5),
        ] {
            let rows = source
                .laser_rows(LAYER, &laser(), &selection, &ReadContext::new())
                .unwrap();
            assert_eq!(rows.selection(), &selection);
            let mut cut = Vec::new();
            for name in channels_of_the_laser() {
                let once = source
                    .read_rows(&request(name), &rows, &ReadContext::new())
                    .unwrap();
                let one_shot = select(&source, name, &selection).unwrap();
                assert_eq!(once, one_shot, "{name} with {selection:?}");
                cut.push(once);
            }
            // a filter's channels point into the one list that the rows hold: the rows and each of the four results
            if let NativeIndex::Listed(list) = &cut[0].native_index {
                assert!(list.holders() > cut.len(), "{} holders", list.holders());
            }
        }
    }

    #[test]
    fn a_channel_shorter_than_the_state_gets_the_rows_that_exist_in_it() {
        let mut source = source_with_state(12);
        // the state is [1, 0, 0, 1, 1, 0, 1, 1, 0, 2, 255, 1]: on at 0, 3, 4, 6, 7, 11
        source.insert(
            LAYER,
            Some(laser()),
            key("short"),
            ColumnData::I32(Shared::new((0..6).map(|i| i * 100).collect())),
        );
        let rows = source
            .laser_rows(
                LAYER,
                &laser(),
                &ReadSelection::new().with_rows(StateRows::OnOnly),
                &ReadContext::new(),
            )
            .unwrap();
        let short = source
            .read_rows(&request("short"), &rows, &ReadContext::new())
            .unwrap();
        // only the rows below 6 exist in it: 0, 3 and 4, and each value is the one at its native index
        assert_eq!(short.native_index.to_vec(), [0, 3, 4]);
        assert_eq!(short.values.as_i32().unwrap(), [0, 300, 400]);
        // the list is not cut for a column that holds every row
        assert_eq!(rows.native_index(12).to_vec(), [0, 3, 4, 6, 7, 11]);
        assert_eq!(rows.native_index(0).to_vec(), Vec::<u64>::new());
        // a stride alone is cut to each column's length too
        let strided = LaserRows::new(
            LAYER,
            laser(),
            ReadSelection::new().with_stride(4),
            None,
            None,
        );
        assert_eq!(
            strided.native_index(6),
            NativeIndex::Strided { step: 4, len: 2 }
        );
        assert_eq!(
            strided.native_index(0),
            NativeIndex::Strided { step: 4, len: 0 }
        );
    }

    #[test]
    fn rows_chosen_for_one_laser_and_layer_are_refused_for_another() {
        let mut source = source_with_state(12);
        let other = LaserKey::new("2", 2);
        source.insert(
            LAYER,
            Some(other.clone()),
            key("x"),
            ColumnData::I32(Shared::new((0..12).collect())),
        );
        source.insert(
            LayerId(2),
            Some(laser()),
            key("x"),
            ColumnData::I32(Shared::new((0..12).collect())),
        );
        let rows = source
            .laser_rows(
                LAYER,
                &laser(),
                &ReadSelection::new().with_stride(2),
                &ReadContext::new(),
            )
            .unwrap();
        for wrong in [
            ColumnRequest::new(LAYER, Some(other), key("x")),
            ColumnRequest::new(LayerId(2), Some(laser()), key("x")),
            ColumnRequest::new(LAYER, None, key("x")),
            request("x").with_range(ColumnRange::Indices(0..4)),
        ] {
            let error = source
                .read_rows(&wrong, &rows, &ReadContext::new())
                .unwrap_err();
            assert!(
                matches!(error.kind(), ErrorKind::InvalidIndex { .. }),
                "{error}"
            );
            assert_eq!(error.location().operation.as_deref(), Some("read_rows"));
        }
        assert!(source
            .read_rows(&request("x"), &rows, &ReadContext::new())
            .is_ok());
    }

    #[test]
    fn choosing_the_rows_of_a_stride_reads_nothing_and_a_filter_needs_the_state() {
        // a source with no columns and no metadata: a stride needs neither
        let empty = InMemorySource::new();
        let rows = empty
            .laser_rows(
                LAYER,
                &laser(),
                &ReadSelection::new().with_stride(4),
                &ReadContext::new(),
            )
            .unwrap();
        assert_eq!(
            rows.native_index(10),
            NativeIndex::Strided { step: 4, len: 3 }
        );
        assert_eq!((rows.layer(), rows.laser()), (LAYER, &laser()));
        // a filter has to find the state: a source with no metadata cannot say where it is
        let error = empty
            .laser_rows(
                LAYER,
                &laser(),
                &ReadSelection::new().with_rows(StateRows::OnOnly),
                &ReadContext::new(),
            )
            .unwrap_err();
        assert!(
            matches!(error.kind(), ErrorKind::NotPresent { .. }),
            "{error}"
        );
        // a laser the layer does not record is NotPresent, with the operation `laser_rows`
        let source = source_with_state(12);
        let nobody = LaserKey::new("9", 9);
        let error = source
            .laser_rows(
                LAYER,
                &nobody,
                &ReadSelection::new().with_rows(StateRows::OffOnly),
                &ReadContext::new(),
            )
            .unwrap_err();
        assert!(
            matches!(error.kind(), ErrorKind::NotPresent { .. }),
            "{error}"
        );
        assert_eq!(error.location().operation.as_deref(), Some("laser_rows"));
        assert_eq!(error.location().laser, Some(nobody));
    }

    #[test]
    fn the_state_channel_is_cut_from_the_state_the_rows_hold_and_the_default_is_the_plain_read() {
        let source = source_with_state(24);
        let selection = ReadSelection::new().with_rows(StateRows::OnOnly);
        let rows = source
            .laser_rows(LAYER, &laser(), &selection, &ReadContext::new())
            .unwrap();
        let state = source
            .read_rows(&request("state"), &rows, &ReadContext::new())
            .unwrap();
        assert!(matches!(&state.values, ColumnData::U8(v) if v.as_slice().iter().all(|s| *s == 1)));
        assert_eq!(state, select(&source, "state", &selection).unwrap());
        // at the precision that was asked for
        let wide = source
            .read_rows(
                &request("state").with_precision(Precision::F64),
                &rows,
                &ReadContext::new(),
            )
            .unwrap();
        assert_eq!(wide.values.as_f64().unwrap().len(), state.len());
        // the default selection is the read that exists
        let all = source
            .laser_rows(
                LAYER,
                &laser(),
                &ReadSelection::default(),
                &ReadContext::new(),
            )
            .unwrap();
        let plain = source.read(&request("x"), &ReadContext::new()).unwrap();
        assert_eq!(
            source
                .read_rows(&request("x"), &all, &ReadContext::new())
                .unwrap()
                .values,
            plain
        );
    }

    #[test]
    fn a_cancelled_token_stops_the_choosing_of_rows() {
        let source = source_with_state(12);
        let token = crate::column::CancelToken::new();
        token.cancel();
        let error = source
            .laser_rows(
                LAYER,
                &laser(),
                &ReadSelection::new().with_rows(StateRows::OnOnly),
                &ReadContext::new().with_cancel(token),
            )
            .unwrap_err();
        assert!(matches!(error.kind(), ErrorKind::Cancelled), "{error}");
    }
}
