#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::fmt;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use windows::core::BOOL;
use windows::Win32::Foundation::{
    CloseHandle, FILETIME, HANDLE, HGLOBAL, HWND, LPARAM, RECT, UNICODE_STRING, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, Toolhelp32ReadProcessMemory,
    PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, QueryWorkingSet, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
    PSAPI_WORKING_SET_BLOCK, PSAPI_WORKING_SET_INFORMATION,
};
use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_VM_READ,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
};

const MIB: u64 = 1024 * 1024;
const IDLE_LIMIT: u64 = 15 * MIB;
const IDLE_DRIFT_LIMIT: u64 = 3 * MIB;
const MAIN_LIMIT: u64 = 120 * MIB;
const OVERLAY_LIMIT: u64 = 40 * MIB;
const STARTUP_WAIT: Duration = Duration::from_secs(30);
const CPU_WINDOW: Duration = Duration::from_secs(10);
const LINGER_WAIT: Duration = Duration::from_secs(25);
const WINDOW_WAIT: Duration = Duration::from_secs(20);
const CLOSE_WAIT: Duration = Duration::from_secs(10);
const EXIT_WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(250);
const ZERO_PERCENT_LIMIT: f64 = 1.0;
const OVERLAY_TIMING_LIMIT: Duration = Duration::from_millis(150);
const MAIN_COLD_LIMIT: Duration = Duration::from_millis(800);
const OVERLAY_WAIT: Duration = Duration::from_secs(10);
const OVERLAY_SETTLE: Duration = Duration::from_secs(1);
const CAPTURE_WAIT: Duration = Duration::from_secs(120);
const LINK_WAIT: Duration = Duration::from_secs(60);
const RECORD_MEMORY_LIMIT: u64 = 150 * MIB;
const RECORD_CPU_LIMIT: f64 = 5.0;
const RECORD_START_WAIT: Duration = Duration::from_secs(120);
const RECORD_STOP_WAIT: Duration = Duration::from_secs(30);
const RECORD_STABLE: Duration = Duration::from_secs(2);
const RECORD_PARSE_WAIT: Duration = Duration::from_secs(5);
const WEBVIEW_PROCESS: &str = "msedgewebview2.exe";
const FFMPEG_PROCESS: &str = "ffmpeg.exe";
const SNAPLINE_PROCESS: &str = "snapline.exe";
const COMMAND_LINE_INFORMATION: u32 = 60;
pub const RECORD_INSTRUCTION: &str =
    "Aaron must select the 1920 x 1080 region when the overlay appears.";

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQueryInformationProcess(
        process_handle: *mut c_void,
        process_information_class: u32,
        process_information: *mut c_void,
        process_information_length: u32,
        return_length: *mut u32,
    ) -> i32;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    Under(u64),
    Within { reference: u64, allowance: u64 },
}

impl Limit {
    #[must_use]
    pub fn allows(self, measured: u64) -> bool {
        match self {
            Self::Under(limit) => measured < limit,
            Self::Within {
                reference,
                allowance,
            } => measured <= reference.saturating_add(allowance),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetRow {
    TrayIdleBefore,
    MainWindow,
    TrayIdleAfter,
    CaptureToLink,
    Recording,
}

impl BudgetRow {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::TrayIdleBefore => "tray idle before any window",
            Self::MainWindow => "main window open",
            Self::TrayIdleAfter => "tray idle after every window",
            Self::CaptureToLink => "capture-to-link run",
            Self::Recording => "recording 1080p30",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FailureKind {
    MissingSample,
    OverMemory {
        measured: u64,
        limit: u64,
    },
    IdleDrift {
        measured: u64,
        reference: u64,
        allowance: u64,
    },
    CpuOver {
        measured: f64,
        limit: f64,
    },
    AppLeak {
        count: usize,
    },
    WebviewLeak {
        count: usize,
    },
    Slow {
        path: &'static str,
        measured: Duration,
        limit: Duration,
    },
    OverlayMissing {
        waited: Duration,
    },
    CaptureUnfinished {
        waited: Duration,
    },
    LinkMissing,
    Unreadable {
        count: usize,
    },
    Recording(RecordingFailure),
}

impl fmt::Display for FailureKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSample => formatter.write_str("the measurement is missing."),
            Self::OverMemory { measured, limit } => {
                if measured > limit {
                    write!(
                        formatter,
                        "{} is over the {} budget by {}.",
                        format_bytes(*measured),
                        format_bytes(*limit),
                        format_bytes(measured.saturating_sub(*limit))
                    )
                } else {
                    write!(
                        formatter,
                        "{} is not under the {} budget.",
                        format_bytes(*measured),
                        format_bytes(*limit)
                    )
                }
            }
            Self::IdleDrift {
                measured,
                reference,
                allowance,
            } => write!(
                formatter,
                "{} is {} above the first idle figure ({}); the allowance is {}.",
                format_bytes(*measured),
                format_bytes(measured.saturating_sub(*reference)),
                format_bytes(*reference),
                format_bytes(*allowance)
            ),
            Self::CpuOver { measured, limit } => write!(
                formatter,
                "CPU {measured:.1}% is not under the {limit:.1}% budget."
            ),
            Self::AppLeak { count } => write!(
                formatter,
                "{count} Snapline process(es) were still running after the budget run; the budget requires none."
            ),
            Self::WebviewLeak { count } => {
                if *count == 1 {
                    formatter.write_str(
                        "1 WebView2 process is still running; the budget requires none.",
                    )
                } else {
                    write!(
                        formatter,
                        "{count} WebView2 processes are still running; the budget requires none."
                    )
                }
            }
            Self::Slow {
                path,
                measured,
                limit,
            } => write!(
                formatter,
                "the {path} took {} ms; the budget is under {} ms.",
                measured.as_millis(),
                limit.as_millis()
            ),
            Self::OverlayMissing { waited } => write!(
                formatter,
                "the region overlay did not appear within {} s.",
                waited.as_secs()
            ),
            Self::CaptureUnfinished { waited } => write!(
                formatter,
                "the capture-to-link run did not finish within {} s.",
                waited.as_secs()
            ),
            Self::LinkMissing => formatter.write_str(
                "no link reached the clipboard; make sure the default workflow has a working upload destination and do not cancel the region overlay.",
            ),
            Self::Unreadable { count } => write!(
                formatter,
                "{count} process memory reading(s) were unavailable, so the total is not trustworthy."
            ),
            Self::Recording(error) => write!(formatter, "{error} {RECORD_INSTRUCTION}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BudgetFailure {
    pub row: BudgetRow,
    pub kind: FailureKind,
}

impl BudgetFailure {
    #[must_use]
    pub fn new(row: BudgetRow, kind: FailureKind) -> Self {
        Self { row, kind }
    }
}

impl fmt::Display for BudgetFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.row.label(), self.kind)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowOutcome {
    pub row: BudgetRow,
    pub measured: bool,
    pub failures: Vec<BudgetFailure>,
}

impl RowOutcome {
    #[must_use]
    pub fn missing(row: BudgetRow) -> Self {
        Self {
            row,
            measured: false,
            failures: vec![BudgetFailure::new(row, FailureKind::MissingSample)],
        }
    }

    #[must_use]
    pub fn measured(row: BudgetRow) -> Self {
        Self {
            row,
            measured: true,
            failures: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_failures(row: BudgetRow, failures: Vec<BudgetFailure>) -> Self {
        Self {
            row,
            measured: true,
            failures,
        }
    }

    pub fn push(&mut self, kind: FailureKind) {
        self.failures.push(BudgetFailure::new(self.row, kind));
    }

    #[must_use]
    pub fn passed(&self) -> bool {
        self.measured && self.failures.is_empty()
    }

    #[must_use]
    pub fn summary_line(&self) -> String {
        if !self.measured {
            return "the measurement is missing.".to_owned();
        }
        match self.failures.first() {
            Some(failure) => failure.kind.to_string(),
            None => "ok".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BudgetSummary {
    pub rows: Vec<RowOutcome>,
}

impl BudgetSummary {
    #[must_use]
    pub fn new(rows: Vec<RowOutcome>) -> Self {
        Self { rows }
    }

    #[must_use]
    pub fn passed_rows(&self) -> usize {
        self.rows.iter().filter(|row| row.passed()).count()
    }

    #[must_use]
    pub fn passed(&self) -> bool {
        self.rows.iter().all(RowOutcome::passed)
    }

    #[must_use]
    pub fn first_failure(&self) -> Option<&RowOutcome> {
        self.rows.iter().find(|row| !row.passed())
    }

    #[must_use]
    pub fn render(&self) -> String {
        let mut text = format!(
            "budget summary: {} of {} rows passed.",
            self.passed_rows(),
            self.rows.len()
        );
        for row in &self.rows {
            text.push_str(&format!(
                "\n  {} {}: {}",
                if row.passed() { "PASS" } else { "FAIL" },
                row.row.label(),
                row.summary_line()
            ));
            if row.row == BudgetRow::Recording && !row.passed() {
                text.push(' ');
                text.push_str(RECORD_INSTRUCTION);
            }
        }
        match self.first_failure() {
            Some(row) => text.push_str(&format!(
                "\nbudget: first failing row: {}.",
                row.row.label()
            )),
            None => text.push_str("\nbudget: every row passed."),
        }
        text
    }
}

fn print_summary(summary: &BudgetSummary) {
    println!("{}", summary.render());
}

fn summary_error(summary: &BudgetSummary) -> anyhow::Error {
    let count = summary.rows.len() - summary.passed_rows();
    let first = summary
        .first_failure()
        .map_or_else(|| "none".to_owned(), |row| row.row.label().to_owned());
    let recording_failed = summary
        .rows
        .iter()
        .any(|row| row.row == BudgetRow::Recording && !row.passed());
    if recording_failed {
        anyhow::anyhow!(
            "{count} of {} budget rows failed; first failing row: {first}. {RECORD_INSTRUCTION}",
            summary.rows.len()
        )
    } else {
        anyhow::anyhow!(
            "{count} of {} budget rows failed; first failing row: {first}",
            summary.rows.len()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessRow {
    pub pid: u32,
    pub parent: u32,
    pub name: String,
    pub created: Option<u64>,
    pub command_line: Option<String>,
}

impl ProcessRow {
    fn new(pid: u32, parent: u32, name: &str) -> Self {
        Self {
            pid,
            parent,
            name: name.to_string(),
            created: None,
            command_line: None,
        }
    }
}

fn lookup(rows: &[ProcessRow]) -> HashMap<u32, usize> {
    rows.iter()
        .enumerate()
        .map(|(index, row)| (row.pid, index))
        .collect()
}

fn chain_to_root(
    rows: &[ProcessRow],
    lookup: &HashMap<u32, usize>,
    start: usize,
    root: u32,
    root_created: u64,
) -> Option<Vec<usize>> {
    let mut index = start;
    let mut chain = Vec::new();
    let mut visited: Vec<u32> = Vec::new();
    loop {
        let row = rows.get(index)?;
        if row.pid == root {
            return Some(chain);
        }
        if visited.contains(&row.pid) {
            return None;
        }
        visited.push(row.pid);
        if row.created.is_some_and(|created| created < root_created) {
            return None;
        }
        chain.push(index);
        index = *lookup.get(&row.parent)?;
    }
}

#[must_use]
pub fn descendants(rows: &[ProcessRow], root: u32, root_created: u64) -> Vec<u32> {
    let lookup = lookup(rows);
    rows.iter()
        .enumerate()
        .filter(|(index, row)| {
            row.pid != root && chain_to_root(rows, &lookup, *index, root, root_created).is_some()
        })
        .map(|(_, row)| row.pid)
        .collect()
}

fn has_data_directory(row: &ProcessRow, needle: &str) -> bool {
    row.command_line
        .as_ref()
        .is_some_and(|line| line.to_lowercase().contains(needle))
}

#[must_use]
pub fn webview_children(
    rows: &[ProcessRow],
    root: u32,
    root_created: u64,
    data_directory: &str,
) -> Vec<u32> {
    let lookup = lookup(rows);
    let needle = data_directory.to_lowercase();
    let mut children = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        if row.pid == root || !row.name.eq_ignore_ascii_case(WEBVIEW_PROCESS) {
            continue;
        }
        let Some(chain) = chain_to_root(rows, &lookup, index, root, root_created) else {
            continue;
        };
        let members: Vec<usize> = std::iter::once(index).chain(chain).collect();
        if members
            .iter()
            .any(|member| has_data_directory(&rows[*member], &needle))
        {
            children.push(row.pid);
            continue;
        }
        if members
            .iter()
            .all(|member| rows[*member].command_line.is_none())
        {
            children.push(row.pid);
        }
    }
    children
}

struct ProcessHandle(HANDLE);

impl ProcessHandle {
    fn handle(&self) -> HANDLE {
        self.0
    }

    fn raw(&self) -> *mut c_void {
        self.0 .0
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        // SAFETY: this value owns the handle and closes it exactly once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn open_process(pid: u32) -> Option<ProcessHandle> {
    let full = PROCESS_QUERY_INFORMATION | PROCESS_VM_READ;
    let limited = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ;
    // SAFETY: OpenProcess has no pointer arguments and the pid comes from the system.
    let handle = match unsafe { OpenProcess(full, false, pid) } {
        Ok(handle) => Ok(handle),
        Err(_) => {
            // SAFETY: the same call with a narrower access mask for sandboxed children.
            unsafe { OpenProcess(limited, false, pid) }
        }
    };
    match handle {
        Ok(handle) => Some(ProcessHandle(handle)),
        Err(_) => None,
    }
}

fn ticks(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}

fn process_created(pid: u32) -> Option<u64> {
    let handle = open_process(pid)?;
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the handle is live and every out pointer refers to a writable FILETIME.
    unsafe {
        GetProcessTimes(
            handle.handle(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    }
    .ok()?;
    Some(ticks(creation))
}

fn process_cpu_seconds(pid: u32) -> Option<f64> {
    let handle = open_process(pid)?;
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    // SAFETY: the handle is live and every out pointer refers to a writable FILETIME.
    unsafe {
        GetProcessTimes(
            handle.handle(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    }
    .ok()?;
    Some((ticks(kernel) + ticks(user)) as f64 / 10_000_000.0)
}

fn page_size() -> u64 {
    let mut info = SYSTEM_INFO::default();
    // SAFETY: GetSystemInfo only writes to the supplied structure.
    unsafe { GetSystemInfo(&mut info) };
    u64::from(info.dwPageSize.max(4096))
}

fn working_set_without_shared_pages(handle: &ProcessHandle) -> Option<u64> {
    let page = page_size();
    let mut bytes = 64 * 1024;
    loop {
        let slots = bytes / size_of::<usize>();
        let mut buffer = vec![0usize; slots];
        // SAFETY: the buffer is writable and exactly bytes long.
        let result = unsafe {
            QueryWorkingSet(
                handle.handle(),
                buffer.as_mut_ptr().cast::<c_void>(),
                bytes as u32,
            )
        };
        if result.is_ok() {
            // SAFETY: on success QueryWorkingSet filled the buffer with the documented structure.
            let info = unsafe { &*buffer.as_ptr().cast::<PSAPI_WORKING_SET_INFORMATION>() };
            let entries = unsafe { buffer.as_ptr().add(1).cast::<PSAPI_WORKING_SET_BLOCK>() };
            let count = info.NumberOfEntries.min(slots.saturating_sub(1));
            let mut total = 0u64;
            for index in 0..count {
                // SAFETY: index is bounded by the entry count and the allocation.
                let block = unsafe { *entries.add(index) };
                // SAFETY: reading the documented bitfield union is valid for any bit pattern.
                let bits = unsafe { block.Anonymous._bitfield };
                if bits & (1usize << 19) == 0 {
                    total += page;
                }
            }
            return Some(total);
        }
        if bytes >= 64 * 1024 * 1024 {
            return None;
        }
        bytes *= 2;
    }
}

fn private_working_set(pid: u32) -> Option<u64> {
    let handle = open_process(pid)?;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2::default();
    counters.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
    // SAFETY: the counters structure is writable and cb matches its size.
    let result = unsafe {
        GetProcessMemoryInfo(
            handle.handle(),
            std::ptr::from_mut(&mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    if result.is_ok() && counters.PrivateWorkingSetSize > 0 {
        return Some(counters.PrivateWorkingSetSize as u64);
    }
    working_set_without_shared_pages(&handle)
}

fn command_line(pid: u32) -> Option<String> {
    let handle = open_process(pid)?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut returned = 0u32;
    // SAFETY: the buffer is writable for its full length and the handle is live.
    let status = unsafe {
        NtQueryInformationProcess(
            handle.raw(),
            COMMAND_LINE_INFORMATION,
            buffer.as_mut_ptr().cast::<c_void>(),
            buffer.len() as u32,
            &mut returned,
        )
    };
    if status < 0 {
        return None;
    }
    // SAFETY: a successful query writes a UNICODE_STRING at the start of the buffer.
    let text = unsafe { std::ptr::read_unaligned(buffer.as_ptr().cast::<UNICODE_STRING>()) };
    let length = usize::from(text.Length);
    if length == 0 || length % 2 != 0 {
        return None;
    }
    let mut wide = vec![0u16; length / 2];
    let pointer = text.Buffer.0.cast::<c_void>();
    let start = buffer.as_ptr() as usize;
    let end = start + buffer.len();
    let address = pointer as usize;
    if address >= start && address < end {
        let offset = address - start;
        if offset + length > buffer.len() {
            return None;
        }
        for (index, slot) in wide.iter_mut().enumerate() {
            let at = offset + index * 2;
            *slot = u16::from_le_bytes([buffer[at], buffer[at + 1]]);
        }
    } else {
        let mut read = 0usize;
        // SAFETY: the source is inside the target process and the destination is writable.
        let ok = unsafe {
            Toolhelp32ReadProcessMemory(
                pid,
                pointer,
                wide.as_mut_ptr().cast::<c_void>(),
                length,
                &mut read,
            )
        };
        if !ok.as_bool() || read != length {
            return None;
        }
    }
    while wide.last() == Some(&0) {
        wide.pop();
    }
    Some(String::from_utf16_lossy(&wide))
}

fn raw_processes() -> Result<Vec<ProcessRow>> {
    // SAFETY: the snapshot is owned by this function and closed before it returns.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .context("could not take a process snapshot")?;
    let guard = ProcessHandle(snapshot);
    let mut entry = PROCESSENTRY32W::default();
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut rows = Vec::new();
    // SAFETY: entry has the documented size and the snapshot is live.
    let mut more = unsafe { Process32FirstW(guard.0, &mut entry) }.is_ok();
    while more {
        let end = entry
            .szExeFile
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..end]);
        rows.push(ProcessRow::new(
            entry.th32ProcessID,
            entry.th32ParentProcessID,
            &name,
        ));
        // SAFETY: entry keeps the documented size and the snapshot is live.
        more = unsafe { Process32NextW(guard.0, &mut entry) }.is_ok();
    }
    Ok(rows)
}

fn scan_processes(root: u32) -> Result<Vec<ProcessRow>> {
    let mut rows = raw_processes()?;
    let reachable: Vec<u32> = descendants(&rows, root, 0);
    for row in &mut rows {
        if row.pid != root && !reachable.contains(&row.pid) {
            continue;
        }
        row.created = process_created(row.pid);
        if row.pid != root && row.name.eq_ignore_ascii_case(WEBVIEW_PROCESS) {
            row.command_line = command_line(row.pid);
        }
    }
    Ok(rows)
}

fn cpu_by_pid(rows: &[ProcessRow], root: u32, root_created: u64) -> HashMap<u32, f64> {
    let mut values = HashMap::new();
    values.insert(root, process_cpu_seconds(root).unwrap_or(0.0));
    for pid in descendants(rows, root, root_created) {
        values.insert(pid, process_cpu_seconds(pid).unwrap_or(0.0));
    }
    values
}

fn total_cpu(values: &HashMap<u32, f64>) -> f64 {
    values.values().sum()
}

struct ChildSample {
    pid: u32,
    name: String,
    private: u64,
    cpu_percent: f64,
}

struct Sample {
    app_private: u64,
    total_private: u64,
    unreadable: usize,
    webview: usize,
    cpu_percent: f64,
    children: Vec<ChildSample>,
}

fn percent(delta: f64, elapsed: f64) -> f64 {
    if elapsed > 0.0 {
        (delta.max(0.0) / elapsed) * 100.0
    } else {
        0.0
    }
}

fn snapshot(root: u32, root_created: u64, data_directory: &str) -> Result<Sample> {
    let rows = scan_processes(root)?;
    let mut unreadable = 0usize;
    let app_private = match private_working_set(root) {
        Some(bytes) => bytes,
        None => {
            unreadable += 1;
            0
        }
    };
    let mut total_private = app_private;
    let mut children = Vec::new();
    for pid in descendants(&rows, root, root_created) {
        match private_working_set(pid) {
            Some(bytes) => {
                total_private += bytes;
                let name = rows
                    .iter()
                    .find(|row| row.pid == pid)
                    .map(|row| row.name.clone())
                    .unwrap_or_default();
                children.push(ChildSample {
                    pid,
                    name,
                    private: bytes,
                    cpu_percent: 0.0,
                });
            }
            None => unreadable += 1,
        }
    }
    let webview = webview_children(&rows, root, root_created, data_directory).len();
    Ok(Sample {
        app_private,
        total_private,
        unreadable,
        webview,
        cpu_percent: 0.0,
        children,
    })
}

fn collect(root: u32, root_created: u64, data_directory: &str) -> Result<Sample> {
    let before = scan_processes(root)?;
    let cpu_before = cpu_by_pid(&before, root, root_created);
    let clock = Instant::now();
    sleep(CPU_WINDOW);
    let elapsed = clock.elapsed().as_secs_f64();
    let after = scan_processes(root)?;
    let cpu_after = cpu_by_pid(&after, root, root_created);
    let mut sample = snapshot(root, root_created, data_directory)?;
    sample.cpu_percent = percent(total_cpu(&cpu_after) - total_cpu(&cpu_before), elapsed);
    for child in &mut sample.children {
        let child_before = cpu_before.get(&child.pid).copied().unwrap_or(0.0);
        let child_after = cpu_after.get(&child.pid).copied().unwrap_or(0.0);
        child.cpu_percent = percent(child_after - child_before, elapsed);
    }
    Ok(sample)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BudgetWindow {
    Main,
}

impl BudgetWindow {
    fn argument(self) -> &'static str {
        match self {
            Self::Main => "main",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Main => "main window",
        }
    }
}

struct AppProcess {
    child: Child,
    stopped: bool,
}

impl AppProcess {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn stop(&mut self, exe: &Path) {
        if self.stopped {
            return;
        }
        let alive = matches!(self.child.try_wait(), Ok(None));
        if alive {
            let mut exit = Command::new(exe)
                .arg("--action")
                .arg("app_exit")
                .spawn()
                .ok();
            let deadline = Instant::now() + EXIT_WAIT;
            while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
                sleep(POLL);
            }
            if let Some(exit) = exit.as_mut() {
                let _ = exit.try_wait();
            }
        }
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.stopped = true;
    }
}

impl Drop for AppProcess {
    fn drop(&mut self) {
        if !self.stopped {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.stopped = true;
        }
    }
}

struct WindowSearch {
    pid: u32,
    windows: Vec<HWND>,
}

unsafe extern "system" fn collect_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: EnumWindows passes back the pointer supplied to it in lparam.
    let search = unsafe { &mut *(lparam.0 as *mut WindowSearch) };
    // SAFETY: hwnd comes from the enumeration and is only queried.
    if unsafe { IsWindowVisible(hwnd) }.as_bool() {
        let mut pid = 0u32;
        // SAFETY: the out pointer refers to a local u32 and hwnd is a live window.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if pid == search.pid {
            search.windows.push(hwnd);
        }
    }
    BOOL(1)
}

fn visible_windows(pid: u32) -> Vec<HWND> {
    let mut search = WindowSearch {
        pid,
        windows: Vec::new(),
    };
    // SAFETY: the callback dereferences the pointer to search and EnumWindows returns it.
    let _ = unsafe {
        EnumWindows(
            Some(collect_window),
            LPARAM((&mut search as *mut WindowSearch) as isize),
        )
    };
    search.windows
}

fn wait_for_windows(pid: u32, expected: usize, window: BudgetWindow) -> Result<()> {
    let deadline = Instant::now() + WINDOW_WAIT;
    loop {
        if visible_windows(pid).len() >= expected {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "the {} did not appear within {} s",
                window.label(),
                WINDOW_WAIT.as_secs()
            );
        }
        sleep(POLL);
    }
}

fn wait_for_no_windows(pid: u32, label: &str, waited: Duration) -> Result<Duration> {
    let started = Instant::now();
    let deadline = started + waited;
    loop {
        if visible_windows(pid).is_empty() {
            return Ok(started.elapsed());
        }
        if Instant::now() >= deadline {
            bail!("the {label} did not close within {} s", waited.as_secs());
        }
        sleep(POLL);
    }
}

fn wait_for_visible_window(pid: u32, label: &str, waited: Duration) -> Result<Duration> {
    let started = Instant::now();
    let deadline = started + waited;
    loop {
        if !visible_windows(pid).is_empty() {
            return Ok(started.elapsed());
        }
        if Instant::now() >= deadline {
            bail!("the {label} did not appear within {} s", waited.as_secs());
        }
        sleep(POLL);
    }
}

fn close_windows(pid: u32) -> Result<()> {
    let windows = visible_windows(pid);
    for window in windows {
        // SAFETY: the handle belongs to a window of the Snapline process.
        let _ = unsafe { PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
    let deadline = Instant::now() + CLOSE_WAIT;
    loop {
        if visible_windows(pid).is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!(
                "a Snapline window did not close within {} s",
                CLOSE_WAIT.as_secs()
            );
        }
        sleep(POLL);
    }
}

fn open_window(exe: &Path, window: BudgetWindow, pid: u32, expected: usize) -> Result<Duration> {
    println!("budget: opening the {}", window.label());
    let started = Instant::now();
    Command::new(exe)
        .arg("--budget-open")
        .arg(window.argument())
        .spawn()
        .with_context(|| {
            format!(
                "could not start a second Snapline instance for the {}",
                window.label()
            )
        })?;
    wait_for_windows(pid, expected, window)?;
    Ok(started.elapsed())
}

fn format_bytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / MIB as f64)
}

fn report(label: &str, sample: &Sample) {
    println!(
        "budget: {label}: app {}, total {}, WebView2 {}, CPU {:.1}%",
        format_bytes(sample.app_private),
        format_bytes(sample.total_private),
        sample.webview,
        sample.cpu_percent
    );
    for child in &sample.children {
        println!(
            "    {} (pid {}) {}",
            child.name,
            child.pid,
            format_bytes(child.private)
        );
    }
    if sample.unreadable == 1 {
        println!("    warning: memory could not be read for 1 process.");
    } else if sample.unreadable > 1 {
        println!(
            "    warning: memory could not be read for {} processes.",
            sample.unreadable
        );
    }
}

fn check_under(row: BudgetRow, failures: &mut Vec<BudgetFailure>, measured: u64, limit: u64) {
    if Limit::Under(limit).allows(measured) {
        println!(
            "  PASS {}: {} is under the {} budget.",
            row.label(),
            format_bytes(measured),
            format_bytes(limit)
        );
        return;
    }
    let kind = FailureKind::OverMemory { measured, limit };
    println!("  FAIL {}: {kind}", row.label());
    failures.push(BudgetFailure::new(row, kind));
}

fn check_within(
    row: BudgetRow,
    failures: &mut Vec<BudgetFailure>,
    measured: u64,
    reference: u64,
    allowance: u64,
) {
    if (Limit::Within {
        reference,
        allowance,
    })
    .allows(measured)
    {
        println!(
            "  PASS {}: {} is within {} of the first idle figure ({}).",
            row.label(),
            format_bytes(measured),
            format_bytes(allowance),
            format_bytes(reference)
        );
        return;
    }
    let kind = FailureKind::IdleDrift {
        measured,
        reference,
        allowance,
    };
    println!("  FAIL {}: {kind}", row.label());
    failures.push(BudgetFailure::new(row, kind));
}

fn check_webview(row: BudgetRow, failures: &mut Vec<BudgetFailure>, count: usize) {
    if count == 0 {
        println!("  PASS {}: no WebView2 processes are running.", row.label());
        return;
    }
    let kind = FailureKind::WebviewLeak { count };
    println!("  FAIL {}: {kind}", row.label());
    failures.push(BudgetFailure::new(row, kind));
}

fn check_cpu(row: BudgetRow, failures: &mut Vec<BudgetFailure>, measured: f64, limit: f64) {
    if measured < limit {
        println!(
            "  PASS {}: CPU {measured:.1}% is under the {limit:.1}% budget.",
            row.label()
        );
        return;
    }
    let kind = FailureKind::CpuOver { measured, limit };
    println!("  FAIL {}: {kind}", row.label());
    failures.push(BudgetFailure::new(row, kind));
}

fn check_slow(
    row: BudgetRow,
    failures: &mut Vec<BudgetFailure>,
    path: &'static str,
    measured: Duration,
    limit: Duration,
) {
    if measured < limit {
        println!(
            "  PASS {}: the {path} took {} ms, under {} ms.",
            row.label(),
            measured.as_millis(),
            limit.as_millis()
        );
        return;
    }
    let kind = FailureKind::Slow {
        path,
        measured,
        limit,
    };
    println!("  FAIL {}: {kind}", row.label());
    failures.push(BudgetFailure::new(row, kind));
}

fn idle_row(
    row: BudgetRow,
    sample: Option<&Sample>,
    reference: Option<u64>,
    check_zero_cpu: bool,
) -> RowOutcome {
    let Some(sample) = sample else {
        println!("  FAIL {}: the measurement is missing.", row.label());
        return RowOutcome::missing(row);
    };
    report(row.label(), sample);
    let mut failures = Vec::new();
    check_under(row, &mut failures, sample.total_private, IDLE_LIMIT);
    check_webview(row, &mut failures, sample.webview);
    if check_zero_cpu {
        check_cpu(row, &mut failures, sample.cpu_percent, ZERO_PERCENT_LIMIT);
    }
    if let Some(first_idle) = reference {
        check_within(
            row,
            &mut failures,
            sample.total_private,
            first_idle,
            IDLE_DRIFT_LIMIT,
        );
    }
    RowOutcome::with_failures(row, failures)
}

fn window_row(
    row: BudgetRow,
    sample: Option<&Sample>,
    limit: u64,
    check_zero_cpu: bool,
) -> RowOutcome {
    let Some(sample) = sample else {
        println!("  FAIL {}: the measurement is missing.", row.label());
        return RowOutcome::missing(row);
    };
    report(row.label(), sample);
    let mut failures = Vec::new();
    check_under(row, &mut failures, sample.total_private, limit);
    if check_zero_cpu {
        check_cpu(row, &mut failures, sample.cpu_percent, ZERO_PERCENT_LIMIT);
    }
    RowOutcome::with_failures(row, failures)
}

fn overlay_limit(frozen_copy: u64) -> u64 {
    OVERLAY_LIMIT.saturating_add(frozen_copy)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordingPlan {
    pub seconds: u64,
    pub width: u32,
    pub height: u32,
    pub frame_rate: u32,
}

impl RecordingPlan {
    pub const DEFAULT: Self = Self {
        seconds: 60,
        width: 1920,
        height: 1080,
        frame_rate: 30,
    };

    #[must_use]
    pub fn sample_windows(&self) -> u32 {
        u32::try_from(self.seconds / CPU_WINDOW.as_secs()).unwrap_or(0)
    }

    #[must_use]
    pub fn samples_duration(&self) -> Duration {
        CPU_WINDOW * self.sample_windows()
    }
}

impl Default for RecordingPlan {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingCommand {
    pub encoder: String,
    pub width: u32,
    pub height: u32,
    pub frame_rate: u32,
}

impl RecordingCommand {
    #[must_use]
    pub fn hardware(&self) -> bool {
        encoder_is_hardware(&self.encoder)
    }

    #[must_use]
    pub fn matches(&self, plan: RecordingPlan) -> bool {
        self.width == plan.width && self.height == plan.height && self.frame_rate == plan.frame_rate
    }
}

#[must_use]
pub fn encoder_is_hardware(encoder: &str) -> bool {
    ["_nvenc", "_qsv", "_amf"]
        .iter()
        .any(|suffix| encoder.ends_with(suffix))
}

fn setting<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    let start = source.find(key)? + key.len();
    let rest = &source[start..];
    let end = rest.find(':').unwrap_or(rest.len());
    Some(&rest[..end])
}

fn number(source: &str, key: &str) -> Option<u32> {
    setting(source, key)?.parse().ok()
}

#[must_use]
pub fn parse_recording_command(line: &str) -> Option<RecordingCommand> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    let encoder = tokens
        .iter()
        .position(|token| *token == "-c:v")
        .and_then(|index| tokens.get(index + 1))
        .map(|value| (*value).to_owned())?;
    let source = *tokens.iter().find(|token| token.starts_with("ddagrab="))?;
    let size = setting(source, "video_size=")?;
    let (width, height) = size.split_once('x')?;
    let width = width.parse().ok()?;
    let height = height.parse().ok()?;
    let frame_rate = number(source, "framerate=")?;
    Some(RecordingCommand {
        encoder,
        width,
        height,
        frame_rate,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordingSample {
    pub app_private: u64,
    pub ffmpeg_private: u64,
    pub total_private: u64,
    pub unreadable: usize,
    pub cpu_percent: f64,
    pub ffmpeg_cpu_percent: f64,
}

impl RecordingSample {
    fn from_sample(sample: &Sample) -> Self {
        let mut ffmpeg_private = 0;
        let mut ffmpeg_cpu_percent = 0.0;
        for child in &sample.children {
            if child.name.eq_ignore_ascii_case(FFMPEG_PROCESS) {
                ffmpeg_private += child.private;
                ffmpeg_cpu_percent += child.cpu_percent;
            }
        }
        Self {
            app_private: sample.app_private,
            ffmpeg_private,
            total_private: sample.total_private,
            unreadable: sample.unreadable,
            cpu_percent: sample.cpu_percent,
            ffmpeg_cpu_percent,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordingReport {
    pub plan: RecordingPlan,
    pub command: RecordingCommand,
    pub samples: Vec<RecordingSample>,
}

impl RecordingReport {
    #[must_use]
    pub fn max_total_private(&self) -> u64 {
        self.samples
            .iter()
            .map(|sample| sample.total_private)
            .max()
            .unwrap_or(0)
    }

    #[must_use]
    pub fn max_app_private(&self) -> u64 {
        self.samples
            .iter()
            .map(|sample| sample.app_private)
            .max()
            .unwrap_or(0)
    }

    #[must_use]
    pub fn max_ffmpeg_private(&self) -> u64 {
        self.samples
            .iter()
            .map(|sample| sample.ffmpeg_private)
            .max()
            .unwrap_or(0)
    }

    #[must_use]
    pub fn max_cpu_percent(&self) -> f64 {
        self.samples
            .iter()
            .fold(0.0, |maximum, sample| maximum.max(sample.cpu_percent))
    }

    #[must_use]
    pub fn max_ffmpeg_cpu_percent(&self) -> f64 {
        self.samples.iter().fold(0.0, |maximum, sample| {
            maximum.max(sample.ffmpeg_cpu_percent)
        })
    }

    #[must_use]
    pub fn memory_passes(&self) -> bool {
        Limit::Under(RECORD_MEMORY_LIMIT).allows(self.max_total_private())
    }

    #[must_use]
    pub fn cpu_passes(&self) -> bool {
        self.max_cpu_percent() < RECORD_CPU_LIMIT
    }

    #[must_use]
    pub fn hardware(&self) -> bool {
        self.command.hardware()
    }

    #[must_use]
    pub fn has_samples(&self) -> bool {
        !self.samples.is_empty()
    }

    #[must_use]
    pub fn target_matches_plan(&self) -> bool {
        self.command.matches(self.plan)
    }

    #[must_use]
    pub fn passes(&self) -> bool {
        self.has_samples()
            && self.target_matches_plan()
            && self.hardware()
            && self.memory_passes()
            && self.cpu_passes()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingFailure {
    NeverStarted,
    EarlyExit {
        after_seconds: u64,
    },
    NeverStopped,
    UnreadableCommand,
    MissingSamples,
    AppExited {
        status: String,
    },
    WrongTarget {
        width: u32,
        height: u32,
        frame_rate: u32,
    },
    SoftwareEncoder {
        encoder: String,
    },
    WebviewOpen {
        count: usize,
    },
}

impl fmt::Display for RecordingFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NeverStarted => write!(
                formatter,
                "the recording never started: FFmpeg was not found under Snapline within {} s.",
                RECORD_START_WAIT.as_secs()
            ),
            Self::EarlyExit { after_seconds } => write!(
                formatter,
                "the recording stopped early: FFmpeg exited after about {after_seconds} s, before the 60 s run finished."
            ),
            Self::NeverStopped => write!(
                formatter,
                "the recording never stopped: FFmpeg was still running {} s after the stop request.",
                RECORD_STOP_WAIT.as_secs()
            ),
            Self::UnreadableCommand => formatter.write_str(
                "the recording started but its FFmpeg command line could not be read.",
            ),
            Self::MissingSamples => {
                formatter.write_str("the recording finished without any measured samples.")
            }
            Self::AppExited { status } => write!(
                formatter,
                "Snapline exited during the recording ({status}) before the run finished."
            ),
            Self::WrongTarget {
                width,
                height,
                frame_rate,
            } => write!(
                formatter,
                "the recording target was {width} x {height} at {frame_rate} fps; the budget needs 1920 x 1080 at 30 fps."
            ),
            Self::SoftwareEncoder { encoder } => write!(
                formatter,
                "the recording used {encoder}, a software encoder; the CPU budget requires a hardware encoder."
            ),
            Self::WebviewOpen { count } => write!(
                formatter,
                "the recording had {count} WebView2 process(es) open; the recording budget needs none."
            ),
        }
    }
}

impl std::error::Error for RecordingFailure {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BudgetMode {
    Default,
    Recording,
    Full,
}

fn parse_budget_mode(args: &[String]) -> Result<BudgetMode> {
    match args {
        [] => Ok(BudgetMode::Default),
        [flag] if flag.as_str() == "--record" => Ok(BudgetMode::Recording),
        [flag] if flag.as_str() == "--full" => Ok(BudgetMode::Full),
        [other, ..] => bail!("unknown budget option: {other} (use --record or --full)"),
    }
}

fn action_command(exe: &Path, action: &str) -> Command {
    let mut command = Command::new(exe);
    command.arg("--action").arg(action);
    command
}

fn send_action(exe: &Path, action: &str) -> Result<()> {
    action_command(exe, action)
        .spawn()
        .with_context(|| format!("could not send the {action} action to Snapline"))?;
    Ok(())
}

fn ffmpeg_children(root: u32, root_created: u64) -> Result<Vec<u32>> {
    let rows = raw_processes()?;
    Ok(descendants(&rows, root, root_created)
        .into_iter()
        .filter(|pid| {
            rows.iter()
                .any(|row| row.pid == *pid && row.name.eq_ignore_ascii_case(FFMPEG_PROCESS))
        })
        .collect())
}

fn wait_for_recording(app: &mut Child, root: u32, root_created: u64) -> Result<RecordingCommand> {
    let deadline = Instant::now() + RECORD_START_WAIT;
    let mut recording_seen = None;
    loop {
        if let Ok(Some(status)) = app.try_wait() {
            bail!("Snapline exited before the recording started ({status})");
        }
        for pid in ffmpeg_children(root, root_created)? {
            let Some(line) = command_line(pid) else {
                continue;
            };
            if !line.contains("ddagrab=") {
                continue;
            }
            if let Some(command) = parse_recording_command(&line) {
                return Ok(command);
            }
            let since = *recording_seen.get_or_insert_with(Instant::now);
            if since.elapsed() >= RECORD_PARSE_WAIT {
                return Err(RecordingFailure::UnreadableCommand.into());
            }
        }
        if Instant::now() >= deadline {
            return Err(RecordingFailure::NeverStarted.into());
        }
        sleep(POLL);
    }
}

fn wait_for_ffmpeg_exit(root: u32, root_created: u64) -> Result<()> {
    let deadline = Instant::now() + RECORD_STOP_WAIT;
    let mut clear_since = None;
    loop {
        if ffmpeg_children(root, root_created)?.is_empty() {
            let since = *clear_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= RECORD_STABLE {
                return Ok(());
            }
        } else {
            clear_since = None;
        }
        if Instant::now() >= deadline {
            return Err(RecordingFailure::NeverStopped.into());
        }
        sleep(POLL);
    }
}

fn recording_row(report: Option<&RecordingReport>) -> RowOutcome {
    let row = BudgetRow::Recording;
    let Some(report) = report else {
        println!("  FAIL {}: the measurement is missing.", row.label());
        return RowOutcome::missing(row);
    };
    println!(
        "budget: recording: app {}, FFmpeg {}, total {}, CPU {:.1}% (FFmpeg {:.1}%), encoder {}",
        format_bytes(report.max_app_private()),
        format_bytes(report.max_ffmpeg_private()),
        format_bytes(report.max_total_private()),
        report.max_cpu_percent(),
        report.max_ffmpeg_cpu_percent(),
        report.command.encoder
    );
    let mut failures = Vec::new();
    if !report.has_samples() {
        failures.push(BudgetFailure::new(
            row,
            FailureKind::Recording(RecordingFailure::MissingSamples),
        ));
    }
    if !report.target_matches_plan() {
        failures.push(BudgetFailure::new(
            row,
            FailureKind::Recording(RecordingFailure::WrongTarget {
                width: report.command.width,
                height: report.command.height,
                frame_rate: report.command.frame_rate,
            }),
        ));
    }
    if report.memory_passes() {
        println!(
            "  PASS {}: {} is under the {} budget.",
            row.label(),
            format_bytes(report.max_total_private()),
            format_bytes(RECORD_MEMORY_LIMIT)
        );
    } else {
        check_under(
            row,
            &mut failures,
            report.max_total_private(),
            RECORD_MEMORY_LIMIT,
        );
    }
    if report.cpu_passes() {
        println!(
            "  PASS {}: CPU {:.1}% is under the {:.1}% budget.",
            row.label(),
            report.max_cpu_percent(),
            RECORD_CPU_LIMIT
        );
    } else {
        check_cpu(
            row,
            &mut failures,
            report.max_cpu_percent(),
            RECORD_CPU_LIMIT,
        );
    }
    let unreadable: usize = report.samples.iter().map(|sample| sample.unreadable).sum();
    if unreadable > 0 {
        let kind = FailureKind::Unreadable { count: unreadable };
        println!("  FAIL {}: {kind}", row.label());
        failures.push(BudgetFailure::new(row, kind));
    }
    if report.hardware() {
        println!(
            "  PASS {}: {} is a hardware encoder.",
            row.label(),
            report.command.encoder
        );
    } else {
        let kind = FailureKind::Recording(RecordingFailure::SoftwareEncoder {
            encoder: report.command.encoder.clone(),
        });
        println!("  FAIL {}: {kind}", row.label());
        failures.push(BudgetFailure::new(row, kind));
    }
    if report.passes() && failures.is_empty() {
        RowOutcome::measured(row)
    } else {
        RowOutcome::with_failures(row, failures)
    }
}

fn webview_data_directory() -> Result<String> {
    let local = std::env::var_os("LOCALAPPDATA")
        .context("LOCALAPPDATA is not set, so the WebView2 data directory is unknown")?;
    Ok(PathBuf::from(local)
        .join("SnapLine")
        .join("WebView2")
        .to_string_lossy()
        .into_owned())
}

fn wait_for_creation(pid: u32) -> Result<u64> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(created) = process_created(pid) {
            return Ok(created);
        }
        if Instant::now() >= deadline {
            bail!("could not inspect the new Snapline process (pid {pid})");
        }
        sleep(POLL);
    }
}

fn wait_for_exit(child: &mut Child) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            bail!("Snapline exited during startup ({status})");
        }
        if Instant::now() >= deadline {
            return Ok(());
        }
        sleep(POLL);
    }
}

fn ensure_binary(root: &Path) -> Result<PathBuf> {
    let exe = root
        .join("target")
        .join("release")
        .join(format!("snapline{}", std::env::consts::EXE_SUFFIX));
    if exe.exists() {
        return Ok(exe);
    }
    println!("budget: {} is missing, so building it now", exe.display());
    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "--package",
            "snapline",
            "--bin",
            "snapline",
        ])
        .current_dir(root)
        .status()
        .context("could not run cargo build --release")?;
    if !status.success() {
        bail!("the release build failed with {status}");
    }
    if !exe.exists() {
        bail!("the release build did not produce {}", exe.display());
    }
    Ok(exe)
}

fn ensure_not_running() -> Result<()> {
    let rows = raw_processes()?;
    if let Some(row) = rows
        .iter()
        .find(|row| row.name.eq_ignore_ascii_case("snapline.exe"))
    {
        bail!(
            "Snapline is already running (pid {}); close it first because the budget check needs one instance.",
            row.pid
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Suite {
    Narrow,
    Full,
}

struct StartedApp {
    app: AppProcess,
    pid: u32,
    created: u64,
}

fn start_app(exe: &Path) -> Result<StartedApp> {
    println!("budget: starting Snapline with --budget-run");
    let child = Command::new(exe)
        .arg("--budget-run")
        .spawn()
        .with_context(|| format!("could not start {}", exe.display()))?;
    let mut app = AppProcess {
        child,
        stopped: false,
    };
    let pid = app.pid();
    let created = wait_for_creation(pid)?;
    wait_for_exit(&mut app.child)?;
    println!(
        "budget: waiting {} s for the tray to settle",
        STARTUP_WAIT.as_secs()
    );
    sleep(STARTUP_WAIT);
    Ok(StartedApp { app, pid, created })
}

fn wait_linger() {
    println!(
        "budget: waiting {} s for the linger period and the WebView2 processes to exit",
        LINGER_WAIT.as_secs()
    );
    sleep(LINGER_WAIT);
}

fn finish(rows: Vec<RowOutcome>) -> Result<()> {
    let summary = BudgetSummary::new(rows);
    print_summary(&summary);
    if summary.passed() {
        Ok(())
    } else {
        Err(summary_error(&summary))
    }
}

#[allow(clippy::missing_errors_doc)]
pub fn run(root: &Path, args: &[String]) -> Result<()> {
    match parse_budget_mode(args)? {
        BudgetMode::Default => run_idle(root),
        BudgetMode::Recording => run_recording(root),
        BudgetMode::Full => run_full(root),
    }
}

#[allow(clippy::missing_errors_doc)]
pub fn run_full(root: &Path) -> Result<()> {
    println!(
        "budget: full check: every section 6 row, including the interactive capture-to-link and recording runs. The capture-to-link run needs a working upload destination on the default workflow, and both runs open a region overlay."
    );
    let mut rows = run_windows(root, Suite::Full)?;
    rows.push(recording_session(root)?);
    finish(rows)
}

#[allow(clippy::missing_errors_doc, clippy::too_many_lines)]
fn run_idle(root: &Path) -> Result<()> {
    let rows = run_windows(root, Suite::Narrow)?;
    finish(rows)
}

#[allow(clippy::missing_errors_doc, clippy::too_many_lines)]
fn run_windows(root: &Path, suite: Suite) -> Result<Vec<RowOutcome>> {
    let exe = ensure_binary(root)?;
    ensure_not_running()?;
    let data_directory = webview_data_directory()?;
    let full = suite == Suite::Full;
    println!("budget: measuring {}", exe.display());
    let mut started = start_app(&exe)?;
    let pid = started.pid;
    let created = started.created;
    let mut rows = Vec::new();

    let idle = collect(pid, created, &data_directory)?;
    let first_idle = idle.total_private;
    rows.push(idle_row(BudgetRow::TrayIdleBefore, Some(&idle), None, full));

    let main_cold = open_window(&exe, BudgetWindow::Main, pid, 1)?;
    let main = collect(pid, created, &data_directory)?;
    let mut main_row = window_row(BudgetRow::MainWindow, Some(&main), MAIN_LIMIT, full);
    if full {
        check_slow(
            BudgetRow::MainWindow,
            &mut main_row.failures,
            "cold main window",
            main_cold,
            MAIN_COLD_LIMIT,
        );
    }
    rows.push(main_row);
    close_windows(pid)?;
    wait_linger();
    let after_main = collect(pid, created, &data_directory)?;
    rows.push(idle_row(
        BudgetRow::TrayIdleAfter,
        Some(&after_main),
        Some(first_idle),
        full,
    ));

    if full {
        rows.push(capture_to_link(&exe, pid, created, &data_directory)?);
    }

    started.app.stop(&exe);

    if full {
        sleep(POLL);
        let leftover = wait_for_no_app_processes(Duration::from_secs(5))?;
        if leftover > 0 {
            if let Some(row) = rows
                .iter_mut()
                .find(|row| row.row == BudgetRow::TrayIdleAfter)
            {
                row.push(FailureKind::AppLeak { count: leftover });
            }
        }
    } else {
        println!(
            "budget: CPU is printed for the record; section 6 targets 0% when nothing changes."
        );
    }

    Ok(rows)
}

#[allow(clippy::missing_errors_doc)]
fn run_recording(root: &Path) -> Result<()> {
    let row = recording_session(root)?;
    finish(vec![row])
}

#[allow(clippy::missing_errors_doc, clippy::too_many_lines)]
fn recording_session(root: &Path) -> Result<RowOutcome> {
    let row = BudgetRow::Recording;
    let plan = RecordingPlan::default();
    let exe = ensure_binary(root)?;
    ensure_not_running()?;
    let data_directory = webview_data_directory()?;
    println!(
        "budget: recording mode: {} s at {} x {} {} fps in {} sampling windows over {} s",
        plan.seconds,
        plan.width,
        plan.height,
        plan.frame_rate,
        plan.sample_windows(),
        plan.samples_duration().as_secs()
    );
    println!("budget: recording is interactive. {RECORD_INSTRUCTION}");
    let mut started = start_app(&exe)?;
    let pid = started.pid;
    let created = started.created;

    let idle = collect(pid, created, &data_directory)?;
    report("tray idle before recording", &idle);
    if idle.webview > 0 {
        started.app.stop(&exe);
        return Ok(RowOutcome::with_failures(
            row,
            vec![BudgetFailure::new(
                row,
                FailureKind::Recording(RecordingFailure::WebviewOpen {
                    count: idle.webview,
                }),
            )],
        ));
    }

    println!(
        "budget: starting the region recorder. {RECORD_INSTRUCTION} The run needs {} x {} at {} fps.",
        plan.width, plan.height, plan.frame_rate
    );
    if let Err(error) = send_action(&exe, "record_video") {
        started.app.stop(&exe);
        return Err(error);
    }
    let command = match wait_for_recording(&mut started.app.child, pid, created) {
        Ok(command) => command,
        Err(error) => {
            started.app.stop(&exe);
            return failure_row(row, error);
        }
    };
    println!(
        "budget: recording with {} at {} x {} {} fps",
        command.encoder, command.width, command.height, command.frame_rate
    );

    if !command.matches(plan) {
        let _ = send_action(&exe, "record_stop");
        let _ = wait_for_ffmpeg_exit(pid, created);
        started.app.stop(&exe);
        return Ok(RowOutcome::with_failures(
            row,
            vec![BudgetFailure::new(
                row,
                FailureKind::Recording(RecordingFailure::WrongTarget {
                    width: command.width,
                    height: command.height,
                    frame_rate: command.frame_rate,
                }),
            )],
        ));
    }
    if !command.hardware() {
        let _ = send_action(&exe, "record_stop");
        let _ = wait_for_ffmpeg_exit(pid, created);
        started.app.stop(&exe);
        return Ok(RowOutcome::with_failures(
            row,
            vec![BudgetFailure::new(
                row,
                FailureKind::Recording(RecordingFailure::SoftwareEncoder {
                    encoder: command.encoder,
                }),
            )],
        ));
    }

    let windows = plan.sample_windows();
    let mut samples = Vec::new();
    let mut early_exit = None;
    let mut app_exit = None;
    let mut webview = None;
    for index in 0..windows {
        let sample = collect(pid, created, &data_directory)?;
        let recording = RecordingSample::from_sample(&sample);
        println!(
            "budget: sample {}/{}: app {}, FFmpeg {}, total {}, CPU {:.1}% (FFmpeg {:.1}%)",
            index + 1,
            windows,
            format_bytes(recording.app_private),
            format_bytes(recording.ffmpeg_private),
            format_bytes(recording.total_private),
            recording.cpu_percent,
            recording.ffmpeg_cpu_percent
        );
        samples.push(recording);
        if sample.webview > 0 {
            webview = Some(sample.webview);
            break;
        }
        if ffmpeg_children(pid, created)?.is_empty() {
            early_exit = Some(u64::from(index + 1) * CPU_WINDOW.as_secs());
            break;
        }
        if let Ok(Some(status)) = started.app.child.try_wait() {
            app_exit = Some(status);
            break;
        }
    }

    let report = RecordingReport {
        plan,
        command,
        samples,
    };
    let stop_result = send_action(&exe, "record_stop");
    let exit_result = wait_for_ffmpeg_exit(pid, created);
    started.app.stop(&exe);

    let mut outcome = recording_row(Some(&report));
    if let Some(count) = webview {
        outcome.push(FailureKind::Recording(RecordingFailure::WebviewOpen {
            count,
        }));
    }
    if let Some(after_seconds) = early_exit {
        outcome.push(FailureKind::Recording(RecordingFailure::EarlyExit {
            after_seconds,
        }));
    }
    if let Some(status) = app_exit {
        outcome.push(FailureKind::Recording(RecordingFailure::AppExited {
            status: status.to_string(),
        }));
    }
    for result in [stop_result, exit_result] {
        if let Err(error) = result {
            match typed_recording_failure(error) {
                Ok(kind) => outcome.failures.push(BudgetFailure::new(row, kind)),
                Err(error) => return Err(error),
            }
        }
    }
    Ok(outcome)
}

fn typed_recording_failure(error: anyhow::Error) -> Result<FailureKind> {
    match error.downcast::<RecordingFailure>() {
        Ok(failure) => Ok(FailureKind::Recording(failure)),
        Err(error) => Err(error),
    }
}

fn failure_row(row: BudgetRow, error: anyhow::Error) -> Result<RowOutcome> {
    let kind = typed_recording_failure(error)?;
    Ok(RowOutcome::with_failures(
        row,
        vec![BudgetFailure::new(row, kind)],
    ))
}

fn app_process_count() -> Result<usize> {
    Ok(raw_processes()?
        .iter()
        .filter(|row| row.name.eq_ignore_ascii_case(SNAPLINE_PROCESS))
        .count())
}

fn wait_for_no_app_processes(waited: Duration) -> Result<usize> {
    let deadline = Instant::now() + waited;
    loop {
        let count = app_process_count()?;
        if count == 0 || Instant::now() >= deadline {
            return Ok(count);
        }
        sleep(POLL);
    }
}

#[must_use]
pub fn is_link(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return true;
    }
    [
        "](http://",
        "](https://",
        "src=\"http://",
        "src=\"https://",
        "src='http://",
        "src='https://",
        "[img]http://",
        "[img]https://",
    ]
    .iter()
    .any(|needle| trimmed.contains(needle))
}

fn clipboard_text() -> Option<String> {
    for _ in 0..20 {
        // SAFETY: OpenClipboard with no owner opens the clipboard for this thread and is balanced below.
        if unsafe { OpenClipboard(None) }.is_ok() {
            let text = read_clipboard_text();
            // SAFETY: balances the successful OpenClipboard on this thread.
            let _ = unsafe { CloseClipboard() };
            return text;
        }
        sleep(Duration::from_millis(50));
    }
    None
}

fn read_clipboard_text() -> Option<String> {
    // SAFETY: the clipboard is open on this thread for the duration of the call.
    let handle = unsafe { GetClipboardData(u32::from(CF_UNICODETEXT.0)) }.ok()?;
    if handle.is_invalid() || handle.0.is_null() {
        return None;
    }
    // SAFETY: the handle is a live global memory block owned by the clipboard.
    let pointer = unsafe { GlobalLock(HGLOBAL(handle.0)) }.cast::<u16>();
    if pointer.is_null() {
        return None;
    }
    let mut length = 0usize;
    // SAFETY: the text is null-terminated and is scanned within a fixed upper bound.
    while length < 512 * 1024 && unsafe { *pointer.add(length) } != 0 {
        length += 1;
    }
    let mut wide = Vec::with_capacity(length);
    for index in 0..length {
        // SAFETY: index is below the scanned length of the same live buffer.
        wide.push(unsafe { *pointer.add(index) });
    }
    // SAFETY: balances GlobalLock on the same handle.
    unsafe {
        let _ = GlobalUnlock(HGLOBAL(handle.0));
    }
    Some(String::from_utf16_lossy(&wide))
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _device: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: EnumDisplayMonitors passes back the pointer supplied to it in data.
    let total = unsafe { &mut *(data.0 as *mut u64) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: info is writable and cbSize matches its size.
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        let width = u64::try_from(info.rcMonitor.right - info.rcMonitor.left).unwrap_or(0);
        let height = u64::try_from(info.rcMonitor.bottom - info.rcMonitor.top).unwrap_or(0);
        *total = total.saturating_add(width.saturating_mul(height).saturating_mul(4));
    }
    BOOL(1)
}

fn monitors_frozen_bytes() -> u64 {
    let mut total = 0u64;
    // SAFETY: the callback dereferences the pointer to total and EnumDisplayMonitors returns it.
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM((&mut total as *mut u64) as isize),
        )
    };
    total
}

fn wait_for_new_link(before: Option<&str>) -> Option<String> {
    let deadline = Instant::now() + LINK_WAIT;
    loop {
        if let Some(text) = clipboard_text() {
            if is_link(&text) && before != Some(text.as_str()) {
                return Some(text);
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        sleep(Duration::from_secs(1));
    }
}

fn capture_row(
    sample: Option<&Sample>,
    overlay_elapsed: Duration,
    limit: u64,
    finished: bool,
    link: bool,
) -> RowOutcome {
    let row = BudgetRow::CaptureToLink;
    let Some(sample) = sample else {
        println!("  FAIL {}: the measurement is missing.", row.label());
        return RowOutcome::missing(row);
    };
    report("region overlay", sample);
    let mut failures = Vec::new();
    check_slow(
        row,
        &mut failures,
        "region overlay",
        overlay_elapsed,
        OVERLAY_TIMING_LIMIT,
    );
    check_under(row, &mut failures, sample.total_private, limit);
    check_webview(row, &mut failures, sample.webview);
    if !finished {
        let kind = FailureKind::CaptureUnfinished {
            waited: CAPTURE_WAIT,
        };
        println!("  FAIL {}: {kind}", row.label());
        failures.push(BudgetFailure::new(row, kind));
    } else if !link {
        let kind = FailureKind::LinkMissing;
        println!("  FAIL {}: {kind}", row.label());
        failures.push(BudgetFailure::new(row, kind));
    } else {
        println!("  PASS {}: the link reached the clipboard.", row.label());
    }
    RowOutcome::with_failures(row, failures)
}

#[allow(clippy::missing_errors_doc)]
fn capture_to_link(exe: &Path, pid: u32, created: u64, data_directory: &str) -> Result<RowOutcome> {
    let row = BudgetRow::CaptureToLink;
    println!(
        "budget: capture-to-link run: when the region overlay appears, Aaron must select a region and let the default workflow copy the link."
    );
    let before = clipboard_text().filter(|text| is_link(text));
    let started = Instant::now();
    send_action(exe, "capture_region")?;
    let overlay_elapsed = match wait_for_visible_window(pid, "region overlay", OVERLAY_WAIT) {
        Ok(_) => started.elapsed(),
        Err(_) => {
            let kind = FailureKind::OverlayMissing {
                waited: OVERLAY_WAIT,
            };
            println!("  FAIL {}: {kind}", row.label());
            return Ok(RowOutcome::with_failures(
                row,
                vec![BudgetFailure::new(row, kind)],
            ));
        }
    };
    sleep(OVERLAY_SETTLE);
    let overlay = snapshot(pid, created, data_directory)?;
    let limit = overlay_limit(monitors_frozen_bytes());
    let finished = wait_for_no_windows(pid, "region overlay", CAPTURE_WAIT).is_ok();
    let link = finished && wait_for_new_link(before.as_deref()).is_some();
    let mut outcome = capture_row(Some(&overlay), overlay_elapsed, limit, finished, link);
    let after = snapshot(pid, created, data_directory)?;
    check_webview(row, &mut outcome.failures, after.webview);
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::Path;
    use std::time::Duration;

    use super::{
        action_command, capture_row, descendants, encoder_is_hardware, is_link,
        overlay_limit, parse_budget_mode, parse_recording_command, recording_row, webview_children,
        window_row, BudgetMode, BudgetRow, BudgetSummary, FailureKind, Limit, ProcessRow,
        RecordingCommand, RecordingFailure, RecordingPlan, RecordingReport, RecordingSample,
        RowOutcome, Sample, IDLE_DRIFT_LIMIT, IDLE_LIMIT, MAIN_LIMIT, MIB,
        OVERLAY_LIMIT, OVERLAY_TIMING_LIMIT, RECORD_CPU_LIMIT,
        RECORD_INSTRUCTION, RECORD_MEMORY_LIMIT,
    };

    const DATA: &str = "C:\\Users\\tester\\AppData\\Local\\SnapLine\\WebView2";
    const COMMAND_LINE: &str = r#""C:\Tools\ffmpeg\ffmpeg.exe" -y -f lavfi -i ddagrab=framerate=30:output_idx=0:video_size=1920x1080:offset_x=10:offset_y=20 -vf hwdownload,format=bgra -map 0:v:0 -c:v h264_nvenc -pix_fmt yuv420p -f mp4 C:\Temp\record-1.mp4"#;

    fn row(
        pid: u32,
        parent: u32,
        name: &str,
        created: u64,
        command_line: Option<&str>,
    ) -> ProcessRow {
        ProcessRow {
            pid,
            parent,
            name: name.to_string(),
            created: Some(created),
            command_line: command_line.map(str::to_string),
        }
    }

    fn app() -> ProcessRow {
        row(100, 1, "snapline.exe", 1_000, None)
    }

    #[test]
    fn under_limit_is_exclusive() {
        assert!(Limit::Under(100).allows(99));
        assert!(!Limit::Under(100).allows(100));
        assert!(!Limit::Under(100).allows(101));
    }

    #[test]
    fn within_limit_includes_the_allowance() {
        let limit = Limit::Within {
            reference: 100,
            allowance: 3,
        };
        assert!(limit.allows(103));
        assert!(!limit.allows(104));
        assert!(limit.allows(90));
    }

    #[test]
    fn within_limit_tolerates_a_reference_near_the_maximum() {
        let limit = Limit::Within {
            reference: u64::MAX,
            allowance: 3,
        };
        assert!(limit.allows(u64::MAX));
    }

    #[test]
    fn children_created_before_the_app_are_stale() {
        let rows = vec![app(), row(200, 100, "msedgewebview2.exe", 900, Some(DATA))];
        assert!(webview_children(&rows, 100, 1_000, DATA).is_empty());
    }

    #[test]
    fn children_created_after_the_app_are_counted() {
        let rows = vec![
            app(),
            row(200, 100, "msedgewebview2.exe", 1_100, Some(DATA)),
        ];
        assert_eq!(webview_children(&rows, 100, 1_000, DATA), vec![200]);
    }

    #[test]
    fn identity_matches_the_data_directory() {
        let rows = vec![
            app(),
            row(
                200,
                100,
                "msedgewebview2.exe",
                1_100,
                Some("--user-data-dir=C:\\Users\\tester\\AppData\\Local\\SnapLine\\WebView2"),
            ),
            row(
                201,
                100,
                "msedgewebview2.exe",
                1_100,
                Some("--user-data-dir=C:\\Temp\\Other\\WebView2"),
            ),
        ];
        assert_eq!(webview_children(&rows, 100, 1_000, DATA), vec![200]);
    }

    #[test]
    fn descendants_of_a_matching_browser_are_counted() {
        let rows = vec![
            app(),
            row(300, 100, "msedgewebview2.exe", 1_100, Some(DATA)),
            row(
                301,
                300,
                "msedgewebview2.exe",
                1_200,
                Some("--type=renderer"),
            ),
        ];
        assert_eq!(webview_children(&rows, 100, 1_000, DATA), vec![300, 301]);
    }

    #[test]
    fn descendants_of_another_browser_are_ignored() {
        let rows = vec![
            app(),
            row(
                400,
                100,
                "msedgewebview2.exe",
                1_100,
                Some("--user-data-dir=C:\\Temp\\Other"),
            ),
            row(
                401,
                400,
                "msedgewebview2.exe",
                1_200,
                Some("--type=renderer"),
            ),
        ];
        assert!(webview_children(&rows, 100, 1_000, DATA).is_empty());
    }

    #[test]
    fn unrelated_processes_are_ignored() {
        let rows = vec![
            app(),
            row(500, 999, "msedgewebview2.exe", 1_100, Some(DATA)),
            row(501, 100, "notepad.exe", 1_100, None),
        ];
        assert!(webview_children(&rows, 100, 1_000, DATA).is_empty());
        assert_eq!(descendants(&rows, 100, 1_000), vec![501]);
    }

    #[test]
    fn a_cycle_does_not_hang() {
        let rows = vec![
            app(),
            row(600, 601, "msedgewebview2.exe", 1_100, Some(DATA)),
            row(601, 600, "msedgewebview2.exe", 1_200, Some(DATA)),
        ];
        assert!(webview_children(&rows, 100, 1_000, DATA).is_empty());
    }

    #[test]
    fn missing_command_lines_fall_back_to_the_parent_chain() {
        let rows = vec![
            app(),
            row(700, 100, "msedgewebview2.exe", 1_100, None),
            row(701, 700, "msedgewebview2.exe", 1_200, None),
        ];
        assert_eq!(webview_children(&rows, 100, 1_000, DATA), vec![700, 701]);
    }

    fn plan() -> RecordingPlan {
        RecordingPlan::default()
    }

    fn command(width: u32, height: u32, frame_rate: u32) -> RecordingCommand {
        RecordingCommand {
            encoder: "h264_nvenc".to_owned(),
            width,
            height,
            frame_rate,
        }
    }

    fn sample(total: u64, cpu: f64) -> RecordingSample {
        RecordingSample {
            app_private: total / 2,
            ffmpeg_private: total - total / 2,
            total_private: total,
            unreadable: 0,
            cpu_percent: cpu,
            ffmpeg_cpu_percent: cpu / 2.0,
        }
    }

    fn report(total: u64, cpu: f64) -> RecordingReport {
        RecordingReport {
            plan: plan(),
            command: command(1920, 1080, 30),
            samples: vec![sample(total, cpu)],
        }
    }

    fn memory_sample(total: u64, webview: usize, cpu: f64) -> Sample {
        Sample {
            app_private: total,
            total_private: total,
            unreadable: 0,
            webview,
            cpu_percent: cpu,
            children: Vec::new(),
        }
    }

    #[test]
    fn the_recording_plan_defaults_to_sixty_seconds_at_1080p30() {
        let plan = plan();
        assert_eq!(plan.seconds, 60);
        assert_eq!(plan.width, 1920);
        assert_eq!(plan.height, 1080);
        assert_eq!(plan.frame_rate, 30);
        assert_eq!(plan.sample_windows(), 6);
        assert_eq!(plan.samples_duration(), Duration::from_secs(60));
    }

    #[test]
    fn the_recording_run_is_explicit_and_off_by_default() {
        assert!(matches!(parse_budget_mode(&[]), Ok(BudgetMode::Default)));
        assert!(matches!(
            parse_budget_mode(&["--record".to_owned()]),
            Ok(BudgetMode::Recording)
        ));
        assert!(parse_budget_mode(&["--record".to_owned(), "extra".to_owned()]).is_err());
        assert!(parse_budget_mode(&["--recording".to_owned()]).is_err());
    }

    #[test]
    fn the_action_command_forwards_one_action() {
        let exe = Path::new(r"C:\Program Files\Snapline\snapline.exe");
        let command = action_command(exe, "record_video");
        assert_eq!(
            command.get_program(),
            OsStr::new(r"C:\Program Files\Snapline\snapline.exe")
        );
        let args: Vec<&OsStr> = command.get_args().collect();
        assert_eq!(
            args,
            vec![OsStr::new("--action"), OsStr::new("record_video")]
        );
    }

    #[test]
    fn hardware_encoders_are_recognised_and_software_is_not() {
        for encoder in [
            "h264_nvenc",
            "h264_qsv",
            "h264_amf",
            "hevc_nvenc",
            "hevc_qsv",
            "hevc_amf",
        ] {
            assert!(encoder_is_hardware(encoder));
        }
        for encoder in ["libx264", "libx265", "libvpx-vp9", ""] {
            assert!(!encoder_is_hardware(encoder));
        }
    }

    #[test]
    fn the_recording_command_line_yields_the_encoder_and_target() {
        assert_eq!(
            parse_recording_command(COMMAND_LINE),
            Some(command(1920, 1080, 30))
        );
    }

    #[test]
    fn a_missing_encoder_or_target_is_rejected() {
        assert_eq!(parse_recording_command("ffmpeg.exe -y -f lavfi"), None);
        assert_eq!(
            parse_recording_command(
                "ffmpeg.exe -i ddagrab=framerate=30:video_size=1920x1080 -c:v libx264"
            ),
            Some(RecordingCommand {
                encoder: "libx264".to_owned(),
                width: 1920,
                height: 1080,
                frame_rate: 30,
            })
        );
        assert_eq!(
            parse_recording_command("ffmpeg.exe -i ddagrab=framerate=30 -c:v libx264"),
            None
        );
        assert_eq!(
            parse_recording_command("ffmpeg.exe -i ddagrab=video_size=1920x1080 -c:v libx264"),
            None
        );
    }

    #[test]
    fn only_1080p30_matches_the_plan() {
        assert!(command(1920, 1080, 30).matches(plan()));
        assert!(!command(1280, 720, 30).matches(plan()));
        assert!(!command(1920, 1080, 60).matches(plan()));
    }

    #[test]
    fn the_report_takes_the_peak_of_every_sample() {
        let report = RecordingReport {
            plan: plan(),
            command: command(1920, 1080, 30),
            samples: vec![
                sample(60 * MIB, 2.0),
                sample(149 * MIB, 4.9),
                sample(80 * MIB, 1.0),
            ],
        };
        assert_eq!(report.max_total_private(), 149 * MIB);
        assert_eq!(report.max_app_private(), (149 * MIB) / 2);
        assert_eq!(report.max_ffmpeg_private(), 149 * MIB - (149 * MIB) / 2);
        assert!(report.max_cpu_percent() > 4.8 && report.max_cpu_percent() < 5.0);
        assert!(report.memory_passes());
        assert!(report.cpu_passes());
        assert!(report.hardware());
        assert!(report.passes());
    }

    #[test]
    fn the_memory_limit_is_strictly_under_150_mb() {
        assert!(report(RECORD_MEMORY_LIMIT - 1, 1.0).memory_passes());
        assert!(!report(RECORD_MEMORY_LIMIT, 1.0).memory_passes());
        assert!(!report(RECORD_MEMORY_LIMIT + 1, 1.0).memory_passes());
    }

    #[test]
    fn the_cpu_limit_is_strictly_under_five_percent() {
        assert!(report(1, RECORD_CPU_LIMIT - 0.01).cpu_passes());
        assert!(!report(1, RECORD_CPU_LIMIT).cpu_passes());
    }

    #[test]
    fn a_software_encoder_never_passes() {
        let mut report = report(1, 1.0);
        report.command.encoder = "libx264".to_owned();
        assert!(!report.hardware());
        assert!(!report.passes());
    }

    #[test]
    fn failures_name_the_missing_start_and_the_missing_stop() {
        assert!(RecordingFailure::NeverStarted
            .to_string()
            .contains("never started"));
        assert!(RecordingFailure::NeverStopped
            .to_string()
            .contains("never stopped"));
        assert!(RecordingFailure::EarlyExit { after_seconds: 30 }
            .to_string()
            .contains("stopped early"));
        assert!(RecordingFailure::WrongTarget {
            width: 1280,
            height: 720,
            frame_rate: 30,
        }
        .to_string()
        .contains("1920 x 1080"));
        assert!(RecordingFailure::SoftwareEncoder {
            encoder: "libx264".to_owned(),
        }
        .to_string()
        .contains("hardware encoder"));
    }

    #[test]
    fn every_memory_row_passes_one_byte_under_its_budget() {
        let idle = memory_sample(IDLE_LIMIT - 1, 0, 0.0);
        assert!(super::idle_row(BudgetRow::TrayIdleBefore, Some(&idle), None, true).passed());
        let main = memory_sample(MAIN_LIMIT - 1, 1, 0.0);
        assert!(window_row(BudgetRow::MainWindow, Some(&main), MAIN_LIMIT, true).passed());
        let after = memory_sample(IDLE_LIMIT - 1, 0, 0.0);
        assert!(super::idle_row(
            BudgetRow::TrayIdleAfter,
            Some(&after),
            Some(IDLE_LIMIT - 1),
            true
        )
        .passed());
        assert!(recording_row(Some(&report(RECORD_MEMORY_LIMIT - 1, 1.0))).passed());
    }

    #[test]
    fn every_memory_row_fails_at_its_exact_budget() {
        let idle = memory_sample(IDLE_LIMIT, 0, 0.0);
        assert!(!super::idle_row(BudgetRow::TrayIdleBefore, Some(&idle), None, false).passed());
        let main = memory_sample(MAIN_LIMIT, 1, 0.0);
        assert!(!window_row(BudgetRow::MainWindow, Some(&main), MAIN_LIMIT, false).passed());
        let after = memory_sample(IDLE_LIMIT, 0, 0.0);
        assert!(!super::idle_row(BudgetRow::TrayIdleAfter, Some(&after), None, false).passed());
        assert!(!recording_row(Some(&report(RECORD_MEMORY_LIMIT, 1.0))).passed());
    }

    #[test]
    fn the_after_windows_row_is_within_three_megabytes_of_the_first_idle() {
        let reference = 10 * MIB;
        let allowed = memory_sample(reference + IDLE_DRIFT_LIMIT, 0, 0.0);
        assert!(super::idle_row(
            BudgetRow::TrayIdleAfter,
            Some(&allowed),
            Some(reference),
            false
        )
        .passed());
        let over = memory_sample(reference + IDLE_DRIFT_LIMIT + 1, 0, 0.0);
        let outcome = super::idle_row(
            BudgetRow::TrayIdleAfter,
            Some(&over),
            Some(reference),
            false,
        );
        assert!(!outcome.passed());
        assert!(outcome.failures.iter().any(|failure| failure.kind
            == FailureKind::IdleDrift {
                measured: reference + IDLE_DRIFT_LIMIT + 1,
                reference,
                allowance: IDLE_DRIFT_LIMIT,
            }));
    }

    #[test]
    fn idle_cpu_must_round_to_zero_percent() {
        let below = memory_sample(1, 0, 0.9);
        assert!(super::idle_row(BudgetRow::TrayIdleBefore, Some(&below), None, true).passed());
        let at = memory_sample(1, 0, 1.0);
        let outcome = super::idle_row(BudgetRow::TrayIdleBefore, Some(&at), None, true);
        assert!(!outcome.passed());
        assert!(outcome.failures.iter().any(|failure| failure.kind
            == FailureKind::CpuOver {
                measured: 1.0,
                limit: 1.0,
            }));
        assert!(super::idle_row(BudgetRow::TrayIdleBefore, Some(&at), None, false).passed());
    }

    #[test]
    fn a_webview_leak_fails_the_idle_rows() {
        let leaked = memory_sample(1, 2, 0.0);
        let outcome = super::idle_row(BudgetRow::TrayIdleBefore, Some(&leaked), None, false);
        assert!(!outcome.passed());
        assert!(outcome
            .failures
            .iter()
            .any(|failure| failure.kind == FailureKind::WebviewLeak { count: 2 }));
        assert!(FailureKind::WebviewLeak { count: 1 }
            .to_string()
            .contains("1 WebView2 process"));
    }

    #[test]
    fn an_app_leak_is_typed_and_names_the_count() {
        let mut row = RowOutcome::measured(BudgetRow::TrayIdleAfter);
        row.push(FailureKind::AppLeak { count: 2 });
        assert!(!row.passed());
        assert_eq!(row.failures.len(), 1);
        assert!(row.failures[0]
            .kind
            .to_string()
            .contains("2 Snapline process"));
    }

    #[test]
    fn a_missing_sample_fails_every_row() {
        for row in [
            BudgetRow::TrayIdleBefore,
            BudgetRow::MainWindow,
            BudgetRow::TrayIdleAfter,
            BudgetRow::CaptureToLink,
            BudgetRow::Recording,
        ] {
            let outcome = RowOutcome::missing(row);
            assert!(!outcome.passed());
            assert_eq!(outcome.failures[0].kind, FailureKind::MissingSample);
            assert!(outcome.summary_line().contains("missing"));
        }
        assert!(!super::idle_row(BudgetRow::TrayIdleBefore, None, None, true).passed());
        assert!(!window_row(BudgetRow::MainWindow, None, MAIN_LIMIT, true).passed());
        assert!(!recording_row(None).passed());
    }

    #[test]
    fn a_recording_report_without_samples_fails() {
        let empty = RecordingReport {
            plan: plan(),
            command: command(1920, 1080, 30),
            samples: Vec::new(),
        };
        assert!(!empty.has_samples());
        assert!(!empty.passes());
        let outcome = recording_row(Some(&empty));
        assert!(!outcome.passed());
        assert!(outcome.failures.iter().any(
            |failure| failure.kind == FailureKind::Recording(RecordingFailure::MissingSamples)
        ));
    }

    #[test]
    fn recording_target_and_encoder_failures_are_typed() {
        let mut wrong = report(1, 1.0);
        wrong.command = command(1280, 720, 30);
        assert!(!wrong.target_matches_plan());
        assert!(!wrong.passes());
        let outcome = recording_row(Some(&wrong));
        assert!(outcome.failures.iter().any(|failure| failure.kind
            == FailureKind::Recording(RecordingFailure::WrongTarget {
                width: 1280,
                height: 720,
                frame_rate: 30,
            })));

        let mut software = report(1, 1.0);
        software.command.encoder = "libx264".to_owned();
        assert!(!software.passes());
        let outcome = recording_row(Some(&software));
        assert!(outcome.failures.iter().any(|failure| failure.kind
            == FailureKind::Recording(RecordingFailure::SoftwareEncoder {
                encoder: "libx264".to_owned(),
            })));
    }

    #[test]
    fn the_recording_failure_says_where_to_select_the_region() {
        let message = FailureKind::Recording(RecordingFailure::WrongTarget {
            width: 1280,
            height: 720,
            frame_rate: 30,
        })
        .to_string();
        assert!(message.contains(RECORD_INSTRUCTION));
        assert!(message.contains("1920 x 1080"));
        let summary = BudgetSummary::new(vec![RowOutcome::with_failures(
            BudgetRow::Recording,
            vec![super::BudgetFailure::new(
                BudgetRow::Recording,
                FailureKind::Recording(RecordingFailure::MissingSamples),
            )],
        )]);
        assert!(summary.render().contains(RECORD_INSTRUCTION));
    }

    #[test]
    fn the_summary_lists_every_row_and_the_first_failure() {
        let pass = super::idle_row(
            BudgetRow::TrayIdleBefore,
            Some(&memory_sample(1, 0, 0.0)),
            None,
            false,
        );
        let fail = window_row(
            BudgetRow::MainWindow,
            Some(&memory_sample(MAIN_LIMIT + 1, 1, 0.0)),
            MAIN_LIMIT,
            false,
        );
        let last = super::idle_row(
            BudgetRow::TrayIdleAfter,
            Some(&memory_sample(1, 0, 0.0)),
            None,
            false,
        );
        let summary = BudgetSummary::new(vec![pass, fail, last]);
        let text = summary.render();
        assert!(text.contains("2 of 3 rows passed"));
        assert!(text.contains("PASS tray idle before any window"));
        assert!(text.contains("FAIL main window open"));
        assert!(text.contains("PASS tray idle after every window"));
        assert!(text.contains("first failing row: main window open"));
        assert!(!summary.passed());
        assert_eq!(summary.passed_rows(), 2);
        assert_eq!(
            summary.first_failure().map(|row| row.row),
            Some(BudgetRow::MainWindow)
        );
    }

    #[test]
    fn a_passing_summary_has_no_failing_row() {
        let summary = BudgetSummary::new(vec![window_row(
            BudgetRow::MainWindow,
            Some(&memory_sample(1, 1, 0.0)),
            MAIN_LIMIT,
            false,
        )]);
        assert!(summary.passed());
        assert!(summary.first_failure().is_none());
        assert!(summary.render().contains("every row passed"));
    }

    #[test]
    fn the_overlay_budget_adds_one_frozen_monitor_copy() {
        assert_eq!(overlay_limit(0), OVERLAY_LIMIT);
        assert_eq!(overlay_limit(1024), OVERLAY_LIMIT + 1024);
        assert_eq!(overlay_limit(u64::MAX), u64::MAX);
    }

    #[test]
    fn the_overlay_timing_limit_is_strict() {
        let mut failures = Vec::new();
        super::check_slow(
            BudgetRow::CaptureToLink,
            &mut failures,
            "region overlay",
            OVERLAY_TIMING_LIMIT - Duration::from_millis(1),
            OVERLAY_TIMING_LIMIT,
        );
        assert!(failures.is_empty());
        super::check_slow(
            BudgetRow::CaptureToLink,
            &mut failures,
            "region overlay",
            OVERLAY_TIMING_LIMIT,
            OVERLAY_TIMING_LIMIT,
        );
        assert_eq!(failures.len(), 1);
        assert!(matches!(&failures[0].kind, FailureKind::Slow { .. }));
    }

    #[test]
    fn the_capture_to_link_row_passes_and_fails_every_part() {
        let good = memory_sample(OVERLAY_LIMIT - 1, 0, 0.0);
        let pass = capture_row(
            Some(&good),
            OVERLAY_TIMING_LIMIT - Duration::from_millis(1),
            OVERLAY_LIMIT,
            true,
            true,
        );
        assert!(pass.passed());

        let slow = capture_row(Some(&good), OVERLAY_TIMING_LIMIT, OVERLAY_LIMIT, true, true);
        assert!(!slow.passed());

        let heavy = memory_sample(OVERLAY_LIMIT, 0, 0.0);
        let leaked = capture_row(Some(&heavy), Duration::ZERO, OVERLAY_LIMIT, true, true);
        assert!(!leaked.passed());
        assert!(leaked.failures.iter().any(|failure| failure.kind
            == FailureKind::OverMemory {
                measured: OVERLAY_LIMIT,
                limit: OVERLAY_LIMIT,
            }));

        let unfinished = capture_row(Some(&good), Duration::ZERO, OVERLAY_LIMIT, false, false);
        assert!(unfinished
            .failures
            .iter()
            .any(|failure| matches!(&failure.kind, FailureKind::CaptureUnfinished { .. })));

        let no_link = capture_row(Some(&good), Duration::ZERO, OVERLAY_LIMIT, true, false);
        assert!(no_link
            .failures
            .iter()
            .any(|failure| failure.kind == FailureKind::LinkMissing));

        let missing = capture_row(None, Duration::ZERO, OVERLAY_LIMIT, false, false);
        assert!(!missing.passed());
        assert!(missing
            .failures
            .iter()
            .any(|failure| failure.kind == FailureKind::MissingSample));
    }

    #[test]
    fn every_link_format_is_recognised() {
        assert!(is_link("https://example.test/image.png"));
        assert!(is_link("  http://example.test/image.png  "));
        assert!(is_link("![image](https://example.test/image.png)"));
        assert!(is_link("<img src=\"https://example.test/image.png\">"));
        assert!(is_link("[img]https://example.test/image.png[/img]"));
        assert!(!is_link("just some text"));
        assert!(!is_link("ftp://example.test/image.png"));
        assert!(!is_link(""));
    }

    #[test]
    fn every_row_has_a_distinct_label() {
        let rows = [
            BudgetRow::TrayIdleBefore,
            BudgetRow::MainWindow,
            BudgetRow::TrayIdleAfter,
            BudgetRow::CaptureToLink,
            BudgetRow::Recording,
        ];
        let mut labels: Vec<&str> = rows.iter().map(|row| row.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), rows.len());
    }
}
