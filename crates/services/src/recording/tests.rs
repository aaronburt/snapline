use super::session::{cancel_requested, ffmpeg_reason, finish_failed_session, Command};
use crossbeam_channel::{unbounded, Receiver, Sender};
use snapline_core::image::ImageFormat;
use snapline_core::recording::RecordingMachine;
use snapline_store::history::{CaptureKind, HistoryRepo};
use std::ffi::OsStr;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Mutex;

use snapline_core::ffmpeg_args::{AudioSource, FfmpegCommand, PcmSampleFormat};
use snapline_core::geometry::Px;
use snapline_core::ports::{CodecError, ImageCodec};
use tempfile::{tempdir, TempDir};

use super::*;

struct TestCodec;

impl ImageCodec for TestCodec {
    fn encode(
        &self,
        _image: &Image,
        _format: ImageFormat,
        _quality: u8,
    ) -> Result<Vec<u8>, CodecError> {
        Ok(vec![9_u8; 32])
    }

    fn decode(&self, _bytes: &[u8]) -> Result<Image, CodecError> {
        Err(CodecError::Decode("not used".to_owned()))
    }
}

struct FakePicker {
    result: Result<PickedTarget, RecordError>,
    picks: AtomicUsize,
}

impl TargetPicker for FakePicker {
    fn pick(&self, _options: PickOptions) -> Result<PickedTarget, RecordError> {
        self.picks.fetch_add(1, Ordering::SeqCst);
        self.result.clone()
    }

    fn active_window(&self) -> Result<PickedTarget, RecordError> {
        self.picks.fetch_add(1, Ordering::SeqCst);
        self.result.clone()
    }
}

struct FakeBorderHost {
    shows: AtomicUsize,
    closes: Arc<AtomicUsize>,
    fail: bool,
}

struct FakeBorderHandle {
    closes: Arc<AtomicUsize>,
}

impl BorderHandle for FakeBorderHandle {
    fn close(&mut self) {
        self.closes.fetch_add(1, Ordering::SeqCst);
    }
}

impl BorderHost for FakeBorderHost {
    fn show(&self, _request: &BorderRequest) -> Result<Box<dyn BorderHandle>, RecordError> {
        self.shows.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(RecordError::Border("window".to_owned()));
        }
        Ok(Box::new(FakeBorderHandle {
            closes: Arc::clone(&self.closes),
        }))
    }
}

struct FakeControlsHost {
    shows: AtomicUsize,
    closes: Arc<AtomicUsize>,
    stops: Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
    cancels: Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
}

struct FakeControlsHandle {
    closes: Arc<AtomicUsize>,
}

impl ControlsHandle for FakeControlsHandle {
    fn close(&mut self) {
        self.closes.fetch_add(1, Ordering::SeqCst);
    }
}

impl ControlsHost for FakeControlsHost {
    fn show(&self, request: &ControlsRequest) -> Result<Box<dyn ControlsHandle>, RecordError> {
        self.shows.fetch_add(1, Ordering::SeqCst);
        self.stops
            .lock()
            .expect("stops")
            .push(Arc::clone(&request.stop));
        self.cancels
            .lock()
            .expect("cancels")
            .push(Arc::clone(&request.cancel));
        Ok(Box::new(FakeControlsHandle {
            closes: Arc::clone(&self.closes),
        }))
    }
}

struct FakeAudio {
    starts: Mutex<Vec<AudioOptions>>,
    stops: Arc<AtomicUsize>,
}

struct FakeAudioSession {
    pipes: Vec<AudioPipe>,
    stops: Arc<AtomicUsize>,
}

impl AudioSession for FakeAudioSession {
    fn pipes(&self) -> Vec<AudioPipe> {
        self.pipes.clone()
    }

    fn stop(&mut self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
    }
}

impl AudioPort for FakeAudio {
    fn start(&self, options: &AudioOptions) -> Result<Box<dyn AudioSession>, RecordError> {
        self.starts
            .lock()
            .expect("audio starts")
            .push(options.clone());
        Ok(Box::new(FakeAudioSession {
            pipes: vec![
                pipe(0, AudioSource::System),
                pipe(1, AudioSource::Microphone),
            ],
            stops: Arc::clone(&self.stops),
        }))
    }
}

fn pipe(index: u32, source: AudioSource) -> AudioPipe {
    AudioPipe {
        source,
        index,
        sample_format: PcmSampleFormat::S16Le,
        sample_rate: 48_000,
        channels: 2,
    }
}

struct FakeRunner {
    commands: Mutex<Vec<Vec<String>>>,
    fail: AtomicBool,
    stops: Arc<AtomicUsize>,
    kills: Arc<AtomicUsize>,
    encoder: VideoEncoder,
    outputs: Mutex<Vec<OutputBounds>>,
}

struct FakeProcess {
    running: bool,
    stops: Arc<AtomicUsize>,
    kills: Arc<AtomicUsize>,
}

impl FfmpegProcess for FakeProcess {
    fn progress(&mut self) -> Option<FfmpegProgress> {
        None
    }

    fn is_running(&mut self) -> bool {
        self.running
    }

    fn stop(&mut self) -> Result<FfmpegExit, RecordError> {
        self.running = false;
        self.stops.fetch_add(1, Ordering::SeqCst);
        Ok(FfmpegExit::success())
    }

    fn kill(&mut self) -> Result<FfmpegExit, RecordError> {
        self.running = false;
        self.kills.fetch_add(1, Ordering::SeqCst);
        Ok(FfmpegExit::success())
    }

    fn wait(&mut self) -> Result<FfmpegExit, RecordError> {
        self.running = false;
        Ok(FfmpegExit::success())
    }
}

impl FfmpegRunner for FakeRunner {
    fn start(&self, command: FfmpegCommand) -> Result<Box<dyn FfmpegProcess>, RecordError> {
        let args = command
            .args()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        self.commands.lock().expect("commands").push(args);
        if self.fail.load(Ordering::SeqCst) {
            return Err(RecordError::FfmpegStart("spawn".to_owned()));
        }
        Ok(Box::new(FakeProcess {
            running: true,
            stops: Arc::clone(&self.stops),
            kills: Arc::clone(&self.kills),
        }))
    }

    fn best_encoder(&self, _ffmpeg: &Path, _format: VideoFormat) -> VideoEncoder {
        self.encoder
    }

    fn outputs(&self) -> Vec<OutputBounds> {
        self.outputs.lock().expect("outputs").clone()
    }
}

struct FakeMedia {
    calls: AtomicUsize,
}

impl MediaPort for FakeMedia {
    fn first_frame(&self, _video: &Path, _ffmpeg: &Path) -> Result<Image, RecordError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Image::bgra(320, 180).map_err(|error| RecordError::Save(error.to_string()))
    }
}

struct FakeStore {
    files: Mutex<Vec<RecordedFile>>,
}

impl RecordingStore for FakeStore {
    fn save(&self, file: RecordedFile) -> Result<SavedRecording, RecordError> {
        let saved = SavedRecording {
            capture_id: Some(1),
            path: file.media.clone(),
            file_name: file
                .media
                .file_name()
                .and_then(OsStr::to_str)
                .unwrap_or("recording.mp4")
                .to_owned(),
            bytes: 4,
            width: file.width,
            height: file.height,
            duration_ms: file.duration_ms,
            kind: file.kind,
        };
        self.files.lock().expect("files").push(file);
        Ok(saved)
    }
}

struct FakePublisher {
    published: Mutex<Vec<SavedRecording>>,
    cancelled: AtomicUsize,
    failures: Mutex<Vec<String>>,
}

impl RecordingPublisher for FakePublisher {
    fn failed(&self, message: &str) {
        self.failures
            .lock()
            .expect("failures")
            .push(message.to_owned());
    }

    fn published(&self, saved: &SavedRecording) {
        self.published
            .lock()
            .expect("published")
            .push(saved.clone());
    }

    fn cancelled(&self) {
        self.cancelled.fetch_add(1, Ordering::SeqCst);
    }
}

struct FakeSink {
    events: Sender<AppEvent>,
}

impl EventSink for FakeSink {
    fn emit(&self, event: AppEvent) {
        let _ = self.events.try_send(event);
    }
}

struct Fixture {
    directory: TempDir,
    services: Services,
    picker: Arc<FakePicker>,
    border: Arc<FakeBorderHost>,
    controls: Arc<FakeControlsHost>,
    audio: Arc<FakeAudio>,
    runner: Arc<FakeRunner>,
    media: Arc<FakeMedia>,
    store: Arc<FakeStore>,
    publisher: Arc<FakePublisher>,
    sender: Sender<AppEvent>,
    events: Receiver<AppEvent>,
}

impl Fixture {
    fn new(
        target: Result<PickedTarget, RecordError>,
        fail_border: bool,
        fail_ffmpeg: bool,
    ) -> Self {
        let directory = tempdir().expect("temp");
        let services = Services::with_paths(paths(directory.path())).expect("services");
        let (sender, events) = unbounded();
        Self {
            picker: Arc::new(FakePicker {
                result: target,
                picks: AtomicUsize::new(0),
            }),
            border: Arc::new(FakeBorderHost {
                shows: AtomicUsize::new(0),
                closes: Arc::new(AtomicUsize::new(0)),
                fail: fail_border,
            }),
            controls: Arc::new(FakeControlsHost {
                shows: AtomicUsize::new(0),
                closes: Arc::new(AtomicUsize::new(0)),
                stops: Mutex::new(Vec::new()),
                cancels: Mutex::new(Vec::new()),
            }),
            audio: Arc::new(FakeAudio {
                starts: Mutex::new(Vec::new()),
                stops: Arc::new(AtomicUsize::new(0)),
            }),
            runner: Arc::new(FakeRunner {
                commands: Mutex::new(Vec::new()),
                fail: AtomicBool::new(fail_ffmpeg),
                stops: Arc::new(AtomicUsize::new(0)),
                kills: Arc::new(AtomicUsize::new(0)),
                encoder: VideoEncoder::H264Nvenc,
                outputs: Mutex::new(Vec::new()),
            }),
            media: Arc::new(FakeMedia {
                calls: AtomicUsize::new(0),
            }),
            store: Arc::new(FakeStore {
                files: Mutex::new(Vec::new()),
            }),
            publisher: Arc::new(FakePublisher {
                published: Mutex::new(Vec::new()),
                cancelled: AtomicUsize::new(0),
                failures: Mutex::new(Vec::new()),
            }),
            directory,
            services,
            sender,
            events,
        }
    }

    fn sink(&self) -> Arc<FakeSink> {
        Arc::new(FakeSink {
            events: self.sender.clone(),
        })
    }

    fn ports(&self) -> RecordingPorts {
        RecordingPorts {
            picker: Arc::clone(&self.picker) as Arc<dyn TargetPicker>,
            border: Arc::clone(&self.border) as Arc<dyn BorderHost>,
            controls: Arc::clone(&self.controls) as Arc<dyn ControlsHost>,
            ffmpeg: Arc::clone(&self.runner) as Arc<dyn FfmpegRunner>,
            audio: Arc::clone(&self.audio) as Arc<dyn AudioPort>,
            media: Arc::clone(&self.media) as Arc<dyn MediaPort>,
            store: Arc::clone(&self.store) as Arc<dyn RecordingStore>,
            events: self.sink() as Arc<dyn EventSink>,
            publisher: Arc::clone(&self.publisher) as Arc<dyn RecordingPublisher>,
        }
    }

    fn plan(
        &self,
        kind: RecordedKind,
        mode: SessionMode,
        border: bool,
        audio: bool,
    ) -> SessionPlan {
        let output = self.directory.path().join("capture.mp4");
        if kind == RecordedKind::Gif {
            std::fs::write(&output, b"stub").expect("stub");
        }
        SessionPlan {
            kind,
            mode,
            target: target(),
            output,
            converted: (kind == RecordedKind::Gif)
                .then(|| self.directory.path().join("capture.gif")),
            settings: SessionSettings {
                ffmpeg: PathBuf::from(r"C:\Tools\ffmpeg.exe"),
                frame_rate: VideoFrameRate::Fps30,
                quality: VideoQuality::High,
                format: VideoFormat::Mp4H264,
                gif: GifSettings::default(),
                border,
                audio: AudioOptions {
                    system: audio,
                    microphone: audio,
                    microphone_device: DEFAULT_MICROPHONE.to_owned(),
                },
                theme: OverlayTheme::Dark,
            },
        }
    }
}

fn paths(directory: &Path) -> Paths {
    Paths::resolve(
        &directory.join("exe"),
        &directory.join("Roaming"),
        &directory.join("Local"),
        &directory.join("Profile"),
        false,
    )
}

fn target() -> PickedTarget {
    PickedTarget {
        rect: Rect::new(Px(10), Px(20), Px(640), Px(360)),
        monitor: 3,
        window: None,
    }
}

fn wait_for_state(events: &Receiver<AppEvent>, wanted: RecordStateDto) {
    loop {
        match events.recv_timeout(Duration::from_secs(5)) {
            Ok(AppEvent::Record(Event::State(state))) if state.state == wanted => return,
            Ok(_) => {}
            Err(error) => panic!("the recording never reached {wanted:?}: {error}"),
        }
    }
}

fn drain_until_history(events: &Receiver<AppEvent>) -> (bool, bool) {
    let mut done = false;
    let mut history = false;
    for _ in 0..12 {
        match events.recv_timeout(Duration::from_millis(200)) {
            Ok(AppEvent::Record(Event::State(state))) => {
                done |= state.state == RecordStateDto::Done;
            }
            Ok(AppEvent::History(crate::history::Event::Added { .. })) => history = true,
            Ok(_) => {}
            Err(_) => break,
        }
        if done && history {
            break;
        }
    }
    (done, history)
}

fn commands(runner: &FakeRunner) -> Vec<Vec<String>> {
    runner.commands.lock().expect("commands").clone()
}

#[test]
fn the_control_bar_stop_button_ends_the_recording_and_the_bar_closes() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let plan = fixture.plan(RecordedKind::Video, SessionMode::Save, false, false);
    let session = Session::start(plan, fixture.ports()).expect("start");
    let control = session.control();
    wait_for_state(&fixture.events, RecordStateDto::Recording);
    assert_eq!(fixture.controls.shows.load(Ordering::SeqCst), 1);
    let press = Arc::clone(&fixture.controls.stops.lock().expect("stops")[0]);
    press();
    session.wait().expect("finish");
    assert_eq!(control.status().state, RecordingState::Done);
    assert_eq!(fixture.controls.closes.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.border.shows.load(Ordering::SeqCst), 0);
}

fn start_recording_fixture(fixture: &Fixture) -> (Session, SessionControl, PathBuf) {
    let plan = fixture.plan(RecordedKind::Video, SessionMode::Save, true, false);
    let output = plan.output.clone();
    std::fs::write(&output, b"bad take").expect("temp recording");
    let session = Session::start(plan, fixture.ports()).expect("start");
    let control = session.control();
    wait_for_state(&fixture.events, RecordStateDto::Recording);
    (session, control, output)
}

#[test]
fn cancelling_discards_the_recording_and_publishes_nothing() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let (session, control, output) = start_recording_fixture(&fixture);
    control.cancel();
    let error = session.wait().expect_err("cancelled");
    assert!(matches!(error, RecordError::Cancelled));
    assert!(!output.exists());
    assert!(fixture.store.files.lock().expect("files").is_empty());
    assert!(fixture
        .publisher
        .published
        .lock()
        .expect("published")
        .is_empty());
    assert_eq!(fixture.publisher.cancelled.load(Ordering::SeqCst), 1);
    assert_eq!(control.status().state, RecordingState::Idle);
    assert_eq!(fixture.border.closes.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.controls.closes.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.runner.kills.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.runner.stops.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.media.calls.load(Ordering::SeqCst), 0);
    let (_, history) = drain_until_history(&fixture.events);
    assert!(!history);
}

#[test]
fn the_bar_cancel_button_routes_to_the_cancel_command() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let (session, control, output) = start_recording_fixture(&fixture);
    let press = Arc::clone(&fixture.controls.cancels.lock().expect("cancels")[0]);
    press();
    let error = session.wait().expect_err("cancelled");
    assert!(matches!(error, RecordError::Cancelled));
    assert!(!output.exists());
    assert_eq!(control.status().state, RecordingState::Idle);
    assert!(fixture.store.files.lock().expect("files").is_empty());
}

#[test]
fn cancelling_after_the_recording_is_stored_changes_nothing() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let plan = fixture.plan(RecordedKind::Video, SessionMode::Save, false, false);
    let session = Session::start(plan, fixture.ports()).expect("start");
    let control = session.control();
    wait_for_state(&fixture.events, RecordStateDto::Recording);
    control.stop();
    session.wait().expect("finish");
    control.cancel();
    assert_eq!(control.status().state, RecordingState::Done);
    assert_eq!(fixture.store.files.lock().expect("files").len(), 1);
    assert_eq!(fixture.publisher.cancelled.load(Ordering::SeqCst), 0);
}

#[test]
fn a_cancel_queued_behind_a_stop_is_still_seen() {
    let (sender, inbox) = unbounded();
    sender.send(Command::Stop).expect("stop");
    assert!(!cancel_requested(&inbox));
    sender.send(Command::Stop).expect("stop");
    sender.send(Command::Cancel).expect("cancel");
    assert!(cancel_requested(&inbox));
}

#[test]
fn the_cancel_action_is_registered_and_runs_without_a_recording() {
    let services = Services::new();
    assert_eq!(run(&ActionId::new("record_cancel"), &services), Ok(()));
}

#[test]
fn a_saved_recording_is_handed_to_the_publisher() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let plan = fixture.plan(RecordedKind::Video, SessionMode::Save, false, false);
    let session = Session::start(plan, fixture.ports()).expect("start");
    let control = session.control();
    wait_for_state(&fixture.events, RecordStateDto::Recording);
    control.stop();
    session.wait().expect("finish");
    let published = fixture.publisher.published.lock().expect("published");
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].capture_id, Some(1));
    assert_eq!(published[0].file_name, "capture.mp4");
}

#[test]
fn a_temporary_recording_is_not_published() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let plan = fixture.plan(RecordedKind::Video, SessionMode::Temporary, false, false);
    let session = Session::start(plan, fixture.ports()).expect("start");
    let control = session.control();
    wait_for_state(&fixture.events, RecordStateDto::Recording);
    control.stop();
    session.wait().expect("finish");
    assert!(fixture
        .publisher
        .published
        .lock()
        .expect("published")
        .is_empty());
}

#[test]
fn the_saved_notice_names_the_file_size_and_duration() {
    let saved = SavedRecording {
        capture_id: Some(9),
        path: PathBuf::from(r"C:\Pics\Snap line\clip.mp4"),
        file_name: "clip.mp4".to_owned(),
        bytes: 1_572_864,
        width: 640,
        height: 360,
        duration_ms: 75_400,
        kind: RecordedKind::Video,
    };
    let thumbnail = PathBuf::from(r"C:\Thumbs\9.webp");
    let notice = recording_saved_notice(&saved, Some(thumbnail.clone()));
    assert_eq!(notice.title, "Recording saved");
    assert_eq!(notice.lines, ["clip.mp4", "1.5 MB, 1:15"]);
    assert_eq!(notice.tag.as_deref(), Some("capture-9"));
    assert_eq!(notice.image, Some(thumbnail));
    assert_eq!(
        notice.open.as_deref(),
        Some("file:///C:/Pics/Snap%20line/clip.mp4")
    );
    let labels: Vec<&str> = notice.buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels, ["Open video", "Open folder"]);
}

#[test]
fn start_stop_records_audio_saves_and_closes_everything() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let plan = fixture.plan(RecordedKind::Video, SessionMode::Save, true, true);
    let output = plan.output.clone();
    let session = Session::start(plan, fixture.ports()).expect("start");
    let control = session.control();
    wait_for_state(&fixture.events, RecordStateDto::Recording);
    control.stop();
    let outcome = session.wait().expect("finish");
    assert_eq!(outcome.path, output);
    assert_eq!(control.status().state, RecordingState::Done);
    assert!(control.status().duration_ms <= 5);
    assert_eq!(fixture.border.closes.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.controls.closes.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.audio.stops.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.runner.stops.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.runner.kills.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.picker.picks.load(Ordering::SeqCst), 0);
    let sent = commands(&fixture.runner);
    assert_eq!(sent.len(), 1);
    let args = &sent[0];
    assert!(args.iter().any(|argument| argument == "h264_nvenc"));
    assert!(args.iter().any(|argument| argument
        == "ddagrab=framerate=30:output_idx=0:video_size=640x360:offset_x=10:offset_y=20"));
    assert!(args
        .iter()
        .any(|argument| argument == r"\\.\pipe\snapline-audio-0"));
    assert!(args
        .iter()
        .any(|argument| argument == r"\\.\pipe\snapline-audio-1"));
    assert!(args
        .iter()
        .any(|argument| argument == "[1:a][2:a]amix=inputs=2[aout]"));
    let starts = fixture.audio.starts.lock().expect("audio starts");
    assert_eq!(starts.len(), 1);
    assert!(starts[0].system);
    assert!(starts[0].microphone);
    let files = fixture.store.files.lock().expect("files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].kind, RecordedKind::Video);
    assert_eq!(files[0].width, 640);
    assert_eq!(files[0].height, 360);
    assert_eq!(fixture.media.calls.load(Ordering::SeqCst), 1);
    let (done, history) = drain_until_history(&fixture.events);
    assert!(done);
    assert!(history);
}

#[test]
fn the_status_events_follow_the_state_machine() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let session = Session::start(
        fixture.plan(RecordedKind::Video, SessionMode::Save, false, false),
        fixture.ports(),
    )
    .expect("start");
    let control = session.control();
    let mut states = Vec::new();
    loop {
        match fixture.events.recv_timeout(Duration::from_secs(5)) {
            Ok(AppEvent::Record(Event::State(state))) => {
                let reached = state.state == RecordStateDto::Recording;
                states.push(state.state);
                if reached {
                    break;
                }
            }
            Ok(_) => {}
            Err(error) => panic!("the recording never started: {error}"),
        }
    }
    control.stop();
    assert!(session.wait().is_ok());
    while let Ok(event) = fixture.events.recv_timeout(Duration::from_millis(300)) {
        if let AppEvent::Record(Event::State(state)) = event {
            states.push(state.state);
        }
    }
    states.dedup();
    assert_eq!(
        states,
        vec![
            RecordStateDto::Arming,
            RecordStateDto::Recording,
            RecordStateDto::Stopping,
            RecordStateDto::Encoding,
            RecordStateDto::Done,
        ]
    );
}

#[test]
fn stopping_before_recording_starts_still_finishes() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let session = Session::start(
        fixture.plan(RecordedKind::Video, SessionMode::Save, false, false),
        fixture.ports(),
    )
    .expect("start");
    let control = session.control();
    control.stop();
    assert!(session.wait().is_ok());
    assert_eq!(control.status().state, RecordingState::Done);
    assert_eq!(fixture.border.shows.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.audio.starts.lock().expect("audio").len(), 0);
}

fn run_on_outputs(outputs: Vec<OutputBounds>) -> (Fixture, Result<SessionOutcome, RecordError>) {
    let fixture = Fixture::new(Ok(target()), false, false);
    *fixture.runner.outputs.lock().expect("outputs") = outputs;
    let session = Session::start(
        fixture.plan(RecordedKind::Video, SessionMode::Save, false, false),
        fixture.ports(),
    )
    .expect("start");
    let control = session.control();
    std::thread::sleep(Duration::from_millis(60));
    control.stop();
    let result = session.wait();
    (fixture, result)
}

#[test]
fn a_secondary_screen_uses_its_own_output_and_relative_offsets() {
    let outputs = vec![
        OutputBounds {
            adapter: 0,
            index: 0,
            bounds: snapline_core::geometry::Rect::new(
                snapline_core::geometry::Px(1920),
                snapline_core::geometry::Px(0),
                snapline_core::geometry::Px(1920),
                snapline_core::geometry::Px(1080),
            ),
        },
        OutputBounds {
            adapter: 0,
            index: 2,
            bounds: snapline_core::geometry::Rect::new(
                snapline_core::geometry::Px(-30),
                snapline_core::geometry::Px(0),
                snapline_core::geometry::Px(1920),
                snapline_core::geometry::Px(1080),
            ),
        },
    ];
    let (fixture, result) = run_on_outputs(outputs);
    assert!(result.is_ok());
    let sent = commands(&fixture.runner);
    assert!(sent[0].iter().any(|argument| argument
        == "ddagrab=framerate=30:output_idx=2:video_size=640x360:offset_x=40:offset_y=20"));
}

#[test]
fn an_area_spanning_two_screens_fails_with_a_clear_message_and_no_ffmpeg() {
    let outputs = vec![
        OutputBounds {
            adapter: 0,
            index: 0,
            bounds: snapline_core::geometry::Rect::new(
                snapline_core::geometry::Px(-1620),
                snapline_core::geometry::Px(0),
                snapline_core::geometry::Px(1920),
                snapline_core::geometry::Px(1080),
            ),
        },
        OutputBounds {
            adapter: 0,
            index: 1,
            bounds: snapline_core::geometry::Rect::new(
                snapline_core::geometry::Px(300),
                snapline_core::geometry::Px(0),
                snapline_core::geometry::Px(1920),
                snapline_core::geometry::Px(1080),
            ),
        },
    ];
    let (fixture, result) = run_on_outputs(outputs);
    let error = result.expect_err("spans screens");
    assert!(error.to_string().contains("single screen"));
    assert!(commands(&fixture.runner).is_empty());
    let failures = fixture.publisher.failures.lock().expect("failures");
    assert_eq!(failures.len(), 1);
    assert!(failures[0].contains("single screen"));
}

fn failed_session(
    fixture: &Fixture,
    mode: SessionMode,
    error: RecordError,
) -> (PathBuf, RecordError) {
    let plan = fixture.plan(RecordedKind::Video, mode, true, false);
    let output = plan.output.clone();
    std::fs::write(&output, b"partial take").expect("partial recording");
    let status = Arc::new(Mutex::new(RecordStatus::default()));
    let mut machine = RecordingMachine::new();
    let error = finish_failed_session(&plan, &fixture.ports(), &mut machine, &status, error);
    (output, error)
}

#[test]
fn a_recording_cut_short_by_ffmpeg_keeps_the_part_that_was_written() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let (output, error) = failed_session(
        &fixture,
        SessionMode::Save,
        RecordError::FfmpegStopped("FFmpeg stopped before the recording was stopped.".to_owned()),
    );
    assert!(output.exists());
    assert!(error.to_string().contains("kept at"));
    let failures = fixture.publisher.failures.lock().expect("failures");
    assert_eq!(failures.len(), 1);
    assert!(was_reported(&error.to_string()));
}

#[test]
fn other_failures_and_temporary_sessions_discard_the_file() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let (output, _) = failed_session(
        &fixture,
        SessionMode::Save,
        RecordError::Audio("no device".to_owned()),
    );
    assert!(!output.exists());
    let (output, _) = failed_session(
        &fixture,
        SessionMode::Temporary,
        RecordError::FfmpegStopped("stopped".to_owned()),
    );
    assert!(!output.exists());
}

#[test]
fn ffmpeg_reasons_keep_only_the_last_lines_of_stderr() {
    assert_eq!(ffmpeg_reason(""), "");
    assert_eq!(ffmpeg_reason("a\n\nb\nc\nd\n"), "b c d".to_owned());
}

#[test]
fn a_border_failure_fails_the_recording_without_ffmpeg() {
    let fixture = Fixture::new(Ok(target()), true, false);
    let session = Session::start(
        fixture.plan(RecordedKind::Video, SessionMode::Save, true, true),
        fixture.ports(),
    )
    .expect("start");
    let control = session.control();
    let error = session.wait().expect_err("border failure");
    assert!(matches!(error, RecordError::Border(_)));
    assert_eq!(control.status().state, RecordingState::Failed);
    assert!(control.status().error.is_some());
    assert!(commands(&fixture.runner).is_empty());
    assert_eq!(fixture.audio.starts.lock().expect("audio").len(), 0);
    assert_eq!(fixture.store.files.lock().expect("files").len(), 0);
}

#[test]
fn an_ffmpeg_start_failure_closes_the_border_and_audio() {
    let fixture = Fixture::new(Ok(target()), false, true);
    let session = Session::start(
        fixture.plan(RecordedKind::Video, SessionMode::Save, true, true),
        fixture.ports(),
    )
    .expect("start");
    let control = session.control();
    let error = session.wait().expect_err("ffmpeg failure");
    assert!(matches!(error, RecordError::FfmpegStart(_)));
    assert_eq!(control.status().state, RecordingState::Failed);
    assert_eq!(fixture.border.closes.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.audio.stops.load(Ordering::SeqCst), 1);
    assert!(fixture.store.files.lock().expect("files").is_empty());
}

#[test]
fn cancelling_the_picker_shows_no_border_and_starts_no_thread() {
    let fixture = Fixture::new(Err(RecordError::Cancelled), false, false);
    let result = begin_with(
        &fixture.services,
        fixture.ports(),
        RecordKindDto::Video,
        SessionMode::Save,
    )
    .expect("begin");
    assert!(result.is_none());
    assert_eq!(fixture.picker.picks.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.border.shows.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.border.closes.load(Ordering::SeqCst), 0);
    assert!(commands(&fixture.runner).is_empty());
}

#[test]
fn an_empty_target_is_rejected_before_the_border() {
    let fixture = Fixture::new(
        Ok(PickedTarget {
            rect: Rect::ZERO,
            monitor: 1,
            window: None,
        }),
        false,
        false,
    );
    let error = match begin_with(
        &fixture.services,
        fixture.ports(),
        RecordKindDto::Video,
        SessionMode::Save,
    ) {
        Ok(_) => panic!("the empty target was accepted"),
        Err(error) => error,
    };
    assert_eq!(error, RecordError::NoTarget);
    assert_eq!(fixture.border.shows.load(Ordering::SeqCst), 0);
}

#[test]
fn a_window_recording_uses_the_active_window() {
    let window = PickedTarget {
        rect: Rect::new(Px(0), Px(0), Px(320), Px(200)),
        monitor: 1,
        window: Some(PickedWindow {
            handle: 42,
            title: "Editor".to_owned(),
        }),
    };
    let fixture = Fixture::new(Ok(window.clone()), false, false);
    let mut plan = fixture.plan(RecordedKind::Video, SessionMode::Save, false, false);
    plan.target = window.clone();
    assert_eq!(
        plan.target.recording_target(),
        RecordingTarget::window(WindowId::new(42), plan.target.rect)
    );
    assert_eq!(plan.target.title(), "Editor");
    let result = begin_with(
        &fixture.services,
        fixture.ports(),
        RecordKindDto::Window,
        SessionMode::Save,
    )
    .expect("begin");
    assert!(result.is_some());
    assert_eq!(fixture.picker.picks.load(Ordering::SeqCst), 1);
}

#[test]
fn a_gif_recording_converts_the_lossless_capture_before_saving() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let plan = fixture.plan(RecordedKind::Gif, SessionMode::Save, false, false);
    let converted = plan.converted.clone().expect("converted");
    let session = Session::start(plan, fixture.ports()).expect("start");
    wait_for_state(&fixture.events, RecordStateDto::Recording);
    session.control().stop();
    let outcome = session.wait().expect("finish");
    assert_eq!(outcome.path, converted);
    let sent = commands(&fixture.runner);
    assert_eq!(sent.len(), 2);
    assert!(sent[0].iter().any(|argument| argument == "libx264"));
    assert!(sent[0]
        .windows(2)
        .any(|pair| pair[0] == "-crf" && pair[1] == "0"));
    let filter = sent[1]
        .iter()
        .position(|argument| argument == "-filter_complex")
        .and_then(|index| sent[1].get(index + 1))
        .expect("gif filter");
    assert!(filter.contains("palettegen"));
    assert!(filter.contains("paletteuse"));
    assert!(filter.contains("fps=15"));
    let files = fixture.store.files.lock().expect("files");
    assert_eq!(files[0].kind, RecordedKind::Gif);
    assert_eq!(files[0].media, converted);
}

#[test]
fn a_temporary_recording_is_never_saved_to_history() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let plan = fixture.plan(RecordedKind::Video, SessionMode::Temporary, false, false);
    let output = plan.output.clone();
    let session = Session::start(plan, fixture.ports()).expect("start");
    wait_for_state(&fixture.events, RecordStateDto::Recording);
    session.control().stop();
    let outcome = session.wait().expect("finish");
    assert_eq!(outcome.path, output);
    assert!(fixture.store.files.lock().expect("files").is_empty());
    assert_eq!(fixture.media.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn every_video_setting_reaches_the_request() {
    let mut settings = Settings::default();
    settings.capture.video_format = StoreVideoFormat::MkvH264;
    settings.capture.frame_rate = FrameRate::Fps60;
    settings.capture.video_quality = StoreVideoQuality::SmallFile;
    settings.capture.record_system_audio = true;
    settings.capture.record_microphone = true;
    settings.capture.recording_border = false;
    let directory = tempdir().expect("temp");
    let resolved = paths(directory.path());
    let mapped = session_settings(&settings, &resolved);
    assert_eq!(mapped.format, VideoFormat::MkvH264);
    assert_eq!(mapped.frame_rate, VideoFrameRate::Fps60);
    assert_eq!(mapped.quality, VideoQuality::SmallFile);
    assert!(mapped.audio.system);
    assert!(mapped.audio.microphone);
    assert!(!mapped.border);
    assert_eq!(mapped.ffmpeg, resolved.exe_dir.join("ffmpeg.exe"));
}

#[test]
fn every_gif_setting_reaches_the_filter() {
    let mut settings = Settings::default();
    settings.capture.gif_frame_rate = StoreGifFrameRate::Fps20;
    settings.capture.gif_max_width = StoreGifMaxWidth::Px480;
    settings.capture.gif_optimise_palette = false;
    let mapped = gif_settings(&settings);
    assert_eq!(mapped.frame_rate, GifFrameRate::Fps20);
    assert_eq!(mapped.max_width, GifMaxWidth::Px480);
    assert!(!mapped.optimise_palette);
}

#[test]
fn converting_a_recording_uses_the_palette_filter() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let input = fixture.directory.path().join("clip.mp4");
    let output = fixture.directory.path().join("clip.gif");
    std::fs::write(&input, b"stub").expect("stub");
    let converted = convert_with(
        fixture.runner.as_ref(),
        Path::new(r"C:\Tools\ffmpeg.exe"),
        &input,
        &output,
        GifSettings::default(),
    )
    .expect("convert");
    assert_eq!(converted, output);
    let sent = commands(&fixture.runner);
    assert_eq!(sent.len(), 1);
    let filter = sent[0]
        .iter()
        .position(|argument| argument == "-filter_complex")
        .and_then(|index| sent[0].get(index + 1))
        .expect("gif filter");
    assert!(filter.contains("palettegen=stats_mode=diff"));
    assert!(filter.contains("paletteuse"));
    assert_eq!(sent[0].last().map(String::as_str), output.to_str());
}

#[test]
fn converting_a_missing_recording_reports_it() {
    let fixture = Fixture::new(Ok(target()), false, false);
    let error = convert_with(
        fixture.runner.as_ref(),
        Path::new(r"C:\Tools\ffmpeg.exe"),
        &fixture.directory.path().join("missing.mp4"),
        &fixture.directory.path().join("missing.gif"),
        GifSettings::default(),
    )
    .expect_err("missing");
    assert!(matches!(error, RecordError::Convert(_)));
    assert!(commands(&fixture.runner).is_empty());
}

fn store_services(directory: &Path) -> Services {
    let factory: Arc<crate::CodecFactory> = Arc::new(|| {
        let codec: Arc<dyn ImageCodec> = Arc::new(TestCodec);
        Ok(codec)
    });
    Services::with_paths(paths(directory))
        .expect("services")
        .with_codec_factory(factory)
}

#[test]
fn a_saved_recording_gets_a_name_folder_history_row_and_thumbnail() {
    let directory = tempdir().expect("temp");
    let services = store_services(directory.path());
    let media = directory.path().join("source.mp4");
    std::fs::write(&media, b"video-bytes").expect("media");
    let saved = store_recording(
        &services,
        RecordedFile {
            media: media.clone(),
            first_frame: Image::bgra(640, 360).ok(),
            kind: RecordedKind::Video,
            width: 640,
            height: 360,
            duration_ms: 2_500,
            source_app: None,
            window_title: "Recording".to_owned(),
        },
    )
    .expect("save");
    assert!(!media.exists());
    assert!(saved.path.is_file());
    assert!(saved.file_name.ends_with(".mp4"));
    assert_eq!(saved.bytes, 11);
    assert_eq!(saved.duration_ms, 2_500);
    let resolved = services.paths().expect("paths");
    assert!(saved.path.starts_with(&resolved.captures_dir));
    let id = saved.capture_id.expect("id");
    let repo = HistoryRepo::open_read(&resolved.history_file).expect("read");
    let capture = repo.get(id).expect("get").expect("row");
    assert_eq!(capture.kind, CaptureKind::Video);
    assert_eq!(capture.duration_ms, Some(2_500));
    assert_eq!(capture.width, Some(640));
    assert_eq!(capture.height, Some(360));
    assert!(resolved.thumbs_dir.join(format!("{id}.webp")).is_file());
    assert_eq!(
        services
            .settings_snapshot()
            .expect("settings")
            .general
            .extra
            .get("counter")
            .and_then(toml::Value::as_integer),
        Some(1)
    );
}

#[test]
fn a_gif_is_recorded_with_its_own_kind_and_extension() {
    let directory = tempdir().expect("temp");
    let services = store_services(directory.path());
    let media = directory.path().join("source.gif");
    std::fs::write(&media, b"gif-bytes").expect("media");
    let saved = store_recording(
        &services,
        RecordedFile {
            media,
            first_frame: None,
            kind: RecordedKind::Gif,
            width: 480,
            height: 270,
            duration_ms: 1_000,
            source_app: None,
            window_title: "Recording".to_owned(),
        },
    )
    .expect("save");
    assert!(saved.file_name.ends_with(".gif"));
    let repo =
        HistoryRepo::open_read(&services.paths().expect("paths").history_file).expect("read");
    let capture = repo
        .get(saved.capture_id.expect("id"))
        .expect("get")
        .expect("row");
    assert_eq!(capture.kind, CaptureKind::Gif);
}

#[test]
fn status_dto_fills_dimensions_and_terminal_details() {
    let status = RecordStatus {
        state: RecordingState::Done,
        kind: Some(RecordedKind::Gif),
        elapsed_ms: 4_200,
        width: 800,
        height: 450,
        duration_ms: 4_200,
        capture_id: Some(9),
        path: Some(PathBuf::from(r"C:\Videos\clip.gif")),
        error: None,
    };
    let dto = status_dto(&status);
    assert_eq!(dto.state, RecordStateDto::Done);
    assert_eq!(dto.kind, Some(RecordKindDto::Gif));
    assert_eq!(dto.width, Some(800));
    assert_eq!(dto.height, Some(450));
    assert_eq!(dto.duration_ms, Some(4_200));
    assert_eq!(dto.capture_id, Some(9));
    assert_eq!(dto.path.as_deref(), Some(r"C:\Videos\clip.gif"));
    let idle = status_dto(&RecordStatus::default());
    assert_eq!(idle.state, RecordStateDto::Idle);
    assert_eq!(idle.width, None);
    assert_eq!(idle.duration_ms, None);
}

#[test]
fn recording_actions_report_not_yet_without_ports() {
    let directory = tempdir().expect("temp");
    let services = Services::with_paths(paths(directory.path())).expect("services");
    let error = run(&ActionId::new("record_video"), &services).expect_err("not yet");
    assert_eq!(error, ServiceError::NotYet);
    assert_eq!(
        services.record().handle(RecordReq::State).expect("state"),
        RecordResp::Status {
            status: status_dto(&current_status())
        }
    );
    assert_eq!(
        services.record().handle(RecordReq::Stop).expect("stop"),
        RecordResp::Status {
            status: status_dto(&current_status())
        }
    );
}

#[test]
fn a_cancelled_error_maps_to_an_invalid_service_error() {
    let error: ServiceError = RecordError::Cancelled.into();
    assert_eq!(error.code(), "invalid");
    assert!(matches!(
        ServiceError::from(RecordError::NotYet),
        ServiceError::NotYet
    ));
    assert_eq!(ServiceError::from(RecordError::Busy).code(), "busy");
}
