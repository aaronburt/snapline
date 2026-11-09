use std::ffi::c_void;
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use snapline_core::ffmpeg_args::{
    audio_pipe_path, AudioPipe, AudioSource, PcmSampleFormat, AUDIO_PIPE_PREFIX,
};
use thiserror::Error;
use windows::core::{Interface, BOOL, GUID, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, HANDLE, PROPERTYKEY, RPC_E_CHANGED_MODE, ERROR_BROKEN_PIPE,
    ERROR_NO_DATA, ERROR_OPERATION_ABORTED, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED,
};
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice,
    IMMDeviceCollection, IMMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_LOOPBACK, DEVICE_STATE_ACTIVE, MMDeviceEnumerator, WAVE_FORMAT_PCM,
};
use windows::Win32::Media::KernelStreaming::{
    KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE,
};
use windows::Win32::Media::Multimedia::{
    KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT,
};
use windows::Win32::System::Com::StructuredStorage::{
    PropVariantClear, PropVariantToStringAlloc, PROPVARIANT,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::{
    GetCurrentThreadId, OpenThread, THREAD_TERMINATE,
};

const DEFAULT_SAMPLE_RATE: u32 = 48_000;
const DEFAULT_CHANNELS: u16 = 2;
const PIPE_BUFFER_BYTES: u32 = 256 * 1024;
const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const IDLE_SLEEP: Duration = Duration::from_millis(5);
const PROGRESS_INTERVAL: Duration = Duration::from_secs(1);
const CONTINUOUS_FRAMES_PER_SECOND: u32 = 1_000_000_000;

const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};
const PKEY_DEVICE_DEVICE_DESC: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 2,
};
const PKEY_DEVICE_INTERFACE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x026e516e_b814_414b_83cd_856d6fef4822),
    pid: 2,
};

static NEXT_PIPE_INDEX: AtomicU32 = AtomicU32::new(0);

#[link(name = "kernel32")]
extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_buffer: u32,
        in_buffer: u32,
        timeout: u32,
        security: *const c_void,
    ) -> HANDLE;
    fn ConnectNamedPipe(pipe: HANDLE, overlapped: *mut c_void) -> BOOL;
    fn DisconnectNamedPipe(pipe: HANDLE) -> BOOL;
    fn WriteFile(
        pipe: HANDLE,
        buffer: *const u8,
        bytes: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> BOOL;
    fn CancelSynchronousIo(thread: HANDLE) -> BOOL;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmFormat {
    pub sample_format: PcmSampleFormat,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Default for PcmFormat {
    fn default() -> Self {
        Self {
            sample_format: PcmSampleFormat::S16Le,
            sample_rate: DEFAULT_SAMPLE_RATE,
            channels: DEFAULT_CHANNELS,
        }
    }
}

impl PcmFormat {
    pub fn bytes_per_sample(self) -> usize {
        sample_bytes(self.sample_format)
    }

    pub fn frame_size(self) -> usize {
        usize::from(self.channels) * self.bytes_per_sample()
    }

    pub fn chunk_frames(self) -> usize {
        (self.sample_rate as usize / 50).max(1)
    }

    pub fn chunk_bytes(self) -> usize {
        self.chunk_frames() * self.frame_size()
    }

    pub fn silence(self, frames: usize) -> Vec<u8> {
        vec![0; frames.saturating_mul(self.frame_size())]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioEvent {
    Started {
        source: AudioSource,
        pipe: String,
    },
    Progress {
        source: AudioSource,
        frames: u64,
        bytes: u64,
    },
    Stopped {
        source: AudioSource,
        frames: u64,
        bytes: u64,
    },
    Disconnected {
        source: AudioSource,
        frames: u64,
        bytes: u64,
    },
    Error {
        source: AudioSource,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct AudioRequest {
    pub system: bool,
    pub microphone: bool,
    pub microphone_id: Option<String>,
    pub format: PcmFormat,
    pub events: Option<Sender<AudioEvent>>,
}

impl Default for AudioRequest {
    fn default() -> Self {
        Self {
            system: true,
            microphone: false,
            microphone_id: None,
            format: PcmFormat::default(),
            events: None,
        }
    }
}

impl AudioRequest {
    pub fn validate(&self) -> Result<(), AudioError> {
        if !self.system && !self.microphone {
            return Err(AudioError::NoStreams);
        }
        if self.format.sample_rate == 0
            || self.format.channels == 0
            || self.format.channels > 2
            || self.format.bytes_per_sample() == 0
        {
            return Err(AudioError::InvalidFormat);
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("audio capture needs at least one source")]
    NoStreams,
    #[error("the audio format must have one or two channels at a positive rate")]
    InvalidFormat,
    #[error("no default {device} device is available")]
    NoDefaultDevice { device: &'static str },
    #[error("the audio device {id} is not available")]
    DeviceNotFound { id: String },
    #[error("the audio device uses a mix format that Snapline cannot convert")]
    UnsupportedMixFormat,
    #[error("the audio pipe {pipe} could not be used: {message}")]
    Pipe { pipe: String, message: String },
    #[error("the {source:?} audio capture thread could not start: {message}")]
    Thread {
        source: AudioSource,
        message: String,
    },
    #[error("the audio device enumerator is unavailable")]
    Enumerator,
    #[error("audio capture for {source:?} failed: {message}")]
    Capture {
        source: AudioSource,
        message: String,
    },
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AudioCapture;

impl AudioCapture {
    pub fn microphones() -> Vec<AudioDevice> {
        microphones()
    }

    pub fn default_microphone() -> Option<AudioDevice> {
        default_microphone()
    }

    pub fn start(request: AudioRequest) -> Result<RunningAudio, AudioError> {
        request.validate()?;
        let sources = resolve_sources(&request)?;
        let mut stop = Arc::new(AtomicBool::new(false));
        let mut pipes = Vec::with_capacity(sources.len());
        let mut descriptors = Vec::with_capacity(sources.len());
        for source in &sources {
            let index = next_pipe_index();
            let path = audio_pipe_path(index);
            let pipe = match create_pipe(&path) {
                Ok(pipe) => pipe,
                Err(error) => return Err(error),
            };
            pipes.push(pipe);
            descriptors.push(AudioPipe {
                source: source.source,
                index,
                sample_format: request.format.sample_format,
                sample_rate: request.format.sample_rate,
                channels: request.format.channels,
            });
        }

        let mut workers = Vec::with_capacity(sources.len());
        for ((source, pipe), descriptor) in sources.into_iter().zip(pipes).zip(&descriptors) {
            let thread_id = Arc::new(AtomicU32::new(0));
            let thread_id_in = Arc::clone(&thread_id);
            let stop_in = Arc::clone(&stop);
            let events = request.events.clone();
            let format = request.format;
            let pipe_path = descriptor.path();
            let name = format!("snapline-audio-{}", descriptor.index);
            let spawned = thread::Builder::new().name(name).spawn(move || {
                capture_thread(
                    source.source,
                    source.device_id,
                    source.loopback,
                    format,
                    pipe,
                    pipe_path,
                    stop_in,
                    thread_id_in,
                    events,
                );
            });
            match spawned {
                Ok(join) => workers.push(Worker {
                    join: Some(join),
                    thread_id,
                }),
                Err(error) => {
                    stop.store(true, Ordering::SeqCst);
                    join_workers(&mut workers);
                    return Err(AudioError::Thread {
                        source: source.source,
                        message: error.to_string(),
                    });
                }
            }
        }

        stop = Arc::clone(&stop);
        Ok(RunningAudio {
            pipes: descriptors,
            workers: Mutex::new(workers),
            stop,
        })
    }
}

#[derive(Debug)]
pub struct RunningAudio {
    pipes: Vec<AudioPipe>,
    workers: Mutex<Vec<Worker>>,
    stop: Arc<AtomicBool>,
}

impl RunningAudio {
    pub fn pipes(&self) -> Vec<AudioPipe> {
        self.pipes.clone()
    }

    pub fn pipe_names(&self) -> Vec<String> {
        self.pipes.iter().map(AudioPipe::path).collect()
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let Ok(mut guard) = self.workers.lock() else {
            return;
        };
        let mut workers = std::mem::take(&mut *guard);
        drop(guard);
        join_workers(&mut workers);
    }
}

impl Drop for RunningAudio {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn microphones() -> Vec<AudioDevice> {
    match capture_devices() {
        Ok(devices) => devices,
        Err(error) => {
            tracing::warn!(%error, "could not enumerate microphones");
            Vec::new()
        }
    }
}

pub fn default_microphone() -> Option<AudioDevice> {
    microphones().into_iter().find(|device| device.default)
}

fn capture_devices() -> Result<Vec<AudioDevice>, AudioError> {
    let _com = ComApartment::start()?;
    let enumerator = create_enumerator()?;
    enumerate(&enumerator, eCapture, "microphone")
}

fn enumerate(
    enumerator: &IMMDeviceEnumerator,
    flow: windows::Win32::Media::Audio::EDataFlow,
    label: &'static str,
) -> Result<Vec<AudioDevice>, AudioError> {
    let collection: IMMDeviceCollection = unsafe {
        enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)
    }
    .map_err(|_| AudioError::Enumerator)?;
    let default_id = default_device(enumerator, flow, label)
        .ok()
        .and_then(|device| device_id(&device).ok());
    let count = unsafe { collection.GetCount() }.map_err(|_| AudioError::Enumerator)?;
    let mut devices = Vec::with_capacity(count as usize);
    for index in 0..count {
        let Ok(device) = (unsafe { collection.Item(index) }) else {
            continue;
        };
        let Ok(id) = device_id(&device) else {
            continue;
        };
        let name = friendly_name(&device)
            .unwrap_or_else(|| format!("{} {}", capitalize(label), devices.len() + 1));
        let default = default_id.as_deref() == Some(id.as_str());
        devices.push(AudioDevice { id, name, default });
    }
    Ok(devices)
}

fn capitalize(value: &str) -> String {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

fn friendly_name(device: &IMMDevice) -> Option<String> {
    let store = open_property_store(device)?;
    if let Some(name) = store.get_string(&PKEY_DEVICE_FRIENDLY_NAME) {
        return Some(name);
    }
    let description = store.get_string(&PKEY_DEVICE_DEVICE_DESC);
    let interface = store.get_string(&PKEY_DEVICE_INTERFACE_FRIENDLY_NAME);
    compose_name(description.as_deref(), interface.as_deref())
}

fn compose_name(description: Option<&str>, interface: Option<&str>) -> Option<String> {
    match (description, interface) {
        (Some(description), Some(interface))
            if !interface.is_empty() && !interface.eq_ignore_ascii_case(description) =>
        {
            Some(format!("{description} ({interface})"))
        }
        (Some(description), _) if !description.is_empty() => Some(description.to_owned()),
        (_, Some(interface)) if !interface.is_empty() => Some(interface.to_owned()),
        _ => None,
    }
}

struct ComApartment {
    uninit: bool,
}

impl ComApartment {
    fn start() -> Result<Self, AudioError> {
        // SAFETY: no pointer is shared and the matching CoUninitialize runs in Drop on this thread.
        let status = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if status.is_ok() {
            Ok(Self { uninit: true })
        } else if status == RPC_E_CHANGED_MODE {
            Ok(Self { uninit: false })
        } else {
            Err(AudioError::Enumerator)
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.uninit {
            // SAFETY: this balances the successful CoInitializeEx in start on the same thread.
            unsafe { CoUninitialize() };
        }
    }
}

fn create_enumerator() -> Result<IMMDeviceEnumerator, AudioError> {
    // SAFETY: the MMDevice enumerator is registered in-process and the returned interface owns a reference.
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
        .map_err(|_| AudioError::Enumerator)
}

fn default_device(
    enumerator: &IMMDeviceEnumerator,
    flow: windows::Win32::Media::Audio::EDataFlow,
    label: &'static str,
) -> Result<IMMDevice, AudioError> {
    // SAFETY: the enumerator is live and returns an owned device interface.
    unsafe { enumerator.GetDefaultAudioEndpoint(flow, eConsole) }
        .map_err(|_| AudioError::NoDefaultDevice { device: label })
}

fn device_id(device: &IMMDevice) -> Result<String, AudioError> {
    // SAFETY: the device is live and the returned string is owned by the caller.
    let id = unsafe { device.GetId() }.map_err(|_| AudioError::Enumerator)?;
    let memory = CoTaskMemory(id.0.cast::<c_void>());
    // SAFETY: the pointer returned by GetId is a live null-terminated UTF-16 string.
    let text = unsafe { id.to_string() }.map_err(|_| AudioError::Enumerator);
    drop(memory);
    text
}

struct CoTaskMemory(*mut c_void);

impl Drop for CoTaskMemory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer was allocated by COM with CoTaskMemAlloc and is owned by this guard.
            unsafe { CoTaskMemFree(Some(self.0)) };
        }
    }
}

struct PropertyStore {
    raw: *mut c_void,
    vtable: *const StoreVtable,
}

impl PropertyStore {
    fn get_string(&self, key: &PROPERTYKEY) -> Option<String> {
        let mut value = PROPVARIANT::default();
        // SAFETY: the store and key are live and the out-pointer refers to a writable PROPVARIANT.
        let status = unsafe { (self.vtable().get_value)(self.raw, key, &mut value) };
        if status.is_err() {
            return None;
        }
        // SAFETY: the PROPVARIANT was filled by GetValue and the returned string is allocated with CoTaskMemAlloc.
        let text = unsafe { PropVariantToStringAlloc(&value) }
            .ok()
            .and_then(|text| string_from_pwstr(text));
        // SAFETY: the PROPVARIANT is owned by this function and is cleared exactly once.
        unsafe {
            let _ = PropVariantClear(&mut value);
        }
        text.filter(|text| !text.is_empty())
    }

    fn vtable(&self) -> &StoreVtable {
        // SAFETY: COM interface pointers always begin with a valid vtable pointer.
        unsafe { &*self.vtable }
    }
}

impl Drop for PropertyStore {
    fn drop(&mut self) {
        // SAFETY: the store owns one reference obtained from OpenPropertyStore and is released once.
        unsafe {
            (self.vtable().base.Release)(self.raw);
        }
    }
}

fn string_from_pwstr(text: PWSTR) -> Option<String> {
    // SAFETY: the pointer is a live null-terminated UTF-16 string allocated with CoTaskMemAlloc.
    let value = unsafe { text.to_string() }.ok();
    // SAFETY: the string was allocated by PropVariantToStringAlloc with CoTaskMemAlloc.
    unsafe { CoTaskMemFree(Some(text.0.cast::<c_void>())) };
    value
}

#[repr(C)]
struct DeviceVtable {
    base: windows::core::IUnknown_Vtbl,
    activate: usize,
    open_property_store: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> windows::core::HRESULT,
    get_id: usize,
    get_state: usize,
}

#[repr(C)]
struct StoreVtable {
    base: windows::core::IUnknown_Vtbl,
    get_count: usize,
    get_at: usize,
    get_value: unsafe extern "system" fn(
        *mut c_void,
        *const PROPERTYKEY,
        *mut PROPVARIANT,
    ) -> windows::core::HRESULT,
    set_value: usize,
    commit: usize,
}

fn open_property_store(device: &IMMDevice) -> Option<PropertyStore> {
    let raw = Interface::as_raw(device);
    if raw.is_null() {
        return None;
    }
    // SAFETY: the device is a live COM interface, so its first pointer is a valid IMMDevice vtable.
    let vtable = unsafe { *raw.cast::<*const DeviceVtable>() };
    if vtable.is_null() {
        return None;
    }
    let mut store = ptr::null_mut();
    // SAFETY: the vtable and device pointer are live and the out-pointer is writable; STGM_READ is zero.
    let status = unsafe { ((*vtable).open_property_store)(raw, 0, &mut store) };
    if status.is_err() || store.is_null() {
        return None;
    }
    // SAFETY: the returned store is a live COM interface, so its first pointer is a valid IPropertyStore vtable.
    let store_vtable = unsafe { *store.cast::<*const StoreVtable>() };
    if store_vtable.is_null() {
        return None;
    }
    Some(PropertyStore {
        raw: store,
        vtable: store_vtable,
    })
}

struct StreamPlan {
    source: AudioSource,
    device_id: String,
    loopback: bool,
}

fn resolve_sources(request: &AudioRequest) -> Result<Vec<StreamPlan>, AudioError> {
    let _com = ComApartment::start()?;
    let enumerator = create_enumerator()?;
    let mut plans = Vec::with_capacity(2);
    if request.system {
        let device = default_device(&enumerator, eRender, "render")?;
        plans.push(StreamPlan {
            source: AudioSource::System,
            device_id: device_id(&device)?,
            loopback: true,
        });
    }
    if request.microphone {
        let device = match request.microphone_id.as_deref() {
            Some(id) => {
                let wanted = HSTRING::from(id);
                // SAFETY: the enumerator is live and the string is a valid null-terminated device id.
                unsafe { enumerator.GetDevice(&wanted) }.map_err(|_| AudioError::DeviceNotFound {
                    id: id.to_owned(),
                })?
            }
            None => default_device(&enumerator, eCapture, "capture")?,
        };
        plans.push(StreamPlan {
            source: AudioSource::Microphone,
            device_id: device_id(&device)?,
            loopback: false,
        });
    }
    Ok(plans)
}

fn next_pipe_index() -> u32 {
    NEXT_PIPE_INDEX.fetch_add(1, Ordering::Relaxed)
}

struct OwnedPipe {
    raw: usize,
}

impl OwnedPipe {
    fn new(handle: HANDLE) -> Self {
        Self {
            raw: handle.0 as usize,
        }
    }

    fn handle(&self) -> HANDLE {
        HANDLE(self.raw as *mut c_void)
    }
}

impl Drop for OwnedPipe {
    fn drop(&mut self) {
        if self.raw != 0 {
            // SAFETY: the handle was created by CreateNamedPipeW and is owned by this value for its whole life.
            unsafe {
                let _ = DisconnectNamedPipe(self.handle());
                let _ = CloseHandle(self.handle());
            }
            self.raw = 0;
        }
    }
}

fn create_pipe(path: &str) -> Result<OwnedPipe, AudioError> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: the name is a live null-terminated UTF-16 string and no security descriptor is needed.
    let handle = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_REJECT_REMOTE_CLIENTS,
            1,
            PIPE_BUFFER_BYTES,
            PIPE_BUFFER_BYTES,
            0,
            ptr::null(),
        )
    };
    if handle.is_invalid() {
        // SAFETY: GetLastError reads the calling thread's last error and takes no pointers.
        let error = unsafe { GetLastError() };
        return Err(AudioError::Pipe {
            pipe: path.to_owned(),
            message: format!("creation failed ({})", error.0),
        });
    }
    Ok(OwnedPipe::new(handle))
}

fn connect_pipe(pipe: &OwnedPipe, path: &str, stop: &AtomicBool) -> Result<bool, AudioError> {
    if stop.load(Ordering::SeqCst) {
        return Ok(false);
    }
    // SAFETY: the pipe handle is live and a null OVERLAPPED asks for a blocking connection.
    let connected = unsafe { ConnectNamedPipe(pipe.handle(), ptr::null_mut()) };
    if connected.as_bool() {
        return Ok(true);
    }
    // SAFETY: GetLastError reads the calling thread's last error and takes no pointers.
    let error = unsafe { GetLastError() };
    if stop.load(Ordering::SeqCst) || error == ERROR_OPERATION_ABORTED {
        return Ok(false);
    }
    if error == ERROR_PIPE_CONNECTED {
        return Ok(true);
    }
    Err(AudioError::Pipe {
        pipe: path.to_owned(),
        message: format!("connection failed ({})", error.0),
    })
}

enum PipeFailure {
    Disconnected,
    Error(u32),
}

fn is_disconnect(error: windows::Win32::Foundation::WIN32_ERROR) -> bool {
    error == ERROR_BROKEN_PIPE || error == ERROR_NO_DATA || error == ERROR_PIPE_NOT_CONNECTED
}

fn write_pipe(pipe: &OwnedPipe, mut bytes: &[u8]) -> Result<(), PipeFailure> {
    while !bytes.is_empty() {
        let length = u32::try_from(bytes.len()).map_err(|_| PipeFailure::Error(0))?;
        let mut written = 0_u32;
        // SAFETY: the handle and buffer are live for the duration of the call and the out-pointer is writable.
        let written_ok = unsafe {
            WriteFile(
                pipe.handle(),
                bytes.as_ptr(),
                length,
                &mut written,
                ptr::null_mut(),
            )
        };
        if written > 0 {
            bytes = &bytes[written as usize..];
        }
        if written_ok.as_bool() {
            continue;
        }
        if written > 0 {
            continue;
        }
        // SAFETY: GetLastError reads the calling thread's last error and takes no pointers.
        let error = unsafe { GetLastError() };
        if is_disconnect(error) {
            return Err(PipeFailure::Disconnected);
        }
        return Err(PipeFailure::Error(error.0));
    }
    Ok(())
}

struct Worker {
    join: Option<JoinHandle<()>>,
    thread_id: Arc<AtomicU32>,
}

impl Worker {
    fn is_finished(&self) -> bool {
        self.join
            .as_ref()
            .map_or(true, std::thread::JoinHandle::is_finished)
    }

    fn cancel(&self) {
        let id = self.thread_id.load(Ordering::SeqCst);
        if id == 0 {
            return;
        }
        let Ok(thread) = (unsafe { OpenThread(THREAD_TERMINATE, false, id) }) else {
            return;
        };
        // SAFETY: the thread handle is live and identifies this worker; a non-blocking call returns false.
        unsafe {
            let _ = CancelSynchronousIo(thread);
            let _ = CloseHandle(thread);
        }
    }
}

fn join_workers(workers: &mut Vec<Worker>) {
    let deadline = Instant::now() + STOP_TIMEOUT;
    while !workers.iter().all(Worker::is_finished) && Instant::now() < deadline {
        for worker in workers.iter() {
            worker.cancel();
        }
        thread::sleep(Duration::from_millis(20));
    }
    for worker in workers.iter_mut() {
        if let Some(join) = worker.join.take() {
            if join.is_finished() {
                let _ = join.join();
            }
        }
    }
}

fn capture_thread(
    source: AudioSource,
    device_id: String,
    loopback: bool,
    format: PcmFormat,
    pipe: OwnedPipe,
    pipe_path: String,
    stop: Arc<AtomicBool>,
    thread_id: Arc<AtomicU32>,
    events: Option<Sender<AudioEvent>>,
) {
    // SAFETY: GetCurrentThreadId takes no pointers and returns the calling thread's identifier.
    thread_id.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
    let mut stats = Stats::default();
    let result = run_capture(
        source,
        &device_id,
        loopback,
        format,
        &pipe,
        &pipe_path,
        &stop,
        &events,
        &mut stats,
    );
    let bytes = stats.frames.saturating_mul(format.frame_size() as u64);
    match result {
        Ok(Outcome::Stopped) => send_event(
            &events,
            AudioEvent::Stopped {
                source,
                frames: stats.frames,
                bytes,
            },
        ),
        Ok(Outcome::Disconnected) => send_event(
            &events,
            AudioEvent::Disconnected {
                source,
                frames: stats.frames,
                bytes,
            },
        ),
        Err(error) => send_event(
            &events,
            AudioEvent::Error {
                source,
                message: error.to_string(),
            },
        ),
    }
}

#[derive(Default)]
struct Stats {
    frames: u64,
}

enum Outcome {
    Stopped,
    Disconnected,
}

fn send_event(events: &Option<Sender<AudioEvent>>, event: AudioEvent) {
    if let Some(events) = events {
        let _ = events.send(event);
    }
}

#[allow(clippy::too_many_arguments)]
fn run_capture(
    source: AudioSource,
    device_id: &str,
    loopback: bool,
    format: PcmFormat,
    pipe: &OwnedPipe,
    pipe_path: &str,
    stop: &AtomicBool,
    events: &Option<Sender<AudioEvent>>,
    stats: &mut Stats,
) -> Result<Outcome, AudioError> {
    let _com = ComApartment::start()?;
    let enumerator = create_enumerator()?;
    let wanted = HSTRING::from(device_id);
    // SAFETY: the enumerator is live and the string is a valid null-terminated device id.
    let device = unsafe { enumerator.GetDevice(&wanted) }.map_err(|_| AudioError::DeviceNotFound {
        id: device_id.to_owned(),
    })?;
    // SAFETY: the device is live and returns an owned IAudioClient for the requested class context.
    let client: IAudioClient = unsafe { device.Activate(CLSCTX_ALL, None) }
        .map_err(|error| capture_error(source, &error))?;
    // SAFETY: the client is live and returns a mix format allocated with CoTaskMemAlloc.
    let mix = unsafe { client.GetMixFormat() }.map_err(|error| capture_error(source, &error))?;
    let mix = MixFormat(mix);
    // SAFETY: the format pointer comes from GetMixFormat and stays live until the guard drops.
    let device_format = unsafe { parse_mix_format(mix.0) }?;
    let flags = if loopback {
        AUDCLNT_STREAMFLAGS_LOOPBACK
    } else {
        0
    };
    // SAFETY: the client and format are live and the shared-mode periods are the documented zero values.
    unsafe {
        client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, mix.0, None)
    }
    .map_err(|error| capture_error(source, &error))?;
    // SAFETY: the initialised client is live and returns an owned capture client service.
    let capture: IAudioCaptureClient =
        unsafe { client.GetService() }.map_err(|error| capture_error(source, &error))?;
    if !connect_pipe(pipe, pipe_path, stop)? {
        return Ok(Outcome::Stopped);
    }
    // SAFETY: the client is initialised and starts a capture stream with no pointers.
    unsafe { client.Start() }.map_err(|error| capture_error(source, &error))?;
    send_event(
        events,
        AudioEvent::Started {
            source,
            pipe: pipe_path.to_owned(),
        },
    );
    let outcome = capture_loop(
        source,
        &capture,
        &device_format,
        format,
        pipe,
        stop,
        events,
        stats,
    );
    // SAFETY: the client was started by this function and is stopped exactly once here.
    let _ = unsafe { client.Stop() };
    outcome
}

struct MixFormat(*mut windows::Win32::Media::Audio::WAVEFORMATEX);

impl Drop for MixFormat {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer was allocated by GetMixFormat with CoTaskMemAlloc and is owned by this guard.
            unsafe { CoTaskMemFree(Some(self.0.cast::<c_void>())) };
        }
    }
}

fn capture_loop(
    source: AudioSource,
    capture: &IAudioCaptureClient,
    device_format: &DeviceFormat,
    format: PcmFormat,
    pipe: &OwnedPipe,
    stop: &AtomicBool,
    events: &Option<Sender<AudioEvent>>,
    stats: &mut Stats,
) -> Result<Outcome, AudioError> {
    let mut converter = Converter::new(*device_format, format);
    let mut buffer = Vec::with_capacity(format.chunk_bytes());
    let mut reported = Instant::now();
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(Outcome::Stopped);
        }
        // SAFETY: the capture client is live and the call takes no pointers.
        let packet = unsafe { capture.GetNextPacketSize() }
            .map_err(|error| capture_error(source, &error))?;
        if packet == 0 {
            thread::sleep(IDLE_SLEEP);
            report_progress(events, source, format, stats, &mut reported);
            continue;
        }
        let mut data = ptr::null_mut();
        let mut frames = 0_u32;
        let mut flags = 0_u32;
        // SAFETY: the capture client is live and all out-pointers refer to writable locals.
        unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }
            .map_err(|error| capture_error(source, &error))?;
        buffer.clear();
        if frames > 0 {
            if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                converter.silence(frames as usize, &mut buffer);
            } else {
                // SAFETY: the buffer belongs to the capture client and covers frames whole sample frames.
                let samples = unsafe {
                    slice::from_raw_parts(
                        data.cast_const(),
                        frames as usize * device_format.block_align,
                    )
                };
                converter.convert(samples, &mut buffer);
            }
        }
        let failure = if buffer.is_empty() {
            None
        } else {
            write_pipe(pipe, &buffer).err()
        };
        // SAFETY: the capture client is live and the number of frames matches the buffer obtained above.
        unsafe { capture.ReleaseBuffer(frames) }
            .map_err(|error| capture_error(source, &error))?;
        if !buffer.is_empty() {
            stats.frames = stats.frames.saturating_add(
                (buffer.len() / format.frame_size().max(1)) as u64,
            );
        }
        if let Some(failure) = failure {
            if stop.load(Ordering::SeqCst) {
                return Ok(Outcome::Stopped);
            }
            return match failure {
                PipeFailure::Disconnected => Ok(Outcome::Disconnected),
                PipeFailure::Error(code) => Err(AudioError::Capture {
                    source,
                    message: format!("the audio pipe write failed ({code})"),
                }),
            };
        }
        report_progress(events, source, format, stats, &mut reported);
    }
}

fn report_progress(
    events: &Option<Sender<AudioEvent>>,
    source: AudioSource,
    format: PcmFormat,
    stats: &Stats,
    reported: &mut Instant,
) {
    if reported.elapsed() < PROGRESS_INTERVAL {
        return;
    }
    send_event(
        events,
        AudioEvent::Progress {
            source,
            frames: stats.frames,
            bytes: stats.frames.saturating_mul(format.frame_size() as u64),
        },
    );
    *reported = Instant::now();
}

fn capture_error(source: AudioSource, error: &windows::core::Error) -> AudioError {
    AudioError::Capture {
        source,
        message: error.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputSample {
    F32,
    I16,
    I24,
    I32,
}

impl InputSample {
    const fn bytes(self) -> usize {
        match self {
            Self::F32 | Self::I32 => 4,
            Self::I24 => 3,
            Self::I16 => 2,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DeviceFormat {
    channels: u16,
    sample_rate: u32,
    sample: InputSample,
    block_align: usize,
}

unsafe fn parse_mix_format(
    format: *const windows::Win32::Media::Audio::WAVEFORMATEX,
) -> Result<DeviceFormat, AudioError> {
    let (tag, bits, channels, sample_rate, block_align, extra) = unsafe {
        (
            (*format).wFormatTag,
            (*format).wBitsPerSample,
            (*format).nChannels,
            (*format).nSamplesPerSec,
            (*format).nBlockAlign,
            (*format).cbSize,
        )
    };
    let sample = match tag {
        WAVE_FORMAT_PCM => pcm_sample(bits),
        WAVE_FORMAT_IEEE_FLOAT => float_sample(bits),
        WAVE_FORMAT_EXTENSIBLE if extra >= 22 => {
            // SAFETY: cbSize reports at least the extra bytes that make a WAVEFORMATEXTENSIBLE.
            let subformat = unsafe {
                (*(format.cast::<windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE>())).SubFormat
            };
            if subformat == KSDATAFORMAT_SUBTYPE_PCM {
                pcm_sample(bits)
            } else if subformat == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
                float_sample(bits)
            } else {
                None
            }
        }
        _ => None,
    }
    .ok_or(AudioError::UnsupportedMixFormat)?;
    if channels == 0 || sample_rate == 0 || block_align as usize < usize::from(channels) * sample.bytes()
    {
        return Err(AudioError::UnsupportedMixFormat);
    }
    Ok(DeviceFormat {
        channels,
        sample_rate,
        sample,
        block_align: block_align as usize,
    })
}

const fn pcm_sample(bits: u16) -> Option<InputSample> {
    match bits {
        16 => Some(InputSample::I16),
        24 => Some(InputSample::I24),
        32 => Some(InputSample::I32),
        _ => None,
    }
}

const fn float_sample(bits: u16) -> Option<InputSample> {
    match bits {
        32 => Some(InputSample::F32),
        _ => None,
    }
}

impl DeviceFormat {
    fn sample(&self, frame: &[u8], channel: usize) -> f32 {
        let offset = channel * self.sample.bytes();
        let Some(sample) = frame.get(offset..offset + self.sample.bytes()) else {
            return 0.0;
        };
        match self.sample {
            InputSample::F32 => sample
                .get(..4)
                .and_then(|bytes| bytes.try_into().ok())
                .map(f32::from_le_bytes)
                .unwrap_or(0.0),
            InputSample::I16 => sample
                .get(..2)
                .and_then(|bytes| bytes.try_into().ok())
                .map(|bytes| f32::from(i16::from_le_bytes(bytes)) / 32_768.0)
                .unwrap_or(0.0),
            InputSample::I24 => sample
                .get(..3)
                .map(|bytes| {
                    let value = i32::from(bytes[0])
                        | (i32::from(bytes[1]) << 8)
                        | (i32::from(bytes[2] as i8) << 16);
                    f32::from(value as i16) / 32_768.0
                })
                .unwrap_or(0.0),
            InputSample::I32 => sample
                .get(..4)
                .and_then(|bytes| bytes.try_into().ok())
                .map(|bytes| (i32::from_le_bytes(bytes) as f32) / 2_147_483_648.0)
                .unwrap_or(0.0),
        }
    }
}

struct Converter {
    input: DeviceFormat,
    output: PcmFormat,
    position: f64,
}

impl Converter {
    fn new(input: DeviceFormat, output: PcmFormat) -> Self {
        Self {
            input,
            output,
            position: 0.0,
        }
    }

    fn step(&self) -> f64 {
        f64::from(self.input.sample_rate) / f64::from(self.output.sample_rate)
    }

    fn convert(&mut self, bytes: &[u8], out: &mut Vec<u8>) {
        let step = self.step();
        for frame in bytes.chunks_exact(self.input.block_align) {
            let values = self.frame_values(frame);
            self.position += 1.0;
            while self.position >= step {
                self.position -= step;
                self.emit(&values, out);
            }
        }
    }

    fn silence(&mut self, frames: usize, out: &mut Vec<u8>) {
        let step = self.step();
        self.position += frames as f64;
        while self.position >= step {
            self.position -= step;
            self.emit(&[0.0, 0.0], out);
        }
    }

    fn frame_values(&self, frame: &[u8]) -> [f32; 2] {
        let channels = usize::from(self.input.channels);
        if channels == 1 {
            let value = self.input.sample(frame, 0);
            return [value, value];
        }
        if self.output.channels == 1 {
            let mut sum = 0.0;
            for channel in 0..channels {
                sum += self.input.sample(frame, channel);
            }
            let value = sum / channels as f32;
            return [value, value];
        }
        [self.input.sample(frame, 0), self.input.sample(frame, 1)]
    }

    fn emit(&self, values: &[f32; 2], out: &mut Vec<u8>) {
        let value = if self.output.channels == 1 {
            (values[0] + values[1]) * 0.5
        } else {
            values[0]
        };
        push_sample(out, self.output.sample_format, value);
        if self.output.channels > 1 {
            push_sample(out, self.output.sample_format, values[1]);
        }
    }
}

fn sample_bytes(format: PcmSampleFormat) -> usize {
    match format {
        PcmSampleFormat::S16Le => 2,
        PcmSampleFormat::S32Le | PcmSampleFormat::F32Le => 4,
    }
}

fn push_sample(out: &mut Vec<u8>, format: PcmSampleFormat, value: f32) {
    match format {
        PcmSampleFormat::S16Le => {
            out.extend_from_slice(&((value.clamp(-1.0, 1.0) * 32_767.0).round() as i16).to_le_bytes());
        }
        PcmSampleFormat::S32Le => {
            out.extend_from_slice(
                &((value.clamp(-1.0, 1.0) * 2_147_483_647.0).round() as i32).to_le_bytes(),
            );
        }
        PcmSampleFormat::F32Le => {
            out.extend_from_slice(&value.clamp(-1.0, 1.0).to_le_bytes());
        }
    }
}

#[cfg(test)]
fn pad_to_frame(bytes: &mut Vec<u8>, format: PcmFormat) {
    let remainder = bytes.len() % format.frame_size();
    if remainder != 0 {
        bytes.resize(bytes.len() + format.frame_size() - remainder, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn device_format(channels: u16, sample_rate: u32) -> DeviceFormat {
        DeviceFormat {
            channels,
            sample_rate,
            sample: InputSample::I16,
            block_align: usize::from(channels) * 2,
        }
    }

    #[test]
    fn pcm_defaults_match_the_ffmpeg_arguments() {
        let format = PcmFormat::default();
        assert_eq!(format.sample_format, PcmSampleFormat::S16Le);
        assert_eq!(format.sample_rate, 48_000);
        assert_eq!(format.channels, 2);
        assert_eq!(format.bytes_per_sample(), 2);
        assert_eq!(format.frame_size(), 4);
        assert_eq!(format.chunk_frames(), 960);
        assert_eq!(format.chunk_bytes(), 3_840);
        assert_eq!(format.chunk_bytes() % format.frame_size(), 0);
    }

    #[test]
    fn silence_is_zeroed_and_frame_aligned() {
        let format = PcmFormat::default();
        let silence = format.silence(480);
        assert_eq!(silence.len(), 1_920);
        assert!(silence.iter().all(|byte| *byte == 0));
        for sample_format in [
            PcmSampleFormat::S16Le,
            PcmSampleFormat::S32Le,
            PcmSampleFormat::F32Le,
        ] {
            let format = PcmFormat {
                sample_format,
                ..PcmFormat::default()
            };
            let silence = format.silence(25);
            assert_eq!(silence.len() % format.frame_size(), 0);
        }
    }

    #[test]
    fn chunks_are_padded_to_whole_frames() {
        let format = PcmFormat::default();
        let mut chunk = vec![1, 2, 3, 4, 5];
        pad_to_frame(&mut chunk, format);
        assert_eq!(chunk.len(), 8);
        assert_eq!(chunk.len() % format.frame_size(), 0);
        let mut aligned = vec![1, 2, 3, 4];
        pad_to_frame(&mut aligned, format);
        assert_eq!(aligned, [1, 2, 3, 4]);
    }

    #[test]
    fn pipe_indices_are_unique_across_sessions() {
        let first = next_pipe_index();
        let second = next_pipe_index();
        assert_ne!(first, second);
        let path = audio_pipe_path(first);
        assert!(path.starts_with(AUDIO_PIPE_PREFIX));
        assert_eq!(path, format!("{AUDIO_PIPE_PREFIX}{first}"));
        assert_eq!(audio_pipe_path(second), format!("{AUDIO_PIPE_PREFIX}{second}"));
        let mut seen = HashSet::new();
        for _ in 0..32 {
            assert!(seen.insert(next_pipe_index()));
        }
    }

    #[test]
    fn request_validation_rejects_empty_and_bad_formats() {
        assert!(matches!(
            AudioRequest {
                system: false,
                microphone: false,
                ..AudioRequest::default()
            }
            .validate(),
            Err(AudioError::NoStreams)
        ));
        for (channels, sample_rate) in [(0, 48_000), (3, 48_000), (2, 0)] {
            assert!(matches!(
                AudioRequest {
                    system: true,
                    format: PcmFormat {
                        channels,
                        sample_rate,
                        ..PcmFormat::default()
                    },
                    ..AudioRequest::default()
                }
                .validate(),
                Err(AudioError::InvalidFormat)
            ));
        }
        assert!(AudioRequest::default().validate().is_ok());
    }

    #[test]
    fn resampling_produces_the_expected_frame_count() {
        let input = device_format(2, 44_100);
        let mut converter = Converter::new(input, PcmFormat::default());
        let mut bytes = vec![0_u8; 4_410 * input.block_align];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = (index % 251) as u8;
        }
        let mut out = Vec::new();
        converter.convert(&bytes, &mut out);
        let frames = out.len() / PcmFormat::default().frame_size();
        assert!(frames.abs_diff(4_800) <= 1, "{frames}");
        assert_eq!(out.len() % PcmFormat::default().frame_size(), 0);
    }

    #[test]
    fn resampling_preserves_a_48_khz_stream() {
        let input = device_format(2, 48_000);
        let mut converter = Converter::new(input, PcmFormat::default());
        let bytes = vec![0_u8; 480 * input.block_align];
        let mut out = Vec::new();
        converter.convert(&bytes, &mut out);
        assert_eq!(out.len(), 480 * PcmFormat::default().frame_size());
    }

    #[test]
    fn mono_is_duplicated_and_stereo_is_preserved() {
        let mut converter = Converter::new(device_format(1, 48_000), PcmFormat::default());
        let mut left = Vec::new();
        converter.convert(&[0x00, 0x40], &mut left);
        assert_eq!(left.len(), 4);
        assert_eq!(i16::from_le_bytes([left[0], left[1]]), 16_384);
        assert_eq!(i16::from_le_bytes([left[2], left[3]]), 16_384);

        let mut converter = Converter::new(device_format(2, 48_000), PcmFormat::default());
        let mut stereo = Vec::new();
        converter.convert(&[0x00, 0x40, 0x00, 0xC0], &mut stereo);
        assert_eq!(i16::from_le_bytes([stereo[0], stereo[1]]), 16_384);
        assert_eq!(i16::from_le_bytes([stereo[2], stereo[3]]), -16_384);
    }

    #[test]
    fn output_sample_formats_convert_from_float() {
        let mut converter = Converter::new(device_format(1, 48_000), PcmFormat {
            sample_format: PcmSampleFormat::F32Le,
            ..PcmFormat::default()
        });
        let mut out = Vec::new();
        converter.convert(&[0x00, 0x40], &mut out);
        assert_eq!(out.len(), 8);
        let value = f32::from_le_bytes([out[0], out[1], out[2], out[3]]);
        assert!((value - 0.5).abs() < 0.001);

        let mut converter = Converter::new(device_format(1, 48_000), PcmFormat {
            sample_format: PcmSampleFormat::S32Le,
            ..PcmFormat::default()
        });
        let mut out = Vec::new();
        converter.convert(&[0x00, 0x40], &mut out);
        assert_eq!(out.len(), 8);
        let value = i32::from_le_bytes([out[0], out[1], out[2], out[3]]);
        assert!(value > 1_000_000_000);
    }

    #[test]
    fn silence_produces_output_frames() {
        let mut converter = Converter::new(device_format(2, 44_100), PcmFormat::default());
        let mut out = Vec::new();
        converter.silence(4_410, &mut out);
        let frames = out.len() / PcmFormat::default().frame_size();
        assert!(frames.abs_diff(4_800) <= 1, "{frames}");
    }

    #[test]
    fn friendly_names_are_composed_like_windows() {
        assert_eq!(
            compose_name(Some("Microphone"), Some("Logitech G432 Gaming Headset")),
            Some("Microphone (Logitech G432 Gaming Headset)".to_owned())
        );
        assert_eq!(
            compose_name(Some("Microphone"), Some("microphone")),
            Some("Microphone".to_owned())
        );
        assert_eq!(compose_name(None, Some("Headset")), Some("Headset".to_owned()));
        assert_eq!(compose_name(Some(""), Some("")), None);
    }

    #[test]
    fn errors_map_to_readable_messages() {
        assert_eq!(
            AudioError::NoStreams.to_string(),
            "audio capture needs at least one source"
        );
        assert_eq!(
            AudioError::NoDefaultDevice { device: "render" }.to_string(),
            "no default render device is available"
        );
        assert!(AudioError::DeviceNotFound {
            id: "device-7".to_owned()
        }
        .to_string()
        .contains("device-7"));
        assert!(AudioError::Capture {
            source: AudioSource::System,
            message: "the device was removed".to_owned()
        }
        .to_string()
        .contains("the device was removed"));
        assert!(AudioError::Pipe {
            pipe: audio_pipe_path(0),
            message: "creation failed (5)".to_owned()
        }
        .to_string()
        .contains("snapline-audio-0"));
    }

    #[test]
    fn disconnect_errors_are_recognised() {
        assert!(is_disconnect(ERROR_BROKEN_PIPE));
        assert!(is_disconnect(ERROR_NO_DATA));
        assert!(is_disconnect(ERROR_PIPE_NOT_CONNECTED));
        assert!(!is_disconnect(ERROR_OPERATION_ABORTED));
        assert!(!is_disconnect(ERROR_PIPE_CONNECTED));
    }

    #[test]
    fn stop_can_be_called_repeatedly() {
        let running = RunningAudio {
            pipes: Vec::new(),
            workers: Mutex::new(Vec::new()),
            stop: Arc::new(AtomicBool::new(false)),
        };
        running.stop();
        running.stop();
        assert!(running.pipe_names().is_empty());
        assert!(running.pipes().is_empty());
    }

    #[test]
    fn pipe_names_follow_the_core_prefix() {
        let running = RunningAudio {
            pipes: vec![
                AudioPipe {
                    source: AudioSource::System,
                    index: 41,
                    sample_format: PcmSampleFormat::S16Le,
                    sample_rate: 48_000,
                    channels: 2,
                },
                AudioPipe {
                    source: AudioSource::Microphone,
                    index: 42,
                    sample_format: PcmSampleFormat::S16Le,
                    sample_rate: 48_000,
                    channels: 2,
                },
            ],
            workers: Mutex::new(Vec::new()),
            stop: Arc::new(AtomicBool::new(false)),
        };
        assert_eq!(
            running.pipe_names(),
            [
                r"\\.\pipe\snapline-audio-41".to_owned(),
                r"\\.\pipe\snapline-audio-42".to_owned()
            ]
        );
        assert_eq!(running.pipes().len(), 2);
        assert_eq!(running.pipes()[1].source, AudioSource::Microphone);
    }

    #[test]
    #[ignore = "desktop"]
    fn device_list_has_usable_shape() {
        let devices = microphones();
        let mut ids = HashSet::new();
        let mut defaults = 0;
        for device in &devices {
            assert!(!device.id.is_empty());
            assert!(!device.name.is_empty());
            assert!(ids.insert(device.id.clone()), "duplicate device id");
            if device.default {
                defaults += 1;
            }
        }
        assert!(defaults <= 1);
        if let Some(default) = default_microphone() {
            assert!(default.default);
            assert!(devices.iter().any(|device| device.id == default.id));
        }
    }
}
