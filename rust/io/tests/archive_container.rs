//! The container of a build log, with real HDF5 files (V18 on): a build written into a working file, sealed into an `.lbl` archive and read back through the registry; the rule that a version lives in one kind of file; the extension rules; what an unsealed file
//! is; changing the HDF5 member of a sealed archive; and what the open archive guards.
//!
//! The mock versions stand in for V18 (decision D00-4: 9000 to 9999 are for tests): `9002` lives in an archive, `9001` in a bare file. The V18 adapter registers itself with `.container(ContainerKind::Archive)` the same way.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use build_log_io::adapters::mock;
use build_log_io::container::{
    create_working_file, create_working_file_with_minimum, extension_kind, pack, seal, unseal,
    Detached, FileArchive, FileState, GuardedSource, MemberSource, SealOptions, USERBLOCK,
};
use build_log_io::dispatch::{open, open_at, production, Registry};
use build_log_io::Libhdf5Version;
use build_log_model::{
    AvailabilityReport, BuildMeta, BuildOverview, BuildSource, CacheStats, ChannelDescriptor,
    ChannelKey, ColumnData, ColumnRequest, ColumnSource, ContainerKind, Error, ErrorKind,
    EventColumns, EventKind, EventPlace, EventRequest, GridData, GridRequest, ImageData,
    ImageRequest, LaserKey, LayerDetail, LayerId, ReadContext, Revision, ValidationReport,
};
use hdf5::File;

// ---------------------------------------------------------------- helpers

struct Dir(PathBuf);

impl Dir {
    fn new(label: &str) -> Dir {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "build-log-io-container-{}-{}-{label}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("a temporary folder");
        Dir(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const BARE: &str = "9001";
const ARCHIVED: &str = "9002";

/// The registry: the mock `9001` in a bare file, and the mock `9002` in an archive.
fn registry() -> Registry {
    Registry::new()
        .with(mock::adapter_set_in(BARE, ContainerKind::BareHdf5).unwrap())
        .unwrap()
        .with(mock::adapter_set_in(ARCHIVED, ContainerKind::Archive).unwrap())
        .unwrap()
}

/// What a build holds besides its mount: a dataset that the writer grows as the build runs.
const BLOCK: usize = 10_000;
const BLOCKS: usize = 5;

fn expected_samples(blocks: usize) -> Vec<i32> {
    (0..(blocks * BLOCK) as i32).collect()
}

fn write_mount(file: &File, key: &str, build_id: &str) {
    let mount = file.create_group(&mock::mount_for(key)).expect("the mount");
    mock::write_mock_file_for(&mount, key, build_id, 3).expect("a mock file");
}

fn grow_samples(file: &File, blocks: usize) {
    let samples = file
        .new_dataset::<i32>()
        .chunk(BLOCK)
        .shape(0..)
        .deflate(3)
        .create("Samples")
        .expect("a growing dataset");
    for block in 0..blocks {
        samples.resize((block + 1) * BLOCK).expect("it grows");
        let values: Vec<i32> = ((block * BLOCK) as i32..((block + 1) * BLOCK) as i32).collect();
        samples
            .write_slice(&values, (block * BLOCK)..((block + 1) * BLOCK))
            .expect("a block is written");
    }
}

/// A working file with the mock `key` mount and the growing dataset, closed.
fn working_build(path: &Path, key: &str, build_id: &str) {
    let file = create_working_file(path).expect("a working file");
    write_mount(&file, key, build_id);
    grow_samples(&file, BLOCKS);
}

fn options() -> SealOptions {
    SealOptions::new()
        .with_machine_config(MemberSource::Bytes(
            b"the machine configuration, opaque to this library".repeat(300),
        ))
        .with_member(
            "logs/capture.log",
            MemberSource::Bytes(b"capture started\n".repeat(100)),
        )
        .with_schema_version(18)
}

fn sealed_build(dir: &Dir, name: &str, key: &str, build_id: &str) -> PathBuf {
    let path = dir.path(name);
    working_build(&path, key, build_id);
    seal(&path, &options()).expect("the working file seals");
    path
}

/// A bare HDF5 file with the mock `key` mount.
fn bare_build(dir: &Dir, name: &str, key: &str, build_id: &str) -> PathBuf {
    let path = dir.path(name);
    let file = File::create(&path).expect("a bare file");
    write_mount(&file, key, build_id);
    path
}

fn kind_of<T>(result: Result<T, Error>) -> ErrorKind {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.kind().clone(),
    }
}

fn build_id(opened: &build_log_io::dispatch::Opened) -> String {
    opened
        .source()
        .build()
        .expect("the metadata")
        .identity
        .build_id
        .clone()
        .expect("a build id")
}

// ---------------------------------------------------------------- writing into a working file and reading it back

#[test]
fn a_build_written_into_a_working_file_is_read_by_stock_hdf5_from_inside_the_sealed_archive() {
    let dir = Dir::new("stock");
    let path = sealed_build(&dir, "build.lbl", ARCHIVED, "archived-1");
    // the plain binding opens the archive's own file, with no help from this library, and finds the userblock and the data HDF5 wrote
    let file = File::open(&path).expect("stock HDF5 opens the archive");
    assert_eq!(file.userblock(), USERBLOCK);
    let samples: Vec<i32> = file.dataset("Samples").unwrap().read_raw().unwrap();
    assert_eq!(samples, expected_samples(BLOCKS));
    assert!(file.group(&mock::mount_for(ARCHIVED)).is_ok());
}

#[test]
fn the_hdf5_member_of_a_sealed_archive_is_the_file_hdf5_wrote_and_every_member_checks() {
    let dir = Dir::new("members");
    let path = sealed_build(&dir, "build.lbl", ARCHIVED, "archived-2");
    let archive = FileArchive::open_path(&path).unwrap();
    assert_eq!(archive.primary().data_offset, USERBLOCK);
    assert_eq!(archive.primary_userblock(), Some(USERBLOCK));
    let hdf5_bytes = archive.read_member(archive.primary(), 1 << 30).unwrap();
    assert_eq!(
        hdf5_bytes[..8],
        [0x89, b'H', b'D', b'F', b'\r', b'\n', 0x1a, b'\n']
    );
    let file_bytes = fs::read(&path).unwrap();
    assert_eq!(
        hdf5_bytes,
        file_bytes[USERBLOCK as usize..USERBLOCK as usize + hdf5_bytes.len()]
    );
    for member in archive.members() {
        archive
            .verify(member)
            .unwrap_or_else(|e| panic!("{}: {e}", member.name));
    }
}

#[test]
fn a_working_file_is_created_exclusively_and_never_overwrites() {
    let dir = Dir::new("exclusive");
    let path = dir.path("build.lbl");
    working_build(&path, ARCHIVED, "first");
    let before = fs::read(&path).unwrap();
    assert!(create_working_file(&path).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn a_working_file_needs_a_libhdf5_that_meets_the_minimum_like_any_other_file() {
    let dir = Dir::new("minimum");
    let path = dir.path("build.lbl");
    let error = create_working_file_with_minimum(&path, Libhdf5Version::new(99, 0, 0))
        .expect_err("no libhdf5 is that new");
    assert!(
        matches!(error.kind(), ErrorKind::UnsupportedInVersion { .. }),
        "{error}"
    );
    assert!(!path.exists());
}

// ---------------------------------------------------------------- opening through the registry

#[test]
fn a_sealed_archive_opens_through_the_registry_with_its_container_members_and_machine_configuration(
) {
    let dir = Dir::new("registry");
    let path = sealed_build(&dir, "build.lbl", ARCHIVED, "archived-3");
    let opened = open(&registry(), &path).expect("the archive opens");
    assert_eq!(build_id(&opened), "archived-3");
    assert_eq!(opened.key().as_str(), ARCHIVED);
    assert_eq!(opened.container(), ContainerKind::Archive);
    let archive = opened.archive().expect("an archive");
    let mcf = archive
        .machine_config()
        .expect("the machine configuration is a member");
    assert_eq!(
        archive.read_member(mcf, 1 << 20).unwrap(),
        b"the machine configuration, opaque to this library".repeat(300)
    );
    assert!(archive.find("logs/capture.log").is_some());
    assert_eq!(opened.check_file(), Some(FileState::Unchanged));
}

#[test]
fn a_bare_file_opens_as_a_bare_file_and_is_not_watched() {
    let dir = Dir::new("bare");
    let path = bare_build(&dir, "old.h5", BARE, "bare-1");
    let opened = open(&registry(), &path).expect("the bare file opens");
    assert_eq!(build_id(&opened), "bare-1");
    assert_eq!(opened.container(), ContainerKind::BareHdf5);
    assert!(opened.archive().is_none());
    assert_eq!(opened.check_file(), None);
}

#[test]
fn production_versions_live_in_the_container_their_number_says() {
    // V17 and earlier are bare HDF5 files; V18 and later are archives. The rule is each adapter's data (versions are matched exactly, with no "18 or higher" in the dispatcher), and this holds the list to it.
    let mut checked = 0;
    for set in production().sets() {
        let number: u32 = set
            .key()
            .as_str()
            .parse()
            .expect("a production version is a number");
        let expected = if number >= 18 {
            ContainerKind::Archive
        } else {
            ContainerKind::BareHdf5
        };
        assert_eq!(
            set.container(),
            expected,
            "version {} lives in {expected}",
            set.key()
        );
        checked += 1;
    }
    assert!(checked >= 2, "V16 and V17 are registered");
}

// ---------------------------------------------------------------- the rules

#[test]
fn a_version_that_lives_in_an_archive_is_refused_as_a_bare_file_and_the_reverse() {
    let dir = Dir::new("rule");
    // an archived version's marker in a bare file
    let bare = bare_build(&dir, "wrong.h5", ARCHIVED, "x");
    match kind_of(open(&registry(), &bare)) {
        ErrorKind::WrongContainer {
            found, expected, ..
        } => assert_eq!(
            (found, expected),
            (ContainerKind::BareHdf5, ContainerKind::Archive)
        ),
        other => panic!("{other:?}"),
    }
    // a bare version's marker in an archive
    let archive = sealed_build(&dir, "wrong.lbl", BARE, "y");
    match kind_of(open(&registry(), &archive)) {
        ErrorKind::WrongContainer {
            found,
            expected,
            detail,
        } => {
            assert_eq!(
                (found, expected),
                (ContainerKind::Archive, ContainerKind::BareHdf5)
            );
            assert!(
                detail.contains("9001"),
                "the detail names the version: {detail}"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_unknown_version_is_unsupported_before_its_container_is_judged() {
    let dir = Dir::new("order");
    // a version that nobody registered, in the place where the registered version keeps its marker, in an archive: the registry knows no adapter for it
    let path = dir.path("build.lbl");
    {
        let file = create_working_file(&path).unwrap();
        let mount = file.create_group(&mock::mount_for(BARE)).unwrap();
        mock::write_mock_file_for(&mount, "9003", "z", 1).unwrap();
    }
    seal(&path, &options()).unwrap();
    let only_bare = Registry::new()
        .with(mock::adapter_set_in(BARE, ContainerKind::BareHdf5).unwrap())
        .unwrap();
    // so what is said is that: not that an archive is the wrong kind for a version nobody knows
    assert!(matches!(
        kind_of(open(&only_bare, &path)),
        ErrorKind::UnsupportedVersion { .. }
    ));
}

#[test]
fn an_extension_that_contradicts_the_content_is_a_wrong_container_whatever_the_version() {
    let dir = Dir::new("extension");
    // a bare HDF5 file named like an archive
    let bare_content = bare_build(&dir, "old.h5", BARE, "a");
    let named_lbl = dir.path("looks-like-an-archive.lbl");
    fs::copy(&bare_content, &named_lbl).unwrap();
    match kind_of(open(&registry(), &named_lbl)) {
        ErrorKind::WrongContainer {
            found, expected, ..
        } => assert_eq!(
            (found, expected),
            (ContainerKind::BareHdf5, ContainerKind::Archive)
        ),
        other => panic!("{other:?}"),
    }
    // an archive named like a bare HDF5 file, in either spelling
    let archive = sealed_build(&dir, "real.lbl", ARCHIVED, "b");
    for name in ["looks-bare.h5", "LOOKS-BARE.HDF5"] {
        let renamed = dir.path(name);
        fs::copy(&archive, &renamed).unwrap();
        match kind_of(open(&registry(), &renamed)) {
            ErrorKind::WrongContainer {
                found, expected, ..
            } => assert_eq!(
                (found, expected),
                (ContainerKind::Archive, ContainerKind::BareHdf5),
                "{name}"
            ),
            other => panic!("{name}: {other:?}"),
        }
    }
}

#[test]
fn any_other_extension_lets_the_content_decide() {
    let dir = Dir::new("other-extension");
    let archive = sealed_build(&dir, "real.lbl", ARCHIVED, "c");
    let bare = bare_build(&dir, "old.h5", BARE, "d");
    for (source, name, id) in [
        (&archive, "download.part", "c"),
        (&archive, "no_extension", "c"),
        (&bare, "old.tmp", "d"),
    ] {
        let copy = dir.path(name);
        fs::copy(source, &copy).unwrap();
        assert_eq!(extension_kind(&copy), None);
        let opened = open(&registry(), &copy).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(build_id(&opened), id, "{name}");
    }
}

// ---------------------------------------------------------------- the working file

#[test]
fn a_working_file_is_unsealed_until_it_is_sealed_and_opens_once_it_is() {
    let dir = Dir::new("unsealed");
    let path = dir.path("build.lbl");
    working_build(&path, ARCHIVED, "in-progress");
    let error = open(&registry(), &path).expect_err("a working file is not a build log yet");
    assert!(
        matches!(error.kind(), ErrorKind::Unsealed { .. }),
        "{error}"
    );
    assert!(error.is_retryable(), "it opens once the build ends");
    seal(&path, &options()).unwrap();
    assert_eq!(build_id(&open(&registry(), &path).unwrap()), "in-progress");
}

#[test]
fn a_seal_that_was_cut_off_leaves_a_working_file_not_a_damaged_archive() {
    let dir = Dir::new("cut-off");
    let path = dir.path("build.lbl");
    working_build(&path, ARCHIVED, "cut");
    let report = seal(&path, &options()).unwrap();
    // everything but the directory and the end records was written
    let end_of_members = report.members.last().unwrap().data_end();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(end_of_members)
        .unwrap();
    let error = open(&registry(), &path).expect_err("no directory, no archive");
    assert!(
        matches!(error.kind(), ErrorKind::Unsealed { .. }),
        "{error}"
    );
    // the HDF5 data is still all there, and sealing again completes the archive
    seal(&path, &options()).unwrap();
    assert_eq!(build_id(&open(&registry(), &path).unwrap()), "cut");
}

// ---------------------------------------------------------------- changing the HDF5 member of a sealed archive

#[test]
fn opening_a_sealed_archive_for_writing_destroys_it_which_is_why_unseal_exists() {
    let dir = Dir::new("hazard");
    let path = sealed_build(&dir, "build.lbl", ARCHIVED, "hazard");
    {
        // an attribute and nothing else: the pass every post-build step is
        let file = File::open_rw(&path).expect("HDF5 opens it for writing");
        file.new_attr::<i32>()
            .create("Opcua_Join_Version")
            .unwrap()
            .write_scalar(&2)
            .unwrap();
    }
    assert!(
        FileArchive::open_path(&path).is_err(),
        "HDF5 wrote where the other members were, and cut the file back to its own end"
    );
}

#[test]
fn a_pass_that_writes_to_a_sealed_archive_keeps_every_other_member_when_it_unseals_first() {
    let dir = Dir::new("pass");
    let path = sealed_build(&dir, "build.lbl", ARCHIVED, "pass");
    let (mcf, log) = {
        let a = FileArchive::open_path(&path).unwrap();
        (
            a.read_member(a.machine_config().unwrap(), 1 << 20).unwrap(),
            a.read_member(a.find("logs/capture.log").unwrap(), 1 << 20)
                .unwrap(),
        )
    };

    let detached = unseal(&path, 1 << 20).expect("unsealing");
    {
        let file = File::open_rw(&path).expect("it is a working file again");
        file.new_attr::<i32>()
            .create("Opcua_Join_Version")
            .unwrap()
            .write_scalar(&2)
            .unwrap();
        // and the pass grows the data as well
        let samples = file.dataset("Samples").unwrap();
        samples.resize((BLOCKS + 1) * BLOCK).unwrap();
        let more: Vec<i32> = ((BLOCKS * BLOCK) as i32..((BLOCKS + 1) * BLOCK) as i32).collect();
        samples
            .write_slice(&more, (BLOCKS * BLOCK)..((BLOCKS + 1) * BLOCK))
            .unwrap();
    }
    seal(&path, &detached.to_options()).expect("sealing again");
    detached.finish().unwrap();

    let opened = open(&registry(), &path).expect("it is an archive again");
    assert_eq!(build_id(&opened), "pass");
    let archive = opened.archive().unwrap();
    assert_eq!(
        archive
            .read_member(archive.machine_config().unwrap(), 1 << 20)
            .unwrap(),
        mcf
    );
    assert_eq!(
        archive
            .read_member(archive.find("logs/capture.log").unwrap(), 1 << 20)
            .unwrap(),
        log
    );
    for member in archive.members() {
        archive.verify(member).unwrap();
    }
    drop(opened);
    let file = File::open(&path).unwrap();
    assert_eq!(
        file.attr("Opcua_Join_Version")
            .unwrap()
            .read_scalar::<i32>()
            .unwrap(),
        2
    );
    assert_eq!(
        file.dataset("Samples").unwrap().read_raw::<i32>().unwrap(),
        expected_samples(BLOCKS + 1)
    );
}

#[test]
fn an_interrupted_pass_on_a_sealed_archive_is_recovered_from_the_side_file() {
    let dir = Dir::new("recover");
    let path = sealed_build(&dir, "build.lbl", ARCHIVED, "recover");
    let detached = unseal(&path, 1 << 20).unwrap();
    drop(detached); // the process dies: the archive is a working file and the other members are in the side file
    assert!(matches!(
        kind_of(open(&registry(), &path)),
        ErrorKind::Unsealed { .. }
    ));
    let recovered = Detached::load(&path)
        .unwrap()
        .expect("the side file is there");
    seal(&path, &recovered.to_options()).unwrap();
    recovered.finish().unwrap();
    let opened = open(&registry(), &path).unwrap();
    assert_eq!(build_id(&opened), "recover");
    assert!(opened.archive().unwrap().machine_config().is_some());
}

// ---------------------------------------------------------------- a bare HDF5 file packed into an archive

#[test]
fn a_bare_hdf5_file_packed_into_an_archive_reads_the_same_through_the_registry() {
    let dir = Dir::new("pack");
    let bare = dir.path("old.h5");
    {
        let file = File::create(&bare).unwrap();
        write_mount(&file, ARCHIVED, "packed");
        grow_samples(&file, BLOCKS);
    }
    let lbl = dir.path("new.lbl");
    pack(&bare, &lbl, &options()).expect("packing");
    let opened = open(&registry(), &lbl).expect("the packed archive opens");
    assert_eq!(build_id(&opened), "packed");
    assert_eq!(opened.container(), ContainerKind::Archive);
    drop(opened);
    let samples: Vec<i32> = File::open(&lbl)
        .unwrap()
        .dataset("Samples")
        .unwrap()
        .read_raw()
        .unwrap();
    assert_eq!(
        samples,
        expected_samples(BLOCKS),
        "the same values, from behind the userblock"
    );
}

#[test]
fn the_v18_miniature_packed_into_an_archive_is_read_from_inside_it_and_its_marker_is_found() {
    // V18 is provisional (testdata/README.md): the facts used are the marker's place and the first laser's first channel, and the file is only read. The miniature is copied, never changed.
    let miniature = build_log_testkit::miniature_path(build_log_testkit::miniature::V18);
    build_log_testkit::skip_unless!(miniature.is_file(), "{} is not there", miniature.display());
    let dir = Dir::new("v18");
    let lbl = dir.path("v18.lbl");
    pack(&miniature, &lbl, &options().with_schema_version(18)).expect("packing the miniature");
    let archive = FileArchive::open_path(&lbl).unwrap();
    assert_eq!(archive.manifest().schema_version, Some(18));
    drop(archive);
    let packed = File::open(&lbl).unwrap();
    let original = File::open(&miniature).unwrap();
    assert_eq!(
        packed
            .group("Build")
            .unwrap()
            .attr("File_Version")
            .unwrap()
            .read_scalar::<i32>()
            .unwrap(),
        18
    );
    let channel = "Build/Lasers/Laser_1/Monitoring_Data/Layer_1/Actual_X";
    let a: Vec<i32> = packed.dataset(channel).unwrap().read_raw().unwrap();
    let b: Vec<i32> = original.dataset(channel).unwrap().read_raw().unwrap();
    assert!(!a.is_empty());
    assert_eq!(
        a, b,
        "the data inside the archive is the data of the file it was made from"
    );
    // the marker of V18 is where its layout puts it, and it is the version 18, read from inside the archive
    let build = packed.group("Build").unwrap();
    let key = build_log_io::dispatch::peek_marker(&build, &["File_Version"])
        .unwrap()
        .expect("a marker");
    assert_eq!(key.as_str(), "18");
}

// ---------------------------------------------------------------- the mount primitive does not look at the container

#[test]
fn a_group_the_caller_holds_is_opened_without_a_container_check() {
    let dir = Dir::new("open-at");
    // 9002 declares an archive, and here its data is in a bare file the caller holds: the caller's file is not looked at
    let bare = bare_build(&dir, "held.h5", ARCHIVED, "held");
    let file = File::open(&bare).unwrap();
    let mount = file.group(&mock::mount_for(ARCHIVED)).unwrap();
    let opened = open_at(&registry(), &mount).expect("a mount opens wherever it is");
    assert_eq!(build_id(&opened), "held");
    assert_eq!(
        opened.container(),
        ContainerKind::Archive,
        "it reports the kind the version declares"
    );
    assert!(opened.archive().is_none() && opened.check_file().is_none());
}

// ---------------------------------------------------------------- what the open archive guards

#[test]
fn a_new_version_put_in_place_of_an_open_archive_is_reported_and_the_open_source_keeps_reading_the_old_one(
) {
    let dir = Dir::new("superseded");
    let path = sealed_build(&dir, "build.lbl", ARCHIVED, "old");
    let opened = open(&registry(), &path).unwrap();
    assert_eq!(opened.check_file(), Some(FileState::Unchanged));

    let next = sealed_build(&dir, "next.lbl", ARCHIVED, "new");
    fs::rename(&next, &path).expect("a new version replaces the file, even while it is open");
    assert_eq!(opened.check_file(), Some(FileState::Superseded));
    assert_eq!(
        build_id(&opened),
        "old",
        "the source reads the consistent file it opened"
    );
    drop(opened);
    assert_eq!(
        build_id(&open(&registry(), &path).unwrap()),
        "new",
        "and a new open reads the new version"
    );
}

#[test]
fn a_deleted_archive_is_reported_removed_and_the_open_source_keeps_reading() {
    let dir = Dir::new("removed");
    let path = sealed_build(&dir, "build.lbl", ARCHIVED, "gone");
    let opened = open(&registry(), &path).unwrap();
    fs::remove_file(&path).expect("a file can be deleted while it is open");
    assert_eq!(opened.check_file(), Some(FileState::Removed));
    assert_eq!(build_id(&opened), "gone");
}

#[cfg(windows)]
#[test]
fn on_windows_hdf5_cannot_open_an_archive_for_writing_while_a_source_of_it_exists_even_once_handed_over(
) {
    let dir = Dir::new("windows-exclusion");
    let path = sealed_build(&dir, "build.lbl", ARCHIVED, "held");
    let opened = open(&registry(), &path).unwrap();
    assert!(
        File::open_rw(&path).is_err(),
        "a writer is refused while a source is open"
    );
    assert!(unseal(&path, 1 << 20).is_err_and(|e| matches!(
        e.kind(),
        ErrorKind::IoError {
            transient: true,
            ..
        }
    )));
    // handing the source over keeps the archive open and guarded for as long as the source lives
    let source = opened.into_source();
    assert!(
        File::open_rw(&path).is_err(),
        "the source holds the archive"
    );
    assert!(source.build().is_ok());
    drop(source);
    File::open_rw(&path).expect("it can be written once the source is gone");
}

#[cfg(windows)]
#[test]
fn on_windows_an_archive_that_a_writer_has_open_is_a_transient_error_and_not_a_half_read_file() {
    let dir = Dir::new("windows-writer");
    let path = sealed_build(&dir, "build.lbl", ARCHIVED, "busy");
    let writer = fs::OpenOptions::new().write(true).open(&path).unwrap();
    let error = open(&registry(), &path).expect_err("a writer has the file");
    assert!(error.is_retryable(), "{error}");
    drop(writer);
    assert!(open(&registry(), &path).is_ok());
}

#[cfg(unix)]
#[test]
fn on_unix_a_change_in_place_under_an_open_source_makes_every_call_fail_with_file_changed() {
    use std::io::{Seek, SeekFrom, Write};
    let dir = Dir::new("unix-modified");
    let path = sealed_build(&dir, "build.lbl", ARCHIVED, "changed");
    let opened = open(&registry(), &path).unwrap();
    assert!(opened.source().build().is_ok());
    let mut writer = fs::OpenOptions::new().write(true).open(&path).unwrap();
    writer.seek(SeekFrom::End(0)).unwrap();
    writer.write_all(b"more").unwrap();
    writer.sync_all().unwrap();
    assert_eq!(opened.check_file(), Some(FileState::Modified));
    let error = opened.source().build().expect_err("the file changed");
    assert!(
        matches!(error.kind(), ErrorKind::FileChanged { .. }),
        "{error}"
    );
    assert!(opened.source().layers().is_err(), "and it stays so");
}

// ---------------------------------------------------------------- the guard delegates every method

/// A source that says which of its methods was called.
#[derive(Default)]
struct Recording(Mutex<Vec<&'static str>>);

impl Recording {
    fn note(&self, name: &'static str) {
        self.0.lock().unwrap().push(name);
    }
}

impl ColumnSource for Recording {
    fn read(&self, _: &ColumnRequest, _: &ReadContext) -> Result<ColumnData, Error> {
        self.note("read");
        Err(Error::new(ErrorKind::Cancelled))
    }
    fn read_events(&self, _: &EventRequest, _: &ReadContext) -> Result<EventColumns, Error> {
        self.note("read_events");
        Err(Error::new(ErrorKind::Cancelled))
    }
    fn read_image(&self, _: &ImageRequest, _: &ReadContext) -> Result<ImageData, Error> {
        self.note("read_image");
        Err(Error::new(ErrorKind::Cancelled))
    }
    fn read_grid(&self, _: &GridRequest, _: &ReadContext) -> Result<GridData, Error> {
        self.note("read_grid");
        Err(Error::new(ErrorKind::Cancelled))
    }
    fn cache_stats(&self) -> Option<CacheStats> {
        self.note("cache_stats");
        None
    }
}

impl BuildSource for Recording {
    fn build(&self) -> Result<Arc<BuildMeta>, Error> {
        self.note("build");
        Err(Error::new(ErrorKind::Cancelled))
    }
    fn revision(&self) -> Result<Revision, Error> {
        self.note("revision");
        Ok(Revision::default())
    }
    fn layers(&self) -> Result<Vec<LayerId>, Error> {
        self.note("layers");
        Ok(Vec::new())
    }
    fn lasers_in_layer(&self, _: LayerId) -> Result<Vec<LaserKey>, Error> {
        self.note("lasers_in_layer");
        Ok(Vec::new())
    }
    fn validate(&self) -> Result<Arc<ValidationReport>, Error> {
        self.note("validate");
        Err(Error::new(ErrorKind::Cancelled))
    }
    fn availability(&self) -> Result<Arc<AvailabilityReport>, Error> {
        self.note("availability");
        Err(Error::new(ErrorKind::Cancelled))
    }
    fn channels(&self, _: &LaserKey) -> Result<Vec<ChannelDescriptor>, Error> {
        self.note("channels");
        Ok(Vec::new())
    }
    fn overview(&self) -> Result<Arc<BuildOverview>, Error> {
        self.note("overview");
        Err(Error::new(ErrorKind::Cancelled))
    }
    fn layer_meta(&self, _: LayerId) -> Result<Arc<LayerDetail>, Error> {
        self.note("layer_meta");
        Err(Error::new(ErrorKind::Cancelled))
    }
}

/// Lets a test see what the wrapped source was asked, after the wrapper has taken it.
struct Shared(Arc<Recording>);

impl ColumnSource for Shared {
    fn read(&self, r: &ColumnRequest, c: &ReadContext) -> Result<ColumnData, Error> {
        self.0.read(r, c)
    }
    fn read_events(&self, r: &EventRequest, c: &ReadContext) -> Result<EventColumns, Error> {
        self.0.read_events(r, c)
    }
    fn read_image(&self, r: &ImageRequest, c: &ReadContext) -> Result<ImageData, Error> {
        self.0.read_image(r, c)
    }
    fn read_grid(&self, r: &GridRequest, c: &ReadContext) -> Result<GridData, Error> {
        self.0.read_grid(r, c)
    }
    fn cache_stats(&self) -> Option<CacheStats> {
        self.0.cache_stats()
    }
}

impl BuildSource for Shared {
    fn build(&self) -> Result<Arc<BuildMeta>, Error> {
        self.0.build()
    }
    fn revision(&self) -> Result<Revision, Error> {
        self.0.revision()
    }
    fn layers(&self) -> Result<Vec<LayerId>, Error> {
        self.0.layers()
    }
    fn lasers_in_layer(&self, l: LayerId) -> Result<Vec<LaserKey>, Error> {
        self.0.lasers_in_layer(l)
    }
    fn validate(&self) -> Result<Arc<ValidationReport>, Error> {
        self.0.validate()
    }
    fn availability(&self) -> Result<Arc<AvailabilityReport>, Error> {
        self.0.availability()
    }
    fn channels(&self, l: &LaserKey) -> Result<Vec<ChannelDescriptor>, Error> {
        self.0.channels(l)
    }
    fn overview(&self) -> Result<Arc<BuildOverview>, Error> {
        self.0.overview()
    }
    fn layer_meta(&self, l: LayerId) -> Result<Arc<LayerDetail>, Error> {
        self.0.layer_meta(l)
    }
}

#[test]
fn the_guard_delegates_every_method_of_both_traits_to_the_source_it_wraps() {
    let dir = Dir::new("delegation");
    let path = sealed_build(&dir, "build.lbl", ARCHIVED, "delegation");
    let archive = Arc::new(FileArchive::open_path(&path).unwrap());
    let recording = Arc::new(Recording::default());
    let guarded = GuardedSource::new(Box::new(Shared(Arc::clone(&recording))), archive);

    let context = ReadContext::new();
    let laser = LaserKey::new("a", 1);
    let layer = LayerId(1);
    let _ = guarded.read(
        &ColumnRequest::new(
            layer,
            Some(laser.clone()),
            ChannelKey::new(["position", "x"]),
        ),
        &context,
    );
    let _ = guarded.read_events(
        &EventRequest::new(
            EventKind::Sensor,
            vec!["s".to_string()],
            EventPlace::NoLayer,
        ),
        &context,
    );
    let _ = guarded.read_image(
        &ImageRequest::new(layer, ChannelKey::new(["images", "pre"])),
        &context,
    );
    let _ = guarded.read_grid(&GridRequest::new(laser.clone(), "grid"), &context);
    let _ = guarded.cache_stats();
    let _ = guarded.build();
    let _ = guarded.revision();
    let _ = guarded.layers();
    let _ = guarded.lasers_in_layer(layer);
    let _ = guarded.validate();
    let _ = guarded.availability();
    let _ = guarded.channels(&laser);
    let _ = guarded.overview();
    let _ = guarded.layer_meta(layer);

    // each call reached the method of the same name, and no call turned into another (a defaulted method that was not delegated would call `build` instead)
    let seen = recording.0.lock().unwrap().clone();
    assert_eq!(
        seen,
        [
            "read",
            "read_events",
            "read_image",
            "read_grid",
            "cache_stats",
            "build",
            "revision",
            "layers",
            "lasers_in_layer",
            "validate",
            "availability",
            "channels",
            "overview",
            "layer_meta"
        ]
    );
}
