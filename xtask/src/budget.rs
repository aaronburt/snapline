#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use windows::core::BOOL;
use windows::Win32::Foundation::{
    CloseHandle, FILETIME, HANDLE, HWND, LPARAM, UNICODE_STRING, WPARAM,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, Toolhelp32ReadProcessMemory,
    PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
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
const PANEL_LIMIT: u64 = 90 * MIB;
const MAIN_LIMIT: u64 = 120 * MIB;
const BOTH_LIMIT: u64 = 150 * MIB;
const STARTUP_WAIT: Duration = Duration::from_secs(30);
const CPU_WINDOW: Duration = Duration::from_secs(10);
const LINGER_WAIT: Duration = Duration::from_secs(25);
const WINDOW_WAIT: Duration = Duration::from_secs(20);
const CLOSE_WAIT: Duration = Duration::from_secs(10);
const EXIT_WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(250);
const WEBVIEW_PROCESS: &str = "msedgewebview2.exe";
const COMMAND_LINE_INFORMATION: u32 = 60;

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

fn cpu_seconds(rows: &[ProcessRow], root: u32, root_created: u64) -> f64 {
    let mut total = process_cpu_seconds(root).unwrap_or(0.0);
    for pid in descendants(rows, root, root_created) {
        total += process_cpu_seconds(pid).unwrap_or(0.0);
    }
    total
}

struct Sample {
    app_private: u64,
    total_private: u64,
    unreadable: usize,
    webview: usize,
    cpu_percent: f64,
    children: Vec<(u32, String, u64)>,
}

fn collect(root: u32, root_created: u64, data_directory: &str) -> Result<Sample> {
    let before = scan_processes(root)?;
    let cpu_before = cpu_seconds(&before, root, root_created);
    let clock = Instant::now();
    sleep(CPU_WINDOW);
    let rows = scan_processes(root)?;
    let elapsed = clock.elapsed().as_secs_f64();
    let cpu_after = cpu_seconds(&rows, root, root_created);
    let cpu_percent = if elapsed > 0.0 {
        ((cpu_after - cpu_before).max(0.0) / elapsed) * 100.0
    } else {
        0.0
    };
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
                children.push((pid, name, bytes));
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
        cpu_percent,
        children,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BudgetWindow {
    Panel,
    Main,
}

impl BudgetWindow {
    fn argument(self) -> &'static str {
        match self {
            Self::Panel => "panel",
            Self::Main => "main",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Panel => "quick panel",
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

fn open_window(exe: &Path, window: BudgetWindow, pid: u32, expected: usize) -> Result<()> {
    println!("budget: opening the {}", window.label());
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
    wait_for_windows(pid, expected, window)
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
    for (pid, name, bytes) in &sample.children {
        println!("    {name} (pid {pid}) {}", format_bytes(*bytes));
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

fn check_under(failures: &mut Vec<String>, label: &str, measured: u64, limit: u64) {
    if Limit::Under(limit).allows(measured) {
        println!(
            "  PASS {label}: {} is under the {} budget.",
            format_bytes(measured),
            format_bytes(limit)
        );
        return;
    }
    let message = if measured > limit {
        format!(
            "{label}: {} is over the {} budget by {}.",
            format_bytes(measured),
            format_bytes(limit),
            format_bytes(measured - limit)
        )
    } else {
        format!(
            "{label}: {} is not under the {} budget.",
            format_bytes(measured),
            format_bytes(limit)
        )
    };
    println!("  FAIL {message}");
    failures.push(message);
}

fn check_within(
    failures: &mut Vec<String>,
    label: &str,
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
            "  PASS {label}: {} is within {} of the first idle figure ({}).",
            format_bytes(measured),
            format_bytes(allowance),
            format_bytes(reference)
        );
        return;
    }
    let message = format!(
        "{label}: {} is {} above the first idle figure ({}); the allowance is {}.",
        format_bytes(measured),
        format_bytes(measured.saturating_sub(reference)),
        format_bytes(reference),
        format_bytes(allowance)
    );
    println!("  FAIL {message}");
    failures.push(message);
}

fn check_webview(failures: &mut Vec<String>, label: &str, count: usize) {
    if count == 0 {
        println!("  PASS {label}: no WebView2 processes are running.");
        return;
    }
    let message = if count == 1 {
        format!("{label}: 1 WebView2 process is still running; the budget requires none.")
    } else {
        format!("{label}: {count} WebView2 processes are still running; the budget requires none.")
    };
    println!("  FAIL {message}");
    failures.push(message);
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

#[allow(clippy::missing_errors_doc, clippy::too_many_lines)]
pub fn run(root: &Path, _args: &[String]) -> Result<()> {
    let exe = ensure_binary(root)?;
    ensure_not_running()?;
    let data_directory = webview_data_directory()?;
    println!("budget: measuring {}", exe.display());
    println!("budget: starting Snapline with --budget-run");
    let child = Command::new(&exe)
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

    let mut failures: Vec<String> = Vec::new();

    let idle = collect(pid, created, &data_directory)?;
    report("tray idle", &idle);
    check_under(&mut failures, "tray idle", idle.total_private, IDLE_LIMIT);
    check_webview(&mut failures, "tray idle", idle.webview);
    let first_idle = idle.total_private;

    open_window(&exe, BudgetWindow::Panel, pid, 1)?;
    let panel = collect(pid, created, &data_directory)?;
    report("quick panel", &panel);
    check_under(
        &mut failures,
        "quick panel",
        panel.total_private,
        PANEL_LIMIT,
    );
    close_windows(pid)?;
    println!(
        "budget: waiting {} s for the linger period and the WebView2 processes to exit",
        LINGER_WAIT.as_secs()
    );
    sleep(LINGER_WAIT);
    let after_panel = collect(pid, created, &data_directory)?;
    report("tray idle after the quick panel", &after_panel);

    open_window(&exe, BudgetWindow::Main, pid, 1)?;
    let main = collect(pid, created, &data_directory)?;
    report("main window", &main);
    check_under(&mut failures, "main window", main.total_private, MAIN_LIMIT);
    close_windows(pid)?;
    println!(
        "budget: waiting {} s for the linger period and the WebView2 processes to exit",
        LINGER_WAIT.as_secs()
    );
    sleep(LINGER_WAIT);
    let after_main = collect(pid, created, &data_directory)?;
    report("tray idle after the main window", &after_main);

    open_window(&exe, BudgetWindow::Main, pid, 1)?;
    open_window(&exe, BudgetWindow::Panel, pid, 2)?;
    let both = collect(pid, created, &data_directory)?;
    report("main window and quick panel", &both);
    check_under(
        &mut failures,
        "main window and quick panel",
        both.total_private,
        BOTH_LIMIT,
    );
    close_windows(pid)?;
    println!(
        "budget: waiting {} s for the linger period and the WebView2 processes to exit",
        LINGER_WAIT.as_secs()
    );
    sleep(LINGER_WAIT);
    let after_both = collect(pid, created, &data_directory)?;
    report("tray idle after every window", &after_both);
    check_under(
        &mut failures,
        "tray idle after every window",
        after_both.total_private,
        IDLE_LIMIT,
    );
    check_within(
        &mut failures,
        "tray idle after every window",
        after_both.total_private,
        first_idle,
        IDLE_DRIFT_LIMIT,
    );
    check_webview(
        &mut failures,
        "tray idle after every window",
        after_both.webview,
    );

    println!("budget: CPU is printed for the record; section 6 targets 0% when nothing changes.");

    app.stop(&exe);

    if failures.is_empty() {
        println!("budget: every measured check passed.");
        return Ok(());
    }
    for failure in &failures {
        eprintln!("  {failure}");
    }
    bail!("{} budget check(s) failed", failures.len());
}

#[cfg(test)]
mod tests {
    use super::{descendants, webview_children, Limit, ProcessRow};

    const DATA: &str = "C:\\Users\\tester\\AppData\\Local\\SnapLine\\WebView2";

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
}
