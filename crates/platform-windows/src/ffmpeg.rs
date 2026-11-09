use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::io::{self, BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver, Sender};
use snapline_core::ffmpeg_args::{
    encoder_candidates, encoder_detection_command, fallback_encoder, hardware_candidates,
    test_encode_command, FfmpegArgsError, FfmpegCommand, VideoEncoder, VideoFormat,
};
use snapline_core::recording::STOP_TIMEOUT;
use thiserror::Error;
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

const STOP_POLL: Duration = Duration::from_millis(10);
const STDERR_LINES: usize = 64;
const STDERR_LINE_LIMIT: usize = 512;
const PROGRESS_FLAG: &str = "-progress";
const PROGRESS_TARGET: &str = "pipe:1";

#[derive(Debug, Error)]
pub enum FfmpegError {
    #[error("The FFmpeg path must be absolute.")]
    RelativePath,
    #[error("The FFmpeg path must point to ffmpeg.exe.")]
    NotFfmpeg,
    #[error("FFmpeg was not found.")]
    Missing,
    #[error("FFmpeg could not be started.")]
    Spawn(#[source] io::Error),
    #[error("FFmpeg could not be monitored.")]
    Monitor(#[source] io::Error),
    #[error("FFmpeg could not be stopped.")]
    Stop(#[source] io::Error),
    #[error("FFmpeg is not running.")]
    NotRunning,
    #[error("The FFmpeg command is not valid.")]
    Command(#[source] FfmpegArgsError),
    #[error("FFmpeg could not list its encoders.")]
    EncoderList,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Progress {
    pub out_time_us: Option<u64>,
    pub out_time_ms: Option<u64>,
    pub frame: Option<u64>,
    pub fps: Option<f64>,
    pub speed: Option<f64>,
    pub done: bool,
}

impl Progress {
    #[must_use]
    pub fn elapsed(&self) -> Option<Duration> {
        self.out_time_us
            .or(self.out_time_ms)
            .map(Duration::from_micros)
    }
}

#[derive(Debug, Clone)]
pub struct StopOutcome {
    status: ExitStatus,
    timed_out: bool,
    stderr: String,
}

impl StopOutcome {
    #[must_use]
    pub fn status(&self) -> &ExitStatus {
        &self.status
    }

    #[must_use]
    pub fn success(&self) -> bool {
        self.status.success()
    }

    #[must_use]
    pub fn code(&self) -> Option<i32> {
        self.status.code()
    }

    #[must_use]
    pub fn timed_out(&self) -> bool {
        self.timed_out
    }

    #[must_use]
    pub fn stderr(&self) -> &str {
        &self.stderr
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderSupport {
    exe: PathBuf,
    listed: Vec<VideoEncoder>,
    verified: Vec<VideoEncoder>,
}

impl EncoderSupport {
    fn new(exe: PathBuf, listed: Vec<VideoEncoder>, verified: Vec<VideoEncoder>) -> Self {
        Self {
            exe,
            listed,
            verified,
        }
    }

    #[must_use]
    pub fn exe(&self) -> &Path {
        &self.exe
    }

    #[must_use]
    pub fn listed(&self) -> &[VideoEncoder] {
        &self.listed
    }

    #[must_use]
    pub fn verified(&self) -> &[VideoEncoder] {
        &self.verified
    }

    #[must_use]
    pub fn is_listed(&self, encoder: VideoEncoder) -> bool {
        self.listed.contains(&encoder)
    }

    #[must_use]
    pub fn is_verified(&self, encoder: VideoEncoder) -> bool {
        self.verified.contains(&encoder)
    }

    #[must_use]
    pub fn hardware(&self, format: VideoFormat) -> Option<VideoEncoder> {
        hardware_candidates(format)
            .iter()
            .copied()
            .find(|encoder| self.is_verified(*encoder))
    }

    #[must_use]
    pub fn software(&self, format: VideoFormat) -> Option<VideoEncoder> {
        encoder_candidates(format)
            .iter()
            .copied()
            .find(|encoder| !encoder.is_hardware() && self.is_verified(*encoder))
    }

    #[must_use]
    pub fn fallback(&self, format: VideoFormat) -> Option<VideoEncoder> {
        let encoder = fallback_encoder(format);
        self.is_verified(encoder).then_some(encoder)
    }

    #[must_use]
    pub fn best(&self, format: VideoFormat) -> Option<VideoEncoder> {
        encoder_candidates(format)
            .iter()
            .copied()
            .find(|encoder| self.is_verified(*encoder))
    }

    #[must_use]
    pub fn has_hardware(&self, format: VideoFormat) -> bool {
        self.hardware(format).is_some()
    }

    #[must_use]
    pub fn has_fallback(&self, format: VideoFormat) -> bool {
        self.fallback(format).is_some()
    }
}

pub fn validate_exe(path: &Path) -> Result<PathBuf, FfmpegError> {
    if !path.is_absolute() {
        return Err(FfmpegError::RelativePath);
    }
    let named = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("ffmpeg.exe"));
    if !named {
        return Err(FfmpegError::NotFfmpeg);
    }
    if !path.is_file() {
        return Err(FfmpegError::Missing);
    }
    Ok(path.to_path_buf())
}

pub fn detect(exe: &Path) -> Result<EncoderSupport, FfmpegError> {
    let exe = validate_exe(exe)?;
    if let Some(cached) = cached(&exe) {
        return Ok(cached);
    }
    let listed = read_encoder_list(&exe)?;
    let mut verified = Vec::new();
    for encoder in probe_candidates() {
        if !listed.contains(&encoder) {
            continue;
        }
        if probe_encoder(&exe, encoder)? {
            verified.push(encoder);
        }
    }
    let support = EncoderSupport::new(exe.clone(), listed, verified);
    remember(&exe, &support);
    Ok(support)
}

fn probe_candidates() -> Vec<VideoEncoder> {
    let mut encoders = Vec::new();
    for format in [
        VideoFormat::Mp4H264,
        VideoFormat::Mp4H265,
        VideoFormat::WebmVp9,
    ] {
        for encoder in encoder_candidates(format) {
            if !encoders.contains(encoder) {
                encoders.push(*encoder);
            }
        }
    }
    encoders
}

fn read_encoder_list(exe: &Path) -> Result<Vec<VideoEncoder>, FfmpegError> {
    let command = encoder_detection_command(exe).map_err(FfmpegError::Command)?;
    let (program, args) = command.into_parts();
    let output = run_output(&program, &args)?;
    if !output.status.success() {
        return Err(FfmpegError::EncoderList);
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(parse_encoder_list(&text))
}

fn probe_encoder(exe: &Path, encoder: VideoEncoder) -> Result<bool, FfmpegError> {
    let command = test_encode_command(exe, encoder).map_err(FfmpegError::Command)?;
    let (program, args) = command.into_parts();
    let output = run_output(&program, &args)?;
    Ok(output.status.success())
}

fn parse_encoder_list(text: &str) -> Vec<VideoEncoder> {
    let mut encoders = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(flags) = parts.next() else {
            continue;
        };
        if !flags.starts_with('V') || flags.len() < 2 {
            continue;
        }
        let Some(name) = parts.next() else {
            continue;
        };
        let Some(encoder) = VideoEncoder::from_name(name) else {
            continue;
        };
        if !encoders.contains(&encoder) {
            encoders.push(encoder);
        }
    }
    encoders
}

fn cache() -> MutexGuard<'static, HashMap<String, EncoderSupport>> {
    static CACHE: OnceLock<Mutex<HashMap<String, EncoderSupport>>> = OnceLock::new();
    let mutex = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn cache_key(exe: &Path) -> String {
    exe.to_string_lossy().to_ascii_lowercase()
}

fn cached(exe: &Path) -> Option<EncoderSupport> {
    cache().get(&cache_key(exe)).cloned()
}

fn remember(exe: &Path, support: &EncoderSupport) {
    cache().insert(cache_key(exe), support.clone());
}

fn stderr_lock(stderr: &Mutex<StderrRing>) -> MutexGuard<'_, StderrRing> {
    match stderr.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct StderrRing {
    lines: VecDeque<String>,
    capacity: usize,
}

impl StderrRing {
    fn new(capacity: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            capacity,
        }
    }

    fn push(&mut self, line: String) {
        let line = if line.chars().count() > STDERR_LINE_LIMIT {
            line.chars().take(STDERR_LINE_LIMIT).collect()
        } else {
            line
        };
        if self.lines.len() == self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    fn summary(&self) -> String {
        self.lines.iter().cloned().collect::<Vec<_>>().join("\n")
    }
}

fn redact_paths(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut token = String::new();
    for character in line.chars() {
        if character.is_whitespace() {
            output.push_str(&redact_token(&token));
            output.push(character);
            token.clear();
        } else {
            token.push(character);
        }
    }
    output.push_str(&redact_token(&token));
    output
}

fn redact_token(token: &str) -> String {
    let stripped = token.trim_matches('"').trim_matches('\'');
    if looks_like_path(stripped) {
        token.replace(stripped, "<path>")
    } else {
        token.to_owned()
    }
}

fn looks_like_path(token: &str) -> bool {
    let bytes = token.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    drive || token.starts_with(r"\\")
}

fn apply_progress_line(line: &str, pending: &mut Progress) -> Option<Progress> {
    let (key, value) = line.trim().split_once('=')?;
    let value = value.trim();
    match key {
        "out_time_us" => {
            if let Ok(parsed) = value.parse::<u64>() {
                pending.out_time_us = Some(parsed);
            }
        }
        "out_time_ms" => {
            if let Ok(parsed) = value.parse::<u64>() {
                pending.out_time_ms = Some(parsed);
            }
        }
        "frame" => {
            if let Ok(parsed) = value.parse::<u64>() {
                pending.frame = Some(parsed);
            }
        }
        "fps" => {
            if let Ok(parsed) = value.parse::<f64>() {
                pending.fps = Some(parsed);
            }
        }
        "speed" => {
            if let Ok(parsed) = value.trim_end_matches('x').parse::<f64>() {
                pending.speed = Some(parsed);
            }
        }
        "progress" => {
            pending.done = value == "end";
            let finished = *pending;
            *pending = Progress::default();
            return Some(finished);
        }
        _ => {}
    }
    None
}

fn read_progress(stdout: ChildStdout, sender: Sender<Progress>) {
    let mut pending = Progress::default();
    let mut open = true;
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else {
            break;
        };
        if let Some(progress) = apply_progress_line(&line, &mut pending) {
            if open && sender.send(progress).is_err() {
                open = false;
            }
        }
    }
    if open && pending != Progress::default() {
        let _ = sender.send(pending);
    }
}

fn read_stderr(stderr: ChildStderr, ring: Arc<Mutex<StderrRing>>) {
    for line in BufReader::new(stderr).lines() {
        let Ok(line) = line else {
            break;
        };
        stderr_lock(&ring).push(redact_paths(&line));
    }
}

fn build_command(program: &Path, args: &[OsString]) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW.0);
    command
}

fn run_output(program: &Path, args: &[OsString]) -> Result<std::process::Output, FfmpegError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW.0);
    command.output().map_err(FfmpegError::Spawn)
}

fn ensure_progress(args: Vec<OsString>) -> Vec<OsString> {
    if args
        .iter()
        .any(|argument| argument.to_str() == Some(PROGRESS_FLAG))
    {
        return args;
    }
    let mut output = Vec::with_capacity(args.len() + 2);
    output.push(OsString::from(PROGRESS_FLAG));
    output.push(OsString::from(PROGRESS_TARGET));
    output.extend(args);
    output
}

pub fn start(command: FfmpegCommand) -> Result<RunningFfmpeg, FfmpegError> {
    RunningFfmpeg::start(command)
}

pub struct RunningFfmpeg {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    progress: Receiver<Progress>,
    progress_thread: Option<JoinHandle<()>>,
    stderr: Arc<Mutex<StderrRing>>,
    stderr_thread: Option<JoinHandle<()>>,
    outcome: Option<StopOutcome>,
}

impl RunningFfmpeg {
    pub fn start(command: FfmpegCommand) -> Result<Self, FfmpegError> {
        let (program, args) = command.into_parts();
        let program = validate_exe(&program)?;
        let args = ensure_progress(args);
        let mut child = build_command(&program, &args)
            .spawn()
            .map_err(FfmpegError::Spawn)?;
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(FfmpegError::Monitor(io::Error::other(
                "FFmpeg did not provide its streams.",
            )));
        };
        let (sender, progress) = unbounded();
        let progress_thread = match thread::Builder::new()
            .name("snapline-ffmpeg-progress".to_owned())
            .spawn(move || read_progress(stdout, sender))
        {
            Ok(handle) => handle,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(FfmpegError::Monitor(error));
            }
        };
        let ring = Arc::new(Mutex::new(StderrRing::new(STDERR_LINES)));
        let writer = Arc::clone(&ring);
        let stderr_thread = match thread::Builder::new()
            .name("snapline-ffmpeg-stderr".to_owned())
            .spawn(move || read_stderr(stderr, writer))
        {
            Ok(handle) => handle,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(FfmpegError::Monitor(error));
            }
        };
        Ok(Self {
            child: Some(child),
            stdin: Some(stdin),
            progress,
            progress_thread: Some(progress_thread),
            stderr: ring,
            stderr_thread: Some(stderr_thread),
            outcome: None,
        })
    }

    #[must_use]
    pub fn progress(&self) -> Receiver<Progress> {
        self.progress.clone()
    }

    #[must_use]
    pub fn is_running(&mut self) -> bool {
        match self.child.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    pub fn stop(&mut self) -> Result<StopOutcome, FfmpegError> {
        if let Some(outcome) = &self.outcome {
            return Ok(outcome.clone());
        }
        if self.child.is_none() {
            return Err(FfmpegError::NotRunning);
        }
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.write_all(b"q\n");
            let _ = stdin.flush();
        }
        let child = self.child.as_mut().ok_or(FfmpegError::NotRunning)?;
        let exited = await_exit(
            || child.try_wait().map(|status| status.is_some()),
            STOP_TIMEOUT,
            STOP_POLL,
        )
        .map_err(FfmpegError::Stop)?;
        if !exited {
            child.kill().map_err(FfmpegError::Stop)?;
        }
        let status = child.wait().map_err(FfmpegError::Stop)?;
        let outcome = self.finish(status, !exited);
        Ok(outcome)
    }

    pub fn kill(&mut self) -> Result<StopOutcome, FfmpegError> {
        if let Some(outcome) = &self.outcome {
            return Ok(outcome.clone());
        }
        self.stdin = None;
        let child = self.child.as_mut().ok_or(FfmpegError::NotRunning)?;
        if !matches!(child.try_wait(), Ok(Some(_))) {
            child.kill().map_err(FfmpegError::Stop)?;
        }
        let status = child.wait().map_err(FfmpegError::Stop)?;
        let outcome = self.finish(status, true);
        Ok(outcome)
    }

    pub fn wait(&mut self) -> Result<StopOutcome, FfmpegError> {
        if let Some(outcome) = &self.outcome {
            return Ok(outcome.clone());
        }
        self.stdin = None;
        let child = self.child.as_mut().ok_or(FfmpegError::NotRunning)?;
        let status = child.wait().map_err(FfmpegError::Stop)?;
        let outcome = self.finish(status, false);
        Ok(outcome)
    }

    fn finish(&mut self, status: ExitStatus, timed_out: bool) -> StopOutcome {
        self.child = None;
        self.stdin = None;
        self.join_threads();
        let outcome = StopOutcome {
            status,
            timed_out,
            stderr: self.stderr_summary(),
        };
        self.outcome = Some(outcome.clone());
        outcome
    }

    fn stderr_summary(&self) -> String {
        stderr_lock(&self.stderr).summary()
    }

    fn join_threads(&mut self) {
        if let Some(handle) = self.progress_thread.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.stderr_thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for RunningFfmpeg {
    fn drop(&mut self) {
        self.stdin = None;
        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(None) => {
                    let _ = child.kill();
                }
                Ok(Some(_)) => {}
                Err(_) => {
                    let _ = child.kill();
                }
            }
            let _ = child.wait();
        }
        self.child = None;
        self.join_threads();
    }
}

fn await_exit(
    mut exited: impl FnMut() -> io::Result<bool>,
    timeout: Duration,
    poll: Duration,
) -> io::Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        if exited()? {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        if !poll.is_zero() {
            thread::sleep(poll);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    fn temp_dir() -> tempfile::TempDir {
        match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(error) => panic!("temp directory: {error}"),
        }
    }

    #[test]
    fn relative_missing_and_non_ffmpeg_paths_are_rejected() {
        assert!(matches!(
            validate_exe(Path::new("ffmpeg.exe")),
            Err(FfmpegError::RelativePath)
        ));
        assert!(matches!(
            validate_exe(Path::new(r"tools\ffmpeg.exe")),
            Err(FfmpegError::RelativePath)
        ));
        assert!(matches!(
            validate_exe(Path::new(r"C:\Tools\ffplay.exe")),
            Err(FfmpegError::NotFfmpeg)
        ));
        let dir = temp_dir();
        assert!(matches!(
            validate_exe(&dir.path().join("ffmpeg.exe")),
            Err(FfmpegError::Missing)
        ));
    }

    #[test]
    fn an_existing_absolute_ffmpeg_exe_is_accepted() {
        let dir = temp_dir();
        let exe = dir.path().join("ffmpeg.exe");
        if let Err(error) = std::fs::write(&exe, b"stub") {
            panic!("stub executable: {error}");
        }
        match validate_exe(&exe) {
            Ok(path) => assert_eq!(path, exe),
            Err(error) => panic!("valid path was rejected: {error}"),
        }
    }

    #[test]
    fn the_file_name_check_is_case_insensitive() {
        let dir = temp_dir();
        let exe = dir.path().join("FFMPEG.EXE");
        if let Err(error) = std::fs::write(&exe, b"stub") {
            panic!("stub executable: {error}");
        }
        assert!(validate_exe(&exe).is_ok());
    }

    #[test]
    fn starting_rejects_a_missing_or_wrongly_named_executable() {
        let dir = temp_dir();
        let missing = dir.path().join("ffmpeg.exe");
        let command = match test_encode_command(&missing, VideoEncoder::LibX264) {
            Ok(command) => command,
            Err(error) => panic!("probe command was rejected: {error}"),
        };
        assert!(matches!(
            RunningFfmpeg::start(command),
            Err(FfmpegError::Missing)
        ));
        let wrong = dir.path().join("ffprobe.exe");
        let command = match test_encode_command(&wrong, VideoEncoder::LibX264) {
            Ok(command) => command,
            Err(error) => panic!("probe command was rejected: {error}"),
        };
        assert!(matches!(
            RunningFfmpeg::start(command),
            Err(FfmpegError::NotFfmpeg)
        ));
        assert!(matches!(detect(&missing), Err(FfmpegError::Missing)));
    }

    #[test]
    fn encoder_lists_keep_only_known_video_encoders() {
        let fixture = "\
Encoders:
 V..... = Video
 A..... = Audio
 ------
 V....D a64multi             Multicolor charset for Commodore 64
 V....D h264_amf             AMD AMF H.264 Encoder (codec h264)
 V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)
 V....D h264_qsv             H.264 (Intel Quick Sync Video acceleration)
 V....D h264_vaapi           H.264 (VAAPI)
 V....D libx264              libx264 H.264 / AVC
 V....D libx265              libx265 H.265 / HEVC
 V....D libvpx-vp9           libvpx VP9
 A....D aac                  AAC
";
        assert_eq!(
            parse_encoder_list(fixture),
            vec![
                VideoEncoder::H264Amf,
                VideoEncoder::H264Nvenc,
                VideoEncoder::H264Qsv,
                VideoEncoder::LibX264,
                VideoEncoder::LibX265,
                VideoEncoder::LibVp9,
            ]
        );
    }

    #[test]
    fn duplicate_encoder_names_are_listed_once() {
        let fixture = "\
 V....D libx264              libx264 H.264 / AVC
 V....D libx264              libx264 H.264 / AVC
";
        assert_eq!(parse_encoder_list(fixture), vec![VideoEncoder::LibX264]);
    }

    #[test]
    fn progress_keys_accumulate_into_one_event_per_block() {
        let mut pending = Progress::default();
        assert_eq!(apply_progress_line("frame=12", &mut pending), None);
        assert_eq!(apply_progress_line("fps=30.0", &mut pending), None);
        assert_eq!(
            apply_progress_line("out_time_us=400000", &mut pending),
            None
        );
        assert_eq!(
            apply_progress_line("out_time_ms=400000", &mut pending),
            None
        );
        assert_eq!(apply_progress_line("speed=1.5x", &mut pending), None);
        let Some(progress) = apply_progress_line("progress=continue", &mut pending) else {
            panic!("progress block was not emitted");
        };
        assert_eq!(progress.frame, Some(12));
        assert_eq!(progress.fps, Some(30.0));
        assert_eq!(progress.out_time_us, Some(400_000));
        assert_eq!(progress.out_time_ms, Some(400_000));
        assert_eq!(progress.speed, Some(1.5));
        assert!(!progress.done);
        assert_eq!(progress.elapsed(), Some(Duration::from_millis(400)));
        assert_eq!(pending, Progress::default());
    }

    #[test]
    fn progress_end_marks_the_event_done() {
        let mut pending = Progress::default();
        let Some(progress) = apply_progress_line("progress=end", &mut pending) else {
            panic!("progress end was not emitted");
        };
        assert!(progress.done);
        assert_eq!(progress.frame, None);
    }

    #[test]
    fn unknown_and_unparsable_progress_lines_are_ignored() {
        let mut pending = Progress::default();
        assert_eq!(
            apply_progress_line("bitrate=1234.5kbits/s", &mut pending),
            None
        );
        assert_eq!(apply_progress_line("speed=N/A", &mut pending), None);
        assert_eq!(apply_progress_line("fps=abc", &mut pending), None);
        assert_eq!(
            apply_progress_line("not a progress line", &mut pending),
            None
        );
        assert_eq!(pending, Progress::default());
    }

    #[test]
    fn the_stop_wait_returns_false_once_the_timeout_expires() {
        let exited = match await_exit(|| Ok(false), Duration::ZERO, Duration::ZERO) {
            Ok(value) => value,
            Err(error) => panic!("wait failed: {error}"),
        };
        assert!(!exited);
    }

    #[test]
    fn the_stop_wait_polls_until_the_child_exits() {
        let mut polls = 0;
        let exited = match await_exit(
            || {
                polls += 1;
                Ok(polls >= 3)
            },
            Duration::from_millis(50),
            Duration::ZERO,
        ) {
            Ok(value) => value,
            Err(error) => panic!("wait failed: {error}"),
        };
        assert!(exited);
        assert_eq!(polls, 3);
    }

    #[test]
    fn arguments_stay_a_list_and_are_never_a_shell_string() {
        let args = vec![
            OsString::from("-i"),
            OsString::from(r"C:\Videos\one & two; three.mp4"),
        ];
        let command = build_command(Path::new(r"C:\Tools\ffmpeg.exe"), &args);
        assert_eq!(
            command.get_program(),
            std::ffi::OsStr::new(r"C:\Tools\ffmpeg.exe")
        );
        let actual: Vec<OsString> = command
            .get_args()
            .map(std::ffi::OsStr::to_os_string)
            .collect();
        assert_eq!(actual, args);
    }

    #[test]
    fn progress_is_added_once_and_only_when_missing() {
        let added = ensure_progress(vec![OsString::from("-y")]);
        assert_eq!(
            strings(&added),
            vec!["-progress".to_owned(), "pipe:1".to_owned(), "-y".to_owned()]
        );
        let existing = vec![
            OsString::from("-progress"),
            OsString::from("pipe:1"),
            OsString::from("-y"),
        ];
        assert_eq!(ensure_progress(existing.clone()), existing);
    }

    #[test]
    fn stderr_is_bounded_to_the_most_recent_lines() {
        let mut ring = StderrRing::new(2);
        ring.push("one".to_owned());
        ring.push("two".to_owned());
        ring.push("three".to_owned());
        assert_eq!(ring.summary(), "two\nthree");
    }

    #[test]
    fn stderr_lines_are_truncated() {
        let mut ring = StderrRing::new(1);
        ring.push("x".repeat(STDERR_LINE_LIMIT + 10));
        assert_eq!(ring.summary().chars().count(), STDERR_LINE_LIMIT);
    }

    #[test]
    fn paths_are_redacted_from_stderr() {
        assert_eq!(
            redact_paths(r"Error opening C:\Videos\a.mp4: Invalid argument"),
            "Error opening <path> Invalid argument"
        );
        assert_eq!(
            redact_paths(r"Could not open \\server\share\clip.mkv"),
            "Could not open <path>"
        );
        assert_eq!(
            redact_paths("libx264 reported an error"),
            "libx264 reported an error"
        );
    }

    #[test]
    fn encoder_support_reports_hardware_and_fallback_availability() {
        let support = EncoderSupport::new(
            PathBuf::from(r"C:\Tools\ffmpeg.exe"),
            vec![
                VideoEncoder::H264Nvenc,
                VideoEncoder::LibX264,
                VideoEncoder::LibX265,
            ],
            vec![VideoEncoder::H264Nvenc, VideoEncoder::LibX264],
        );
        assert_eq!(
            support.hardware(VideoFormat::Mp4H264),
            Some(VideoEncoder::H264Nvenc)
        );
        assert_eq!(
            support.software(VideoFormat::Mp4H264),
            Some(VideoEncoder::LibX264)
        );
        assert_eq!(
            support.fallback(VideoFormat::Mp4H264),
            Some(VideoEncoder::LibX264)
        );
        assert_eq!(
            support.best(VideoFormat::Mp4H264),
            Some(VideoEncoder::H264Nvenc)
        );
        assert!(support.has_hardware(VideoFormat::Mp4H264));
        assert!(support.has_fallback(VideoFormat::Mp4H264));
        assert_eq!(support.hardware(VideoFormat::WebmVp9), None);
        assert!(!support.has_fallback(VideoFormat::WebmVp9));
        assert!(!support.is_verified(VideoEncoder::LibX265));
        assert!(support.is_listed(VideoEncoder::LibX265));
    }

    fn bundled_ffmpeg() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("third_party")
            .join("ffmpeg")
            .join("ffmpeg.exe")
    }

    #[test]
    #[ignore = "desktop"]
    fn the_bundled_ffmpeg_detects_its_encoders() {
        let exe = bundled_ffmpeg();
        let support = match detect(&exe) {
            Ok(support) => support,
            Err(error) => panic!("detection failed: {error}"),
        };
        assert!(support.is_listed(VideoEncoder::LibX264));
        assert!(support.is_verified(VideoEncoder::LibX264));
        assert!(support.has_fallback(VideoFormat::Mp4H264));
    }

    #[test]
    #[ignore = "desktop"]
    fn the_bundled_ffmpeg_runs_and_reaps_a_one_frame_encode() {
        let exe = bundled_ffmpeg();
        let command = match test_encode_command(&exe, VideoEncoder::LibX264) {
            Ok(command) => command,
            Err(error) => panic!("probe command was rejected: {error}"),
        };
        let mut running = match RunningFfmpeg::start(command) {
            Ok(running) => running,
            Err(error) => panic!("probe did not start: {error}"),
        };
        let outcome = match running.wait() {
            Ok(outcome) => outcome,
            Err(error) => panic!("probe did not finish: {error}"),
        };
        assert!(outcome.success());
        assert!(!outcome.timed_out());
        let _ = running.progress();
    }
}
