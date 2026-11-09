use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::recording::RecordingTarget;

pub const AUDIO_PIPE_PREFIX: &str = r"\\.\pipe\snapline-audio-";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Codec {
    H264,
    H265,
    Vp9,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoFormat {
    Mp4H264,
    Mp4H265,
    WebmVp9,
    MkvH264,
}

impl VideoFormat {
    pub const fn codec(self) -> Codec {
        match self {
            Self::Mp4H264 | Self::MkvH264 => Codec::H264,
            Self::Mp4H265 => Codec::H265,
            Self::WebmVp9 => Codec::Vp9,
        }
    }

    pub const fn container(self) -> &'static str {
        match self {
            Self::Mp4H264 | Self::Mp4H265 => "mp4",
            Self::WebmVp9 => "webm",
            Self::MkvH264 => "matroska",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mp4H264 | Self::Mp4H265 => "mp4",
            Self::WebmVp9 => "webm",
            Self::MkvH264 => "mkv",
        }
    }
}

impl Default for VideoFormat {
    fn default() -> Self {
        Self::Mp4H264
    }
}

impl fmt::Display for VideoFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Mp4H264 => "MP4 H.264",
            Self::Mp4H265 => "MP4 H.265",
            Self::WebmVp9 => "WebM VP9",
            Self::MkvH264 => "MKV H.264",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoFrameRate {
    Fps24,
    Fps30,
    Fps60,
}

impl VideoFrameRate {
    pub const ALL: [Self; 3] = [Self::Fps24, Self::Fps30, Self::Fps60];

    pub const fn as_u32(self) -> u32 {
        match self {
            Self::Fps24 => 24,
            Self::Fps30 => 30,
            Self::Fps60 => 60,
        }
    }
}

impl Default for VideoFrameRate {
    fn default() -> Self {
        Self::Fps30
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoQuality {
    High,
    Balanced,
    SmallFile,
    Lossless,
}

impl VideoQuality {
    pub const fn crf(self) -> Option<u32> {
        match self {
            Self::High => Some(20),
            Self::Balanced => Some(26),
            Self::SmallFile => Some(32),
            Self::Lossless => None,
        }
    }
}

impl Default for VideoQuality {
    fn default() -> Self {
        Self::High
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoEncoder {
    H264Nvenc,
    H264Qsv,
    H264Amf,
    HevcNvenc,
    HevcQsv,
    HevcAmf,
    LibX264,
    LibX265,
    LibVp9,
}

impl VideoEncoder {
    pub const fn name(self) -> &'static str {
        match self {
            Self::H264Nvenc => "h264_nvenc",
            Self::H264Qsv => "h264_qsv",
            Self::H264Amf => "h264_amf",
            Self::HevcNvenc => "hevc_nvenc",
            Self::HevcQsv => "hevc_qsv",
            Self::HevcAmf => "hevc_amf",
            Self::LibX264 => "libx264",
            Self::LibX265 => "libx265",
            Self::LibVp9 => "libvpx-vp9",
        }
    }

    pub const fn codec(self) -> Codec {
        match self {
            Self::H264Nvenc | Self::H264Qsv | Self::H264Amf | Self::LibX264 => Codec::H264,
            Self::HevcNvenc | Self::HevcQsv | Self::HevcAmf | Self::LibX265 => Codec::H265,
            Self::LibVp9 => Codec::Vp9,
        }
    }

    pub const fn is_hardware(self) -> bool {
        matches!(
            self,
            Self::H264Nvenc
                | Self::H264Qsv
                | Self::H264Amf
                | Self::HevcNvenc
                | Self::HevcQsv
                | Self::HevcAmf
        )
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "h264_nvenc" => Some(Self::H264Nvenc),
            "h264_qsv" => Some(Self::H264Qsv),
            "h264_amf" => Some(Self::H264Amf),
            "hevc_nvenc" => Some(Self::HevcNvenc),
            "hevc_qsv" => Some(Self::HevcQsv),
            "hevc_amf" => Some(Self::HevcAmf),
            "libx264" => Some(Self::LibX264),
            "libx265" => Some(Self::LibX265),
            "libvpx-vp9" => Some(Self::LibVp9),
            _ => None,
        }
    }
}

impl fmt::Display for VideoEncoder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

static H264_ENCODERS: [VideoEncoder; 4] = [
    VideoEncoder::H264Nvenc,
    VideoEncoder::H264Qsv,
    VideoEncoder::H264Amf,
    VideoEncoder::LibX264,
];

static HEVC_ENCODERS: [VideoEncoder; 4] = [
    VideoEncoder::HevcNvenc,
    VideoEncoder::HevcQsv,
    VideoEncoder::HevcAmf,
    VideoEncoder::LibX265,
];

static VP9_ENCODERS: [VideoEncoder; 1] = [VideoEncoder::LibVp9];

static H264_HARDWARE: [VideoEncoder; 3] = [
    VideoEncoder::H264Nvenc,
    VideoEncoder::H264Qsv,
    VideoEncoder::H264Amf,
];

static HEVC_HARDWARE: [VideoEncoder; 3] = [
    VideoEncoder::HevcNvenc,
    VideoEncoder::HevcQsv,
    VideoEncoder::HevcAmf,
];

pub fn encoder_candidates(format: VideoFormat) -> &'static [VideoEncoder] {
    match format {
        VideoFormat::Mp4H264 | VideoFormat::MkvH264 => &H264_ENCODERS,
        VideoFormat::Mp4H265 => &HEVC_ENCODERS,
        VideoFormat::WebmVp9 => &VP9_ENCODERS,
    }
}

pub fn hardware_candidates(format: VideoFormat) -> &'static [VideoEncoder] {
    match format {
        VideoFormat::Mp4H264 | VideoFormat::MkvH264 => &H264_HARDWARE,
        VideoFormat::Mp4H265 => &HEVC_HARDWARE,
        VideoFormat::WebmVp9 => &[],
    }
}

pub const fn fallback_encoder(format: VideoFormat) -> VideoEncoder {
    match format {
        VideoFormat::Mp4H264 | VideoFormat::MkvH264 => VideoEncoder::LibX264,
        VideoFormat::Mp4H265 => VideoEncoder::LibX265,
        VideoFormat::WebmVp9 => VideoEncoder::LibVp9,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GifFrameRate {
    Fps10,
    Fps15,
    Fps20,
    Fps30,
}

impl GifFrameRate {
    pub const ALL: [Self; 4] = [Self::Fps10, Self::Fps15, Self::Fps20, Self::Fps30];

    pub const fn as_u32(self) -> u32 {
        match self {
            Self::Fps10 => 10,
            Self::Fps15 => 15,
            Self::Fps20 => 20,
            Self::Fps30 => 30,
        }
    }
}

impl Default for GifFrameRate {
    fn default() -> Self {
        Self::Fps15
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GifMaxWidth {
    Original,
    Px1280,
    Px800,
    Px480,
}

impl GifMaxWidth {
    pub const ALL: [Self; 4] = [Self::Original, Self::Px1280, Self::Px800, Self::Px480];

    pub const fn pixels(self) -> Option<u32> {
        match self {
            Self::Original => None,
            Self::Px1280 => Some(1280),
            Self::Px800 => Some(800),
            Self::Px480 => Some(480),
        }
    }
}

impl Default for GifMaxWidth {
    fn default() -> Self {
        Self::Px800
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GifSettings {
    pub frame_rate: GifFrameRate,
    pub max_width: GifMaxWidth,
    pub optimise_palette: bool,
}

impl Default for GifSettings {
    fn default() -> Self {
        Self {
            frame_rate: GifFrameRate::default(),
            max_width: GifMaxWidth::default(),
            optimise_palette: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioSource {
    System,
    Microphone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PcmSampleFormat {
    S16Le,
    F32Le,
    S32Le,
}

impl PcmSampleFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::S16Le => "s16le",
            Self::F32Le => "f32le",
            Self::S32Le => "s32le",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioPipe {
    pub source: AudioSource,
    pub index: u32,
    pub sample_format: PcmSampleFormat,
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioPipe {
    pub fn path(&self) -> String {
        audio_pipe_path(self.index)
    }
}

pub fn audio_pipe_path(index: u32) -> String {
    format!("{AUDIO_PIPE_PREFIX}{index}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoRequest {
    pub ffmpeg: PathBuf,
    pub target: RecordingTarget,
    pub output: PathBuf,
    pub output_index: u32,
    pub format: VideoFormat,
    pub encoder: VideoEncoder,
    pub frame_rate: VideoFrameRate,
    pub quality: VideoQuality,
    pub audio: Vec<AudioPipe>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GifCaptureRequest {
    pub ffmpeg: PathBuf,
    pub target: RecordingTarget,
    pub output: PathBuf,
    pub output_index: u32,
    pub frame_rate: GifFrameRate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GifConvertRequest {
    pub ffmpeg: PathBuf,
    pub input: PathBuf,
    pub output: PathBuf,
    pub settings: GifSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertFormat {
    Gif(GifSettings),
    WebmVp9,
    Mp4Small,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertRequest {
    pub ffmpeg: PathBuf,
    pub input: PathBuf,
    pub output: PathBuf,
    pub format: ConvertFormat,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FfmpegCommand {
    program: PathBuf,
    args: Vec<OsString>,
}

impl FfmpegCommand {
    pub fn program(&self) -> &Path {
        &self.program
    }

    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    pub fn into_parts(self) -> (PathBuf, Vec<OsString>) {
        (self.program, self.args)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FfmpegArgsError {
    #[error("the ffmpeg path must be absolute")]
    FfmpegPathNotAbsolute,
    #[error("the recording target is empty")]
    EmptyTarget,
    #[error("the recording target offset must not be negative")]
    NegativeTargetOffset,
    #[error("{encoder} cannot encode {format}")]
    EncoderFormatMismatch {
        encoder: VideoEncoder,
        format: VideoFormat,
    },
    #[error("the audio sample rate and channel count must be greater than zero")]
    InvalidAudioFormat,
}

fn os(value: &str) -> OsString {
    OsString::from(value)
}

fn build(ffmpeg: &Path, args: Vec<OsString>) -> Result<FfmpegCommand, FfmpegArgsError> {
    if !ffmpeg.is_absolute() {
        return Err(FfmpegArgsError::FfmpegPathNotAbsolute);
    }
    Ok(FfmpegCommand {
        program: ffmpeg.to_path_buf(),
        args,
    })
}

fn ddagrab_source(
    target: RecordingTarget,
    output_index: u32,
    frame_rate: u32,
) -> Result<String, FfmpegArgsError> {
    let bounds = target.bounds();
    if bounds.is_empty() {
        return Err(FfmpegArgsError::EmptyTarget);
    }
    let x = bounds.x.get();
    let y = bounds.y.get();
    if x < 0 || y < 0 {
        return Err(FfmpegArgsError::NegativeTargetOffset);
    }
    Ok(format!(
        "ddagrab=framerate={frame_rate}:output_idx={output_index}:video_size={}x{}:offset_x={x}:offset_y={y}",
        bounds.width.get(),
        bounds.height.get()
    ))
}

fn quality_args(encoder: VideoEncoder, quality: VideoQuality) -> Vec<String> {
    let value = quality.crf();
    let args: Vec<String> = match encoder {
        VideoEncoder::LibX264 | VideoEncoder::LibX265 => {
            vec!["-crf".to_owned(), value.unwrap_or(0).to_string()]
        }
        VideoEncoder::LibVp9 => match value {
            Some(value) => vec![
                "-crf".to_owned(),
                value.to_string(),
                "-b:v".to_owned(),
                "0".to_owned(),
            ],
            None => vec!["-lossless".to_owned(), "1".to_owned()],
        },
        VideoEncoder::H264Nvenc | VideoEncoder::HevcNvenc => match value {
            Some(value) => vec![
                "-rc".to_owned(),
                "vbr".to_owned(),
                "-cq".to_owned(),
                value.to_string(),
            ],
            None => vec!["-tune".to_owned(), "lossless".to_owned()],
        },
        VideoEncoder::H264Qsv | VideoEncoder::HevcQsv => match value {
            Some(value) => vec!["-global_quality".to_owned(), value.to_string()],
            None => vec!["-global_quality".to_owned(), "1".to_owned()],
        },
        VideoEncoder::H264Amf | VideoEncoder::HevcAmf => {
            let value = value.unwrap_or(0).to_string();
            vec![
                "-rc".to_owned(),
                "cqp".to_owned(),
                "-qp_i".to_owned(),
                value.clone(),
                "-qp_p".to_owned(),
                value.clone(),
                "-qp_b".to_owned(),
                value,
            ]
        }
    };
    args
}

fn append_audio_inputs(
    args: &mut Vec<OsString>,
    audio: &[AudioPipe],
) -> Result<(), FfmpegArgsError> {
    for pipe in audio {
        if pipe.sample_rate == 0 || pipe.channels == 0 {
            return Err(FfmpegArgsError::InvalidAudioFormat);
        }
        args.push(os("-f"));
        args.push(os(pipe.sample_format.as_str()));
        args.push(os("-ar"));
        args.push(OsString::from(pipe.sample_rate.to_string()));
        args.push(os("-ac"));
        args.push(OsString::from(pipe.channels.to_string()));
        args.push(os("-i"));
        args.push(OsString::from(pipe.path()));
    }
    Ok(())
}

fn audio_filter(audio: &[AudioPipe]) -> Option<(String, &'static str)> {
    if audio.len() < 2 {
        return None;
    }
    let inputs: String = (1..=audio.len())
        .map(|index| format!("[{index}:a]"))
        .collect();
    Some((format!("{inputs}amix=inputs={}[aout]", audio.len()), "aout"))
}

pub fn video_command(request: &VideoRequest) -> Result<FfmpegCommand, FfmpegArgsError> {
    if request.encoder.codec() != request.format.codec() {
        return Err(FfmpegArgsError::EncoderFormatMismatch {
            encoder: request.encoder,
            format: request.format,
        });
    }

    let source = ddagrab_source(
        request.target,
        request.output_index,
        request.frame_rate.as_u32(),
    )?;

    let mut args = vec![
        os("-y"),
        os("-f"),
        os("lavfi"),
        os("-i"),
        OsString::from(source),
    ];
    append_audio_inputs(&mut args, &request.audio)?;
    args.push(os("-vf"));
    args.push(os("hwdownload,format=bgra"));

    if let Some((filter, _)) = audio_filter(&request.audio) {
        args.push(os("-filter_complex"));
        args.push(OsString::from(filter));
    }

    args.push(os("-map"));
    args.push(os("0:v:0"));

    match request.audio.len() {
        0 => {}
        1 => {
            args.push(os("-map"));
            args.push(os("1:a:0"));
        }
        _ => {
            args.push(os("-map"));
            args.push(os("[aout]"));
        }
    }

    args.push(os("-c:v"));
    args.push(os(request.encoder.name()));
    for value in quality_args(request.encoder, request.quality) {
        args.push(OsString::from(value));
    }
    args.push(os("-pix_fmt"));
    args.push(os("yuv420p"));
    args.push(os("-f"));
    args.push(os(request.format.container()));
    args.push(request.output.as_os_str().to_os_string());

    build(&request.ffmpeg, args)
}

pub fn gif_capture_command(request: &GifCaptureRequest) -> Result<FfmpegCommand, FfmpegArgsError> {
    let source = ddagrab_source(
        request.target,
        request.output_index,
        request.frame_rate.as_u32(),
    )?;
    let args = vec![
        os("-y"),
        os("-f"),
        os("lavfi"),
        os("-i"),
        OsString::from(source),
        os("-vf"),
        os("hwdownload,format=bgra"),
        os("-c:v"),
        os("libx264"),
        os("-crf"),
        os("0"),
        os("-pix_fmt"),
        os("yuv444p"),
        os("-f"),
        os("mp4"),
        request.output.as_os_str().to_os_string(),
    ];
    build(&request.ffmpeg, args)
}

fn gif_filter(settings: GifSettings) -> String {
    let fps = settings.frame_rate.as_u32();
    let scale = match settings.max_width.pixels() {
        Some(width) => format!("scale=w=min({width}\\,iw):h=-2:flags=lanczos,"),
        None => String::new(),
    };
    let palette = if settings.optimise_palette {
        "palettegen=stats_mode=diff"
    } else {
        "palettegen"
    };
    format!("[0:v]fps={fps},{scale}split[a][b];[a]{palette}[p];[b][p]paletteuse")
}

pub fn gif_convert_command(request: &GifConvertRequest) -> Result<FfmpegCommand, FfmpegArgsError> {
    let filter = gif_filter(request.settings);
    let args = vec![
        os("-y"),
        os("-i"),
        request.input.as_os_str().to_os_string(),
        os("-filter_complex"),
        OsString::from(filter),
        os("-an"),
        os("-loop"),
        os("0"),
        os("-f"),
        os("gif"),
        request.output.as_os_str().to_os_string(),
    ];
    build(&request.ffmpeg, args)
}

pub fn convert_command(request: &ConvertRequest) -> Result<FfmpegCommand, FfmpegArgsError> {
    match request.format {
        ConvertFormat::Gif(settings) => gif_convert_command(&GifConvertRequest {
            ffmpeg: request.ffmpeg.clone(),
            input: request.input.clone(),
            output: request.output.clone(),
            settings,
        }),
        ConvertFormat::WebmVp9 => {
            let args = vec![
                os("-y"),
                os("-i"),
                request.input.as_os_str().to_os_string(),
                os("-c:v"),
                os("libvpx-vp9"),
                os("-f"),
                os("webm"),
                request.output.as_os_str().to_os_string(),
            ];
            build(&request.ffmpeg, args)
        }
        ConvertFormat::Mp4Small => {
            let args = vec![
                os("-y"),
                os("-i"),
                request.input.as_os_str().to_os_string(),
                os("-c:v"),
                os("libx264"),
                os("-crf"),
                os("28"),
                os("-f"),
                os("mp4"),
                request.output.as_os_str().to_os_string(),
            ];
            build(&request.ffmpeg, args)
        }
    }
}

pub fn encoder_detection_command(ffmpeg: &Path) -> Result<FfmpegCommand, FfmpegArgsError> {
    build(ffmpeg, vec![os("-hide_banner"), os("-encoders")])
}

pub fn test_encode_command(
    ffmpeg: &Path,
    encoder: VideoEncoder,
) -> Result<FfmpegCommand, FfmpegArgsError> {
    let args = vec![
        os("-hide_banner"),
        os("-f"),
        os("lavfi"),
        os("-i"),
        os("color=c=black:s=256x256:r=1"),
        os("-frames:v"),
        os("1"),
        os("-c:v"),
        os(encoder.name()),
        os("-pix_fmt"),
        os("yuv420p"),
        os("-f"),
        os("null"),
        os("-"),
    ];
    build(ffmpeg, args)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::geometry::{Px, Rect};
    use crate::recording::RecordingTarget;

    fn ffmpeg() -> PathBuf {
        PathBuf::from(r"C:\Tools\ffmpeg.exe")
    }

    fn region() -> Rect {
        Rect::new(Px(100), Px(200), Px(640), Px(480))
    }

    fn target() -> RecordingTarget {
        RecordingTarget::region(region())
    }

    fn output() -> PathBuf {
        PathBuf::from(r"C:\Videos\capture.mp4")
    }

    fn video_request() -> VideoRequest {
        VideoRequest {
            ffmpeg: ffmpeg(),
            target: target(),
            output: output(),
            output_index: 0,
            format: VideoFormat::Mp4H264,
            encoder: VideoEncoder::H264Nvenc,
            frame_rate: VideoFrameRate::Fps30,
            quality: VideoQuality::High,
            audio: Vec::new(),
        }
    }

    fn gif_capture_request() -> GifCaptureRequest {
        GifCaptureRequest {
            ffmpeg: ffmpeg(),
            target: target(),
            output: PathBuf::from(r"C:\Temp\snapline-gif.mp4"),
            output_index: 0,
            frame_rate: GifFrameRate::Fps15,
        }
    }

    fn gif_convert_request(settings: GifSettings) -> GifConvertRequest {
        GifConvertRequest {
            ffmpeg: ffmpeg(),
            input: PathBuf::from(r"C:\Temp\snapline-gif.mp4"),
            output: PathBuf::from(r"C:\Videos\capture.gif"),
            settings,
        }
    }

    fn command_video(request: &VideoRequest) -> FfmpegCommand {
        match video_command(request) {
            Ok(command) => command,
            Err(error) => panic!("video command was rejected: {error}"),
        }
    }

    fn strings(command: &FfmpegCommand) -> Vec<String> {
        command
            .args()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter()
            .position(|argument| argument == flag)
            .and_then(|index| args.get(index + 1))
            .map(String::as_str)
    }

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2)
            .any(|pair| pair[0] == flag && pair[1] == value)
    }

    fn audio(source: AudioSource, index: u32) -> AudioPipe {
        AudioPipe {
            source,
            index,
            sample_format: PcmSampleFormat::S16Le,
            sample_rate: 48_000,
            channels: 2,
        }
    }

    #[test]
    fn every_video_format_encoder_frame_rate_and_quality_builds() {
        for format in [
            VideoFormat::Mp4H264,
            VideoFormat::Mp4H265,
            VideoFormat::WebmVp9,
            VideoFormat::MkvH264,
        ] {
            for encoder in encoder_candidates(format) {
                for frame_rate in VideoFrameRate::ALL {
                    for quality in [
                        VideoQuality::High,
                        VideoQuality::Balanced,
                        VideoQuality::SmallFile,
                        VideoQuality::Lossless,
                    ] {
                        let request = VideoRequest {
                            format,
                            encoder: *encoder,
                            frame_rate,
                            quality,
                            ..video_request()
                        };
                        let command = command_video(&request);
                        let args = strings(&command);
                        assert_eq!(command.program(), Path::new(r"C:\Tools\ffmpeg.exe"));
                        assert_eq!(
                            value_after(&args, "-c:v"),
                            Some(encoder.name()),
                            "{format} {encoder}"
                        );
                        assert!(has_pair(&args, "-f", format.container()));
                        assert_eq!(
                            args.last().map(String::as_str),
                            Some(r"C:\Videos\capture.mp4")
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn video_container_extensions_match_the_settings_options() {
        assert_eq!(VideoFormat::Mp4H264.container(), "mp4");
        assert_eq!(VideoFormat::Mp4H264.extension(), "mp4");
        assert_eq!(VideoFormat::Mp4H265.container(), "mp4");
        assert_eq!(VideoFormat::Mp4H265.extension(), "mp4");
        assert_eq!(VideoFormat::WebmVp9.container(), "webm");
        assert_eq!(VideoFormat::WebmVp9.extension(), "webm");
        assert_eq!(VideoFormat::MkvH264.container(), "matroska");
        assert_eq!(VideoFormat::MkvH264.extension(), "mkv");
    }

    #[test]
    fn video_frame_rates_are_the_documented_choices() {
        assert_eq!(
            VideoFrameRate::ALL.map(VideoFrameRate::as_u32),
            [24, 30, 60]
        );
        assert_eq!(VideoFrameRate::default(), VideoFrameRate::Fps30);
        assert_eq!(VideoQuality::default(), VideoQuality::High);
        assert_eq!(VideoFormat::default(), VideoFormat::Mp4H264);
    }

    #[test]
    fn quality_presets_map_to_cq_and_crf_values() {
        assert_eq!(VideoQuality::High.crf(), Some(20));
        assert_eq!(VideoQuality::Balanced.crf(), Some(26));
        assert_eq!(VideoQuality::SmallFile.crf(), Some(32));
        assert_eq!(VideoQuality::Lossless.crf(), None);
    }

    #[test]
    fn software_encoders_use_crf_or_lossless_mode() {
        for (encoder, format, value, lossless_flag) in [
            (VideoEncoder::LibX264, VideoFormat::Mp4H264, "20", "-crf"),
            (VideoEncoder::LibX265, VideoFormat::Mp4H265, "20", "-crf"),
        ] {
            let request = VideoRequest {
                format,
                encoder,
                quality: VideoQuality::High,
                ..video_request()
            };
            let args = strings(&command_video(&request));
            assert!(has_pair(&args, "-crf", value), "{encoder}");
            assert!(lossless_flag == "-crf");

            let request = VideoRequest {
                format,
                encoder,
                quality: VideoQuality::Lossless,
                ..video_request()
            };
            let args = strings(&command_video(&request));
            assert!(has_pair(&args, "-crf", "0"), "{encoder}");
        }
    }

    #[test]
    fn vp9_uses_crf_with_zero_bitrate_or_its_lossless_switch() {
        let request = VideoRequest {
            format: VideoFormat::WebmVp9,
            encoder: VideoEncoder::LibVp9,
            quality: VideoQuality::SmallFile,
            ..video_request()
        };
        let args = strings(&command_video(&request));
        assert!(has_pair(&args, "-crf", "32"));
        assert!(has_pair(&args, "-b:v", "0"));

        let request = VideoRequest {
            quality: VideoQuality::Lossless,
            ..request
        };
        let args = strings(&command_video(&request));
        assert!(has_pair(&args, "-lossless", "1"));
        assert!(!args.iter().any(|argument| argument == "-crf"));
    }

    #[test]
    fn nvenc_uses_cq_or_tune_lossless() {
        let request = VideoRequest {
            encoder: VideoEncoder::H264Nvenc,
            quality: VideoQuality::High,
            ..video_request()
        };
        let args = strings(&command_video(&request));
        assert!(has_pair(&args, "-rc", "vbr"));
        assert!(has_pair(&args, "-cq", "20"));

        let request = VideoRequest {
            quality: VideoQuality::Lossless,
            ..request
        };
        let args = strings(&command_video(&request));
        assert!(has_pair(&args, "-tune", "lossless"));
        assert!(!args.iter().any(|argument| argument == "-cq"));
    }

    #[test]
    fn qsv_uses_global_quality() {
        let request = VideoRequest {
            encoder: VideoEncoder::H264Qsv,
            quality: VideoQuality::Balanced,
            ..video_request()
        };
        let args = strings(&command_video(&request));
        assert!(has_pair(&args, "-global_quality", "26"));

        let request = VideoRequest {
            quality: VideoQuality::Lossless,
            ..request
        };
        let args = strings(&command_video(&request));
        assert!(has_pair(&args, "-global_quality", "1"));
    }

    #[test]
    fn amf_uses_constant_quantiser_parameters() {
        let request = VideoRequest {
            encoder: VideoEncoder::H264Amf,
            quality: VideoQuality::SmallFile,
            ..video_request()
        };
        let args = strings(&command_video(&request));
        assert!(has_pair(&args, "-rc", "cqp"));
        assert!(has_pair(&args, "-qp_i", "32"));
        assert!(has_pair(&args, "-qp_p", "32"));
        assert!(has_pair(&args, "-qp_b", "32"));

        let request = VideoRequest {
            quality: VideoQuality::Lossless,
            ..request
        };
        let args = strings(&command_video(&request));
        assert!(has_pair(&args, "-qp_i", "0"));
        assert!(has_pair(&args, "-qp_p", "0"));
        assert!(has_pair(&args, "-qp_b", "0"));
    }

    #[test]
    fn video_command_is_a_complete_argument_vector() {
        let command = command_video(&video_request());
        let args = strings(&command);
        assert_eq!(args[0], "-y");
        assert!(has_pair(&args, "-f", "lavfi"));
        assert_eq!(
            value_after(&args, "-i"),
            Some("ddagrab=framerate=30:output_idx=0:video_size=640x480:offset_x=100:offset_y=200")
        );
        assert!(has_pair(&args, "-vf", "hwdownload,format=bgra"));
        assert!(has_pair(&args, "-map", "0:v:0"));
        assert_eq!(value_after(&args, "-c:v"), Some("h264_nvenc"));
        assert!(has_pair(&args, "-pix_fmt", "yuv420p"));
        assert!(has_pair(&args, "-f", "mp4"));
        assert_eq!(
            args.last().map(String::as_str),
            Some(r"C:\Videos\capture.mp4")
        );
    }

    #[test]
    fn a_window_target_is_cropped_like_a_region() {
        let request = VideoRequest {
            target: RecordingTarget::window(crate::ids::WindowId::new(77), region()),
            ..video_request()
        };
        let args = strings(&command_video(&request));
        assert_eq!(
            value_after(&args, "-i"),
            Some("ddagrab=framerate=30:output_idx=0:video_size=640x480:offset_x=100:offset_y=200")
        );
    }

    #[test]
    fn every_video_frame_rate_reaches_ddagrab() {
        for frame_rate in VideoFrameRate::ALL {
            let request = VideoRequest {
                frame_rate,
                ..video_request()
            };
            let args = strings(&command_video(&request));
            let expected = format!("ddagrab=framerate={}", frame_rate.as_u32());
            assert!(
                value_after(&args, "-i").is_some_and(|source| source.starts_with(&expected)),
                "{frame_rate:?}"
            );
        }
    }

    #[test]
    fn empty_and_negative_targets_are_rejected() {
        let request = VideoRequest {
            target: RecordingTarget::region(Rect::ZERO),
            ..video_request()
        };
        assert_eq!(video_command(&request), Err(FfmpegArgsError::EmptyTarget));

        let request = VideoRequest {
            target: RecordingTarget::region(Rect::new(Px(-1), Px(0), Px(10), Px(10))),
            ..video_request()
        };
        assert_eq!(
            video_command(&request),
            Err(FfmpegArgsError::NegativeTargetOffset)
        );
    }

    #[test]
    fn mismatched_encoders_are_rejected() {
        let request = VideoRequest {
            format: VideoFormat::Mp4H264,
            encoder: VideoEncoder::LibVp9,
            ..video_request()
        };
        assert_eq!(
            video_command(&request),
            Err(FfmpegArgsError::EncoderFormatMismatch {
                encoder: VideoEncoder::LibVp9,
                format: VideoFormat::Mp4H264
            })
        );

        let request = VideoRequest {
            format: VideoFormat::MkvH264,
            encoder: VideoEncoder::HevcNvenc,
            ..video_request()
        };
        assert!(matches!(
            video_command(&request),
            Err(FfmpegArgsError::EncoderFormatMismatch { .. })
        ));
    }

    #[test]
    fn no_audio_has_no_audio_inputs_or_maps() {
        let args = strings(&command_video(&video_request()));
        assert!(!args
            .iter()
            .any(|argument| argument.contains("snapline-audio")));
        assert!(!args.iter().any(|argument| argument == "-filter_complex"));
        assert!(!args.iter().any(|argument| argument == "1:a:0"));
        assert!(!args.iter().any(|argument| argument == "amix"));
    }

    #[test]
    fn one_audio_pipe_is_mapped_without_amix() {
        let request = VideoRequest {
            audio: vec![audio(AudioSource::System, 0)],
            ..video_request()
        };
        let args = strings(&command_video(&request));
        assert!(has_pair(&args, "-i", r"\\.\pipe\snapline-audio-0"));
        assert!(has_pair(&args, "-f", "s16le"));
        assert!(has_pair(&args, "-ar", "48000"));
        assert!(has_pair(&args, "-ac", "2"));
        assert!(has_pair(&args, "-map", "1:a:0"));
        assert!(!args.iter().any(|argument| argument.contains("amix")));
    }

    #[test]
    fn system_and_microphone_pipes_are_mixed_with_amix() {
        let request = VideoRequest {
            audio: vec![
                audio(AudioSource::System, 0),
                audio(AudioSource::Microphone, 1),
            ],
            ..video_request()
        };
        let args = strings(&command_video(&request));
        assert!(has_pair(&args, "-i", r"\\.\pipe\snapline-audio-0"));
        assert!(has_pair(&args, "-i", r"\\.\pipe\snapline-audio-1"));
        assert_eq!(
            value_after(&args, "-filter_complex"),
            Some("[1:a][2:a]amix=inputs=2[aout]")
        );
        assert!(has_pair(&args, "-map", "[aout]"));
        assert!(!args.iter().any(|argument| argument == "1:a:0"));
    }

    #[test]
    fn pcm_sample_formats_reach_the_input_arguments() {
        for (sample_format, name) in [
            (PcmSampleFormat::S16Le, "s16le"),
            (PcmSampleFormat::F32Le, "f32le"),
            (PcmSampleFormat::S32Le, "s32le"),
        ] {
            let request = VideoRequest {
                audio: vec![AudioPipe {
                    source: AudioSource::System,
                    index: 0,
                    sample_format,
                    sample_rate: 48_000,
                    channels: 2,
                }],
                ..video_request()
            };
            let args = strings(&command_video(&request));
            assert!(has_pair(&args, "-f", name), "{sample_format:?}");
        }
    }

    #[test]
    fn invalid_audio_formats_are_rejected() {
        let request = VideoRequest {
            audio: vec![AudioPipe {
                sample_rate: 0,
                ..audio(AudioSource::System, 0)
            }],
            ..video_request()
        };
        assert_eq!(
            video_command(&request),
            Err(FfmpegArgsError::InvalidAudioFormat)
        );

        let request = VideoRequest {
            audio: vec![AudioPipe {
                channels: 0,
                ..audio(AudioSource::System, 0)
            }],
            ..video_request()
        };
        assert_eq!(
            video_command(&request),
            Err(FfmpegArgsError::InvalidAudioFormat)
        );
    }

    #[test]
    fn audio_pipe_paths_follow_the_documented_pattern() {
        assert_eq!(audio_pipe_path(0), r"\\.\pipe\snapline-audio-0");
        assert_eq!(audio_pipe_path(12), r"\\.\pipe\snapline-audio-12");
        assert_eq!(
            audio(AudioSource::Microphone, 3).path(),
            r"\\.\pipe\snapline-audio-3"
        );
        assert_eq!(AUDIO_PIPE_PREFIX, r"\\.\pipe\snapline-audio-");
    }

    #[test]
    fn encoder_candidates_are_priority_ordered_with_fallbacks_last() {
        assert_eq!(
            encoder_candidates(VideoFormat::Mp4H264),
            [
                VideoEncoder::H264Nvenc,
                VideoEncoder::H264Qsv,
                VideoEncoder::H264Amf,
                VideoEncoder::LibX264
            ]
            .as_slice()
        );
        assert_eq!(
            encoder_candidates(VideoFormat::MkvH264),
            [
                VideoEncoder::H264Nvenc,
                VideoEncoder::H264Qsv,
                VideoEncoder::H264Amf,
                VideoEncoder::LibX264
            ]
            .as_slice()
        );
        assert_eq!(
            encoder_candidates(VideoFormat::Mp4H265),
            [
                VideoEncoder::HevcNvenc,
                VideoEncoder::HevcQsv,
                VideoEncoder::HevcAmf,
                VideoEncoder::LibX265
            ]
            .as_slice()
        );
        assert_eq!(
            encoder_candidates(VideoFormat::WebmVp9),
            [VideoEncoder::LibVp9].as_slice()
        );
        assert_eq!(hardware_candidates(VideoFormat::Mp4H264).len(), 3);
        assert_eq!(hardware_candidates(VideoFormat::Mp4H265).len(), 3);
        assert!(hardware_candidates(VideoFormat::WebmVp9).is_empty());
        assert_eq!(
            fallback_encoder(VideoFormat::Mp4H264),
            VideoEncoder::LibX264
        );
        assert_eq!(
            fallback_encoder(VideoFormat::Mp4H265),
            VideoEncoder::LibX265
        );
        assert_eq!(fallback_encoder(VideoFormat::WebmVp9), VideoEncoder::LibVp9);
    }

    #[test]
    fn encoder_names_round_trip() {
        for encoder in [
            VideoEncoder::H264Nvenc,
            VideoEncoder::H264Qsv,
            VideoEncoder::H264Amf,
            VideoEncoder::HevcNvenc,
            VideoEncoder::HevcQsv,
            VideoEncoder::HevcAmf,
            VideoEncoder::LibX264,
            VideoEncoder::LibX265,
            VideoEncoder::LibVp9,
        ] {
            assert_eq!(VideoEncoder::from_name(encoder.name()), Some(encoder));
        }
        assert_eq!(VideoEncoder::from_name("h264_vaapi"), None);
        assert!(VideoEncoder::H264Nvenc.is_hardware());
        assert!(!VideoEncoder::LibX264.is_hardware());
    }

    #[test]
    fn gif_defaults_match_the_plan() {
        let settings = GifSettings::default();
        assert_eq!(settings.frame_rate, GifFrameRate::Fps15);
        assert_eq!(settings.max_width, GifMaxWidth::Px800);
        assert!(settings.optimise_palette);
        assert_eq!(
            GifFrameRate::ALL.map(GifFrameRate::as_u32),
            [10, 15, 20, 30]
        );
        assert_eq!(
            GifMaxWidth::ALL.map(GifMaxWidth::pixels),
            [None, Some(1280), Some(800), Some(480)]
        );
    }

    #[test]
    fn every_gif_frame_rate_width_and_palette_choice_builds() {
        for frame_rate in GifFrameRate::ALL {
            for max_width in GifMaxWidth::ALL {
                for optimise_palette in [false, true] {
                    let settings = GifSettings {
                        frame_rate,
                        max_width,
                        optimise_palette,
                    };
                    let command = match gif_convert_command(&gif_convert_request(settings)) {
                        Ok(command) => command,
                        Err(error) => panic!("gif command was rejected: {error}"),
                    };
                    let args = strings(&command);
                    let filter = value_after(&args, "-filter_complex");
                    let Some(filter) = filter else {
                        panic!("missing gif filter");
                    };
                    assert!(filter.contains(&format!("fps={}", frame_rate.as_u32())));
                    match max_width.pixels() {
                        Some(width) => assert!(filter
                            .contains(&format!("scale=w=min({width}\\,iw):h=-2:flags=lanczos"))),
                        None => assert!(!filter.contains("scale=")),
                    }
                    assert_eq!(filter.contains("stats_mode=diff"), optimise_palette);
                    assert!(filter.contains("palettegen"));
                    assert!(filter.contains("paletteuse"));
                    assert!(has_pair(&args, "-loop", "0"));
                    assert!(has_pair(&args, "-f", "gif"));
                    assert!(args.iter().any(|argument| argument == "-an"));
                    assert_eq!(
                        args.last().map(String::as_str),
                        Some(r"C:\Videos\capture.gif")
                    );
                }
            }
        }
    }

    #[test]
    fn gif_capture_records_a_lossless_mp4_at_the_gif_frame_rate() {
        let request = GifCaptureRequest {
            frame_rate: GifFrameRate::Fps20,
            ..gif_capture_request()
        };
        let command = match gif_capture_command(&request) {
            Ok(command) => command,
            Err(error) => panic!("gif capture was rejected: {error}"),
        };
        let args = strings(&command);
        assert_eq!(value_after(&args, "-c:v"), Some("libx264"));
        assert!(has_pair(&args, "-crf", "0"));
        assert!(has_pair(&args, "-pix_fmt", "yuv444p"));
        assert!(has_pair(&args, "-f", "mp4"));
        assert!(
            value_after(&args, "-i").is_some_and(|source| source.contains("ddagrab=framerate=20:"))
        );
        assert_eq!(
            args.last().map(String::as_str),
            Some(r"C:\Temp\snapline-gif.mp4")
        );
    }

    #[test]
    fn converter_gif_reuses_the_gif_palette_filter() {
        let request = ConvertRequest {
            ffmpeg: ffmpeg(),
            input: PathBuf::from(r"C:\Videos\source.mp4"),
            output: PathBuf::from(r"C:\Videos\converted.gif"),
            format: ConvertFormat::Gif(GifSettings {
                frame_rate: GifFrameRate::Fps10,
                max_width: GifMaxWidth::Px480,
                optimise_palette: true,
            }),
        };
        let command = match convert_command(&request) {
            Ok(command) => command,
            Err(error) => panic!("converter was rejected: {error}"),
        };
        let args = strings(&command);
        let filter = value_after(&args, "-filter_complex");
        assert!(filter.is_some_and(|filter| filter.contains("fps=10")));
        assert!(filter.is_some_and(|filter| filter.contains("scale=w=min(480\\,iw)")));
        assert!(filter.is_some_and(|filter| filter.contains("stats_mode=diff")));
        assert!(has_pair(&args, "-f", "gif"));
    }

    #[test]
    fn converter_webm_uses_vp9() {
        let request = ConvertRequest {
            ffmpeg: ffmpeg(),
            input: PathBuf::from(r"C:\Videos\source.mp4"),
            output: PathBuf::from(r"C:\Videos\converted.webm"),
            format: ConvertFormat::WebmVp9,
        };
        let command = match convert_command(&request) {
            Ok(command) => command,
            Err(error) => panic!("converter was rejected: {error}"),
        };
        let args = strings(&command);
        assert!(has_pair(&args, "-c:v", "libvpx-vp9"));
        assert!(has_pair(&args, "-f", "webm"));
    }

    #[test]
    fn converter_smaller_mp4_uses_crf_28() {
        let request = ConvertRequest {
            ffmpeg: ffmpeg(),
            input: PathBuf::from(r"C:\Videos\source.mp4"),
            output: PathBuf::from(r"C:\Videos\converted.mp4"),
            format: ConvertFormat::Mp4Small,
        };
        let command = match convert_command(&request) {
            Ok(command) => command,
            Err(error) => panic!("converter was rejected: {error}"),
        };
        let args = strings(&command);
        assert!(has_pair(&args, "-c:v", "libx264"));
        assert!(has_pair(&args, "-crf", "28"));
        assert!(has_pair(&args, "-f", "mp4"));
    }

    #[test]
    fn paths_with_shell_metacharacters_stay_single_arguments() {
        let nasty = PathBuf::from(r"C:\Videos\a & b | c ; d % ! ' ^ (x).mp4");
        let request = VideoRequest {
            output: nasty.clone(),
            ..video_request()
        };
        let args = strings(&command_video(&request));
        let expected = nasty.to_string_lossy().into_owned();
        assert_eq!(args.last(), Some(&expected));
        assert_eq!(
            args.iter()
                .filter(|argument| argument.contains("a & b"))
                .count(),
            1
        );
        assert!(!args.iter().any(|argument| argument.contains("\"C:")));
        assert!(!args.iter().any(|argument| argument.contains("^C:")));
        assert!(has_pair(&args, "-f", "mp4"));
    }

    #[test]
    fn converter_paths_with_metacharacters_stay_single_arguments() {
        let input = PathBuf::from(r"C:\Videos\one & two; three.mp4");
        let output = PathBuf::from(r"C:\Videos\four | five.gif");
        let request = ConvertRequest {
            ffmpeg: ffmpeg(),
            input: input.clone(),
            output: output.clone(),
            format: ConvertFormat::Gif(GifSettings::default()),
        };
        let command = match convert_command(&request) {
            Ok(command) => command,
            Err(error) => panic!("converter was rejected: {error}"),
        };
        let args = strings(&command);
        let input_text = input.to_string_lossy().into_owned();
        let output_text = output.to_string_lossy().into_owned();
        assert_eq!(value_after(&args, "-i"), Some(input_text.as_str()));
        assert_eq!(args.last().map(String::as_str), Some(output_text.as_str()));
        assert_eq!(
            args.iter()
                .filter(|argument| argument.contains("one & two"))
                .count(),
            1
        );
        assert_eq!(
            args.iter()
                .filter(|argument| argument.contains("four | five"))
                .count(),
            1
        );
    }

    #[test]
    fn relative_ffmpeg_paths_are_rejected_by_every_builder() {
        let relative = PathBuf::from(r"tools\ffmpeg.exe");

        let request = VideoRequest {
            ffmpeg: relative.clone(),
            ..video_request()
        };
        assert_eq!(
            video_command(&request),
            Err(FfmpegArgsError::FfmpegPathNotAbsolute)
        );

        let request = GifCaptureRequest {
            ffmpeg: relative.clone(),
            ..gif_capture_request()
        };
        assert_eq!(
            gif_capture_command(&request),
            Err(FfmpegArgsError::FfmpegPathNotAbsolute)
        );

        let request = GifConvertRequest {
            ffmpeg: relative.clone(),
            ..gif_convert_request(GifSettings::default())
        };
        assert_eq!(
            gif_convert_command(&request),
            Err(FfmpegArgsError::FfmpegPathNotAbsolute)
        );

        let request = ConvertRequest {
            ffmpeg: relative.clone(),
            input: PathBuf::from(r"C:\Videos\source.mp4"),
            output: PathBuf::from(r"C:\Videos\converted.webm"),
            format: ConvertFormat::WebmVp9,
        };
        assert_eq!(
            convert_command(&request),
            Err(FfmpegArgsError::FfmpegPathNotAbsolute)
        );

        assert_eq!(
            encoder_detection_command(&relative),
            Err(FfmpegArgsError::FfmpegPathNotAbsolute)
        );
        assert_eq!(
            test_encode_command(&relative, VideoEncoder::H264Nvenc),
            Err(FfmpegArgsError::FfmpegPathNotAbsolute)
        );
        assert_eq!(
            encoder_detection_command(Path::new("ffmpeg.exe")),
            Err(FfmpegArgsError::FfmpegPathNotAbsolute)
        );
    }

    #[test]
    fn absolute_ffmpeg_paths_are_accepted() {
        assert!(encoder_detection_command(&ffmpeg()).is_ok());
        assert!(test_encode_command(&ffmpeg(), VideoEncoder::LibX264).is_ok());
    }

    #[test]
    fn encoder_detection_uses_the_documented_command() {
        let command = match encoder_detection_command(&ffmpeg()) {
            Ok(command) => command,
            Err(error) => panic!("detection was rejected: {error}"),
        };
        assert_eq!(strings(&command), ["-hide_banner", "-encoders"]);
        assert_eq!(command.program(), Path::new(r"C:\Tools\ffmpeg.exe"));
    }

    #[test]
    fn test_encode_builds_a_single_frame_null_encode() {
        for encoder in [
            VideoEncoder::H264Nvenc,
            VideoEncoder::H264Qsv,
            VideoEncoder::H264Amf,
            VideoEncoder::HevcNvenc,
            VideoEncoder::HevcQsv,
            VideoEncoder::HevcAmf,
            VideoEncoder::LibX264,
            VideoEncoder::LibX265,
            VideoEncoder::LibVp9,
        ] {
            let command = match test_encode_command(&ffmpeg(), encoder) {
                Ok(command) => command,
                Err(error) => panic!("test encode was rejected: {error}"),
            };
            let args = strings(&command);
            assert_eq!(
                args,
                [
                    "-hide_banner",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=black:s=256x256:r=1",
                    "-frames:v",
                    "1",
                    "-c:v",
                    encoder.name(),
                    "-pix_fmt",
                    "yuv420p",
                    "-f",
                    "null",
                    "-",
                ]
            );
        }
    }
}
