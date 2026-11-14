use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use snapline_core::naming;
use snapline_store::history::{CaptureId, CaptureKind, HistoryError, HistoryRepo, NewCapture};
use snapline_store::io::write_atomic;

pub const MANIFEST_VERSION: u32 = 1;
const MANIFEST_EXTENSION: &str = "json";
const REJECTED_DIR: &str = "rejected";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    AfterManifest,
    AfterStage,
    AfterPublish,
    BeforeRegister,
    AfterRegister,
}

impl FaultPoint {
    pub const ALL: [Self; 5] = [
        Self::AfterManifest,
        Self::AfterStage,
        Self::AfterPublish,
        Self::BeforeRegister,
        Self::AfterRegister,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    Fail,
    Crash,
}

pub trait Faults: Send + Sync {
    fn at(&self, point: FaultPoint) -> Option<Fault>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoFaults;

impl Faults for NoFaults {
    fn at(&self, _point: FaultPoint) -> Option<Fault> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub operation_id: String,
    pub final_path: String,
    pub staged_path: String,
    pub bytes: i64,
    pub width: i64,
    pub height: i64,
    pub source_app: Option<String>,
    pub created_at: i64,
}

impl Manifest {
    fn capture(&self) -> NewCapture {
        NewCapture {
            kind: CaptureKind::Image,
            path: self.final_path.clone(),
            bytes: self.bytes,
            width: Some(self.width),
            height: Some(self.height),
            duration_ms: None,
            source_app: self.source_app.clone(),
            created_at: self.created_at,
            ocr_text: None,
        }
    }
}

#[derive(Debug)]
pub enum DurableError {
    Write(io::Error),
    Crashed(FaultPoint),
}

impl std::fmt::Display for DurableError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Write(error) => write!(formatter, "{error}"),
            Self::Crashed(point) => write!(formatter, "the save stopped at {point:?}"),
        }
    }
}

impl std::error::Error for DurableError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    Registered(CaptureId),
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committed {
    pub final_path: PathBuf,
    pub registration: Registration,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reconciled {
    pub registered: Vec<(CaptureId, PathBuf)>,
    pub still_pending: usize,
    pub abandoned: usize,
    pub rejected: usize,
}

#[derive(Clone)]
pub struct Journal {
    directory: PathBuf,
    faults: Arc<dyn Faults>,
}

impl Journal {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
            faults: Arc::new(NoFaults),
        }
    }

    pub fn with_faults(mut self, faults: Arc<dyn Faults>) -> Self {
        self.faults = faults;
        self
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn operation_id(created_at: i64, counter: u32, random: &str) -> String {
        let random: String = random
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(12)
            .collect();
        format!("{created_at:x}-{counter:x}-{random}")
    }

    pub fn staged_path(final_path: &Path, operation_id: &str) -> PathBuf {
        let name = final_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        final_path.with_file_name(format!(".{name}.{operation_id}.part"))
    }

    pub fn commit(
        &self,
        repo: &mut HistoryRepo,
        manifest: &Manifest,
        encoded: &[u8],
    ) -> Result<Committed, DurableError> {
        let final_path = PathBuf::from(&manifest.final_path);
        let staged_path = PathBuf::from(&manifest.staged_path);
        let manifest_path = self.manifest_path(&manifest.operation_id);

        self.write_manifest(manifest).map_err(DurableError::Write)?;
        self.fault(FaultPoint::AfterManifest, &manifest_path, None)?;

        if let Err(error) = stage(&staged_path, encoded) {
            abandon(&manifest_path, Some(&staged_path));
            return Err(DurableError::Write(error));
        }
        self.fault(FaultPoint::AfterStage, &manifest_path, Some(&staged_path))?;

        if let Err(error) = publish(&staged_path, &final_path) {
            abandon(&manifest_path, Some(&staged_path));
            return Err(DurableError::Write(error));
        }
        self.fault(FaultPoint::AfterPublish, &manifest_path, None)?;

        let registration = match self.register(repo, manifest) {
            Ok(registration) => registration,
            Err(RegisterOutcome::Crashed(point)) => return Err(DurableError::Crashed(point)),
        };
        Ok(Committed {
            final_path,
            registration,
        })
    }

    pub fn pending_count(&self) -> usize {
        self.manifest_files().len()
    }

    pub fn reconcile(&self, repo: &mut HistoryRepo) -> Reconciled {
        let mut report = Reconciled::default();
        self.remove_temporary_files();
        for path in self.manifest_files() {
            let manifest = match read_manifest(&path) {
                Ok(manifest) => manifest,
                Err(reason) => {
                    tracing::warn!(file = %path.display(), reason, "a save manifest was rejected");
                    self.quarantine(&path);
                    report.rejected += 1;
                    continue;
                }
            };
            self.reconcile_one(repo, &path, &manifest, &mut report);
        }
        report
    }

    fn reconcile_one(
        &self,
        repo: &mut HistoryRepo,
        manifest_path: &Path,
        manifest: &Manifest,
        report: &mut Reconciled,
    ) {
        let staged = PathBuf::from(&manifest.staged_path);
        let mut final_path = PathBuf::from(&manifest.final_path);
        let mut manifest = manifest.clone();

        match repo.capture_for_operation(&manifest.operation_id) {
            Ok(Some(_)) => {
                let _ = fs::remove_file(&staged);
                let _ = fs::remove_file(manifest_path);
                return;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "History is not available to reconcile a save");
                report.still_pending += 1;
                return;
            }
        }

        match fs::metadata(&staged) {
            Ok(meta) => {
                if i64::try_from(meta.len()).ok() != Some(manifest.bytes) {
                    tracing::warn!(file = %staged.display(), "a staged capture was incomplete and was discarded");
                    abandon(manifest_path, Some(&staged));
                    report.abandoned += 1;
                    return;
                }
                let final_matches = fs::metadata(&final_path)
                    .map(|existing| i64::try_from(existing.len()).ok() == Some(manifest.bytes))
                    .unwrap_or(false);
                if final_matches {
                    let _ = fs::remove_file(&staged);
                } else {
                    if final_path.exists() {
                        let folder = final_path
                            .parent()
                            .map(Path::to_path_buf)
                            .unwrap_or_default();
                        let name = final_path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let free = naming::resolve_collision(&name, |candidate| {
                            folder.join(candidate).exists()
                        });
                        final_path = folder.join(free);
                        manifest.final_path = final_path.to_string_lossy().into_owned();
                        if self.write_manifest(&manifest).is_err() {
                            report.still_pending += 1;
                            return;
                        }
                    }
                    if let Err(error) = publish(&staged, &final_path) {
                        tracing::warn!(%error, "a staged capture could not be published");
                        report.still_pending += 1;
                        return;
                    }
                }
            }
            Err(_) => {
                if !final_path.is_file() {
                    tracing::warn!(operation = %manifest.operation_id, "a save left no image behind");
                    abandon(manifest_path, None);
                    report.abandoned += 1;
                    return;
                }
            }
        }

        match self.register(repo, &manifest) {
            Ok(Registration::Registered(id)) => report.registered.push((id, final_path)),
            Ok(Registration::Pending) => report.still_pending += 1,
            Err(RegisterOutcome::Crashed(_)) => report.still_pending += 1,
        }
    }

    fn register(
        &self,
        repo: &mut HistoryRepo,
        manifest: &Manifest,
    ) -> Result<Registration, RegisterOutcome> {
        let manifest_path = self.manifest_path(&manifest.operation_id);
        match self.faults.at(FaultPoint::BeforeRegister) {
            Some(Fault::Crash) => {
                return Err(RegisterOutcome::Crashed(FaultPoint::BeforeRegister));
            }
            Some(Fault::Fail) => {
                tracing::warn!("History registration was refused by fault injection");
                return Ok(Registration::Pending);
            }
            None => {}
        }
        let recorded = match repo.record_operation(&manifest.operation_id, manifest.capture()) {
            Ok(recorded) => recorded,
            Err(error) => {
                log_history_failure(&error);
                return Ok(Registration::Pending);
            }
        };
        match self.faults.at(FaultPoint::AfterRegister) {
            Some(Fault::Crash) => {
                return Err(RegisterOutcome::Crashed(FaultPoint::AfterRegister));
            }
            Some(Fault::Fail) => return Ok(Registration::Registered(recorded.id)),
            None => {}
        }
        if let Err(error) = fs::remove_file(&manifest_path) {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(%error, "a completed save manifest could not be removed");
            }
        }
        Ok(Registration::Registered(recorded.id))
    }

    fn fault(
        &self,
        point: FaultPoint,
        manifest_path: &Path,
        staged: Option<&Path>,
    ) -> Result<(), DurableError> {
        match self.faults.at(point) {
            None => Ok(()),
            Some(Fault::Crash) => Err(DurableError::Crashed(point)),
            Some(Fault::Fail) if point == FaultPoint::AfterPublish => Ok(()),
            Some(Fault::Fail) => {
                abandon(manifest_path, staged);
                Err(DurableError::Write(io::Error::other(format!(
                    "the save failed at {point:?}"
                ))))
            }
        }
    }

    fn manifest_path(&self, operation_id: &str) -> PathBuf {
        self.directory
            .join(format!("{operation_id}.{MANIFEST_EXTENSION}"))
    }

    fn write_manifest(&self, manifest: &Manifest) -> io::Result<()> {
        let bytes = serde_json::to_vec(manifest).map_err(io::Error::other)?;
        write_atomic(&self.manifest_path(&manifest.operation_id), &bytes)
    }

    fn manifest_files(&self) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path.extension().and_then(std::ffi::OsStr::to_str)
                        == Some(MANIFEST_EXTENSION)
            })
            .collect();
        files.sort();
        files
    }

    fn remove_temporary_files(&self) {
        let Ok(entries) = fs::read_dir(&self.directory) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().and_then(std::ffi::OsStr::to_str) == Some("tmp") {
                let _ = fs::remove_file(path);
            }
        }
    }

    fn quarantine(&self, path: &Path) {
        let rejected = self.directory.join(REJECTED_DIR);
        let Some(name) = path.file_name() else {
            return;
        };
        if fs::create_dir_all(&rejected).is_ok() && fs::rename(path, rejected.join(name)).is_ok() {
            return;
        }
        let _ = fs::remove_file(path);
    }
}

enum RegisterOutcome {
    Crashed(FaultPoint),
}

fn log_history_failure(error: &HistoryError) {
    tracing::warn!(%error, "the capture could not be added to History and will be retried");
}

fn read_manifest(path: &Path) -> Result<Manifest, &'static str> {
    let bytes = fs::read(path).map_err(|_| "unreadable")?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|_| "malformed")?;
    if manifest.version != MANIFEST_VERSION {
        return Err("unsupported version");
    }
    if manifest.operation_id.is_empty()
        || !manifest
            .operation_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("invalid operation identifier");
    }
    if manifest.bytes < 0 || manifest.width <= 0 || manifest.height <= 0 {
        return Err("invalid dimensions");
    }
    let staged = Path::new(&manifest.staged_path);
    let final_path = Path::new(&manifest.final_path);
    if !staged.is_absolute() || !final_path.is_absolute() || staged.parent() != final_path.parent()
    {
        return Err("invalid paths");
    }
    Ok(manifest)
}

fn stage(path: &Path, encoded: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(path)?;
    file.write_all(encoded)?;
    file.sync_all()
}

fn publish(staged: &Path, final_path: &Path) -> io::Result<()> {
    fs::rename(staged, final_path)
}

fn abandon(manifest_path: &Path, staged: Option<&Path>) {
    if let Some(staged) = staged {
        let _ = fs::remove_file(staged);
    }
    let _ = fs::remove_file(manifest_path);
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use snapline_store::history::HistoryFilter;
    use tempfile::TempDir;

    use super::*;

    struct Once {
        point: FaultPoint,
        fault: Fault,
        spent: AtomicBool,
    }

    impl Once {
        fn new(point: FaultPoint, fault: Fault) -> Arc<Self> {
            Arc::new(Self {
                point,
                fault,
                spent: AtomicBool::new(false),
            })
        }
    }

    impl Faults for Once {
        fn at(&self, point: FaultPoint) -> Option<Fault> {
            if point == self.point && !self.spent.swap(true, Ordering::SeqCst) {
                Some(self.fault)
            } else {
                None
            }
        }
    }

    struct Fixture {
        directory: TempDir,
        repo: HistoryRepo,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().expect("temp dir");
            let repo =
                HistoryRepo::open_write(&directory.path().join("history.db")).expect("history");
            Self { directory, repo }
        }

        fn journal(&self) -> Journal {
            Journal::new(self.directory.path().join("pending"))
        }

        fn folder(&self) -> PathBuf {
            self.directory.path().join("shots")
        }

        fn manifest(&self, name: &str, operation: &str, bytes: &[u8]) -> Manifest {
            let final_path = self.folder().join(name);
            Manifest {
                version: MANIFEST_VERSION,
                operation_id: operation.to_owned(),
                final_path: final_path.to_string_lossy().into_owned(),
                staged_path: Journal::staged_path(&final_path, operation)
                    .to_string_lossy()
                    .into_owned(),
                bytes: i64::try_from(bytes.len()).expect("length"),
                width: 4,
                height: 3,
                source_app: Some("chrome.exe".to_owned()),
                created_at: 1_000,
            }
        }

        fn captures(&self) -> usize {
            self.repo
                .page(HistoryFilter::All, None)
                .expect("page")
                .items
                .len()
        }

        fn folder_files(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.folder())
                .map(|entries| {
                    entries
                        .filter_map(Result::ok)
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        }
    }

    const IMAGE: &[u8] = b"image-bytes";

    #[test]
    fn a_clean_save_registers_once_and_leaves_no_recovery_files() {
        let mut fixture = Fixture::new();
        let journal = fixture.journal();
        let manifest = fixture.manifest("a.png", "op-1", IMAGE);
        let committed = journal
            .commit(&mut fixture.repo, &manifest, IMAGE)
            .expect("commit");
        assert!(matches!(
            committed.registration,
            Registration::Registered(_)
        ));
        assert_eq!(fs::read(&committed.final_path).expect("image"), IMAGE);
        assert_eq!(journal.pending_count(), 0);
        assert_eq!(fixture.folder_files(), vec!["a.png".to_owned()]);
        assert_eq!(fixture.captures(), 1);
    }

    #[test]
    fn a_fault_at_every_boundary_leaves_a_recoverable_capture_or_a_clear_failure() {
        struct Expect {
            point: FaultPoint,
            fault: Fault,
            reports_failure: bool,
            captures_after_commit: usize,
            captures_after_restart: usize,
            files_after_restart: usize,
        }
        let table = [
            Expect {
                point: FaultPoint::AfterManifest,
                fault: Fault::Fail,
                reports_failure: true,
                captures_after_commit: 0,
                captures_after_restart: 0,
                files_after_restart: 0,
            },
            Expect {
                point: FaultPoint::AfterManifest,
                fault: Fault::Crash,
                reports_failure: true,
                captures_after_commit: 0,
                captures_after_restart: 0,
                files_after_restart: 0,
            },
            Expect {
                point: FaultPoint::AfterStage,
                fault: Fault::Fail,
                reports_failure: true,
                captures_after_commit: 0,
                captures_after_restart: 0,
                files_after_restart: 0,
            },
            Expect {
                point: FaultPoint::AfterStage,
                fault: Fault::Crash,
                reports_failure: true,
                captures_after_commit: 0,
                captures_after_restart: 1,
                files_after_restart: 1,
            },
            Expect {
                point: FaultPoint::AfterPublish,
                fault: Fault::Fail,
                reports_failure: false,
                captures_after_commit: 1,
                captures_after_restart: 1,
                files_after_restart: 1,
            },
            Expect {
                point: FaultPoint::AfterPublish,
                fault: Fault::Crash,
                reports_failure: true,
                captures_after_commit: 0,
                captures_after_restart: 1,
                files_after_restart: 1,
            },
            Expect {
                point: FaultPoint::BeforeRegister,
                fault: Fault::Fail,
                reports_failure: false,
                captures_after_commit: 0,
                captures_after_restart: 1,
                files_after_restart: 1,
            },
            Expect {
                point: FaultPoint::BeforeRegister,
                fault: Fault::Crash,
                reports_failure: true,
                captures_after_commit: 0,
                captures_after_restart: 1,
                files_after_restart: 1,
            },
            Expect {
                point: FaultPoint::AfterRegister,
                fault: Fault::Fail,
                reports_failure: false,
                captures_after_commit: 1,
                captures_after_restart: 1,
                files_after_restart: 1,
            },
            Expect {
                point: FaultPoint::AfterRegister,
                fault: Fault::Crash,
                reports_failure: true,
                captures_after_commit: 1,
                captures_after_restart: 1,
                files_after_restart: 1,
            },
        ];
        for expect in table {
            let mut fixture = Fixture::new();
            let faulty = fixture
                .journal()
                .with_faults(Once::new(expect.point, expect.fault));
            let manifest = fixture.manifest("a.png", "op-1", IMAGE);
            let result = faulty.commit(&mut fixture.repo, &manifest, IMAGE);
            let label = format!("{:?} {:?}", expect.point, expect.fault);
            assert_eq!(result.is_err(), expect.reports_failure, "{label}");
            assert_eq!(fixture.captures(), expect.captures_after_commit, "{label}");

            let restarted = fixture.journal();
            restarted.reconcile(&mut fixture.repo);
            assert_eq!(
                fixture.captures(),
                expect.captures_after_restart,
                "{label} after restart"
            );
            assert_eq!(restarted.pending_count(), 0, "{label} manifests");
            assert_eq!(
                fixture.folder_files().len(),
                expect.files_after_restart,
                "{label} files"
            );
            assert!(
                !fixture
                    .folder_files()
                    .iter()
                    .any(|name| name.ends_with(".part")),
                "{label} staged leftovers"
            );

            restarted.reconcile(&mut fixture.repo);
            assert_eq!(
                fixture.captures(),
                expect.captures_after_restart,
                "{label} after a second restart"
            );
        }
    }

    #[test]
    fn an_unavailable_history_keeps_the_image_and_registers_it_later_exactly_once() {
        let mut fixture = Fixture::new();
        let faulty = fixture
            .journal()
            .with_faults(Once::new(FaultPoint::BeforeRegister, Fault::Fail));
        let manifest = fixture.manifest("a.png", "op-1", IMAGE);
        let committed = faulty
            .commit(&mut fixture.repo, &manifest, IMAGE)
            .expect("commit");
        assert_eq!(committed.registration, Registration::Pending);
        assert!(committed.final_path.is_file());
        assert_eq!(faulty.pending_count(), 1);
        assert_eq!(fixture.captures(), 0);

        let stuck = fixture
            .journal()
            .with_faults(Once::new(FaultPoint::BeforeRegister, Fault::Fail));
        let report = stuck.reconcile(&mut fixture.repo);
        assert_eq!(report.still_pending, 1);
        assert!(report.registered.is_empty());
        assert_eq!(fixture.captures(), 0);

        let report = fixture.journal().reconcile(&mut fixture.repo);
        assert_eq!(report.registered.len(), 1);
        assert_eq!(report.registered[0].1, committed.final_path);
        assert_eq!(fixture.captures(), 1);
        assert_eq!(fixture.journal().pending_count(), 0);
    }

    #[test]
    fn a_save_into_an_unwritable_folder_fails_cleanly() {
        let mut fixture = Fixture::new();
        fs::write(fixture.folder(), b"not a folder").expect("blocker");
        let journal = fixture.journal();
        let manifest = fixture.manifest("a.png", "op-1", IMAGE);
        let error = journal
            .commit(&mut fixture.repo, &manifest, IMAGE)
            .expect_err("must fail");
        assert!(matches!(error, DurableError::Write(_)));
        assert_eq!(journal.pending_count(), 0);
        assert_eq!(fixture.captures(), 0);
    }

    #[test]
    fn an_incomplete_staged_file_is_discarded_not_published() {
        let mut fixture = Fixture::new();
        let journal = fixture.journal();
        let manifest = fixture.manifest("a.png", "op-1", IMAGE);
        journal.write_manifest(&manifest).expect("manifest");
        fs::create_dir_all(fixture.folder()).expect("folder");
        fs::write(&manifest.staged_path, &IMAGE[..3]).expect("partial");

        let report = journal.reconcile(&mut fixture.repo);
        assert_eq!(report.abandoned, 1);
        assert_eq!(fixture.captures(), 0);
        assert!(fixture.folder_files().is_empty());
        assert_eq!(journal.pending_count(), 0);
    }

    #[test]
    fn a_staged_file_whose_name_is_now_taken_is_published_under_the_next_free_name() {
        let mut fixture = Fixture::new();
        let journal = fixture.journal();
        let manifest = fixture.manifest("a.png", "op-1", IMAGE);
        journal.write_manifest(&manifest).expect("manifest");
        fs::create_dir_all(fixture.folder()).expect("folder");
        fs::write(&manifest.staged_path, IMAGE).expect("staged");
        fs::write(&manifest.final_path, b"somebody else's file").expect("taken");

        let report = journal.reconcile(&mut fixture.repo);
        assert_eq!(report.registered.len(), 1);
        let published = &report.registered[0].1;
        assert_eq!(
            published.file_name().and_then(std::ffi::OsStr::to_str),
            Some("a (2).png")
        );
        assert_eq!(fs::read(published).expect("published"), IMAGE);
        assert_eq!(
            fs::read(&manifest.final_path).expect("original"),
            b"somebody else's file"
        );
        assert_eq!(fixture.captures(), 1);
    }

    #[test]
    fn unreadable_or_unsupported_manifests_are_quarantined_not_acted_on() {
        let mut fixture = Fixture::new();
        let journal = fixture.journal();
        fs::create_dir_all(journal.directory()).expect("pending");
        fs::write(journal.directory().join("bad.json"), b"{not json").expect("malformed");
        let mut future = fixture.manifest("a.png", "op-2", IMAGE);
        future.version = MANIFEST_VERSION + 1;
        fs::write(
            journal.directory().join("op-2.json"),
            serde_json::to_vec(&future).expect("json"),
        )
        .expect("future");
        let mut escaping = fixture.manifest("a.png", "op-3", IMAGE);
        escaping.staged_path = fixture
            .directory
            .path()
            .join("elsewhere")
            .join("x.part")
            .to_string_lossy()
            .into_owned();
        fs::write(
            journal.directory().join("op-3.json"),
            serde_json::to_vec(&escaping).expect("json"),
        )
        .expect("escaping");

        let report = journal.reconcile(&mut fixture.repo);
        assert_eq!(report.rejected, 3);
        assert_eq!(journal.pending_count(), 0);
        assert_eq!(fixture.captures(), 0);
        let rejected = fs::read_dir(journal.directory().join(REJECTED_DIR))
            .expect("rejected folder")
            .count();
        assert_eq!(rejected, 3);
    }

    #[test]
    fn leftover_temporary_manifest_files_are_cleared() {
        let mut fixture = Fixture::new();
        let journal = fixture.journal();
        fs::create_dir_all(journal.directory()).expect("pending");
        let temporary = journal.directory().join("op-1.json.tmp");
        fs::write(&temporary, b"{").expect("temporary");
        journal.reconcile(&mut fixture.repo);
        assert!(!temporary.exists());
    }

    #[test]
    fn several_interrupted_saves_are_each_recovered_once() {
        let mut fixture = Fixture::new();
        for (name, operation) in [("a.png", "op-1"), ("b.png", "op-2"), ("c.png", "op-3")] {
            let faulty = fixture
                .journal()
                .with_faults(Once::new(FaultPoint::BeforeRegister, Fault::Crash));
            let manifest = fixture.manifest(name, operation, IMAGE);
            assert!(faulty.commit(&mut fixture.repo, &manifest, IMAGE).is_err());
        }
        let journal = fixture.journal();
        assert_eq!(journal.pending_count(), 3);
        let report = journal.reconcile(&mut fixture.repo);
        assert_eq!(report.registered.len(), 3);
        assert_eq!(fixture.captures(), 3);
        let report = journal.reconcile(&mut fixture.repo);
        assert!(report.registered.is_empty());
        assert_eq!(fixture.captures(), 3);
    }

    #[test]
    fn operation_identifiers_are_filename_safe_and_distinct() {
        let first = Journal::operation_id(1_700_000_000_123, 5, "k3f9");
        let second = Journal::operation_id(1_700_000_000_123, 6, "k3f9");
        assert_ne!(first, second);
        assert!(first
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-'));
        assert_eq!(
            Journal::operation_id(1, 1, "../..\\x y"),
            Journal::operation_id(1, 1, "xy")
        );
    }

    #[test]
    fn the_staged_name_sits_beside_the_final_file() {
        let staged = Journal::staged_path(Path::new("/shots/a.png"), "op-1");
        assert_eq!(staged.parent(), Some(Path::new("/shots")));
        assert_eq!(
            staged.file_name().and_then(std::ffi::OsStr::to_str),
            Some(".a.png.op-1.part")
        );
    }
}
