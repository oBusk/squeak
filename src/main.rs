//! Skateboard: find Keychron mice that speak the `0xFFC1` protocol, poll their battery once,
//! then listen for pushed reports (with a slow fallback poll). See `docs/protocol.md`.

use std::time::{Duration, Instant};
use std::{mem, ptr};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_GET_DEVICE_INTERFACE_LIST_PRESENT, CM_Get_Device_Interface_List_SizeW,
    CM_Get_Device_Interface_ListW, CR_SUCCESS,
};
use windows_sys::Win32::Devices::HumanInterfaceDevice::{
    HIDD_ATTRIBUTES, HIDP_CAPS, HIDP_REPORT_TYPE, HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS,
    HidD_FreePreparsedData, HidD_GetAttributes, HidD_GetHidGuid, HidD_GetPreparsedData,
    HidD_GetProductString, HidP_GetCaps, HidP_GetValueCaps, HidP_Input, HidP_Output,
    PHIDP_PREPARSED_DATA,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_IO_PENDING, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, SYSTEMTIME, WAIT_FAILED, WAIT_OBJECT_0,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, ReadFile,
    WriteFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::{
    CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
};
use windows_sys::core::GUID;

const KEYCHRON_VID: u16 = 0x3434;
const MOUSE_USAGE_PAGE: u16 = 0xFFC1;
const MOUSE_USAGE: u16 = 0x0001;
/// 1 byte report ID + 63 bytes data.
const REPORT_LEN: usize = 64;

const REPORT_COMMAND: u8 = 0xB3;
const REPORT_REPLY: u8 = 0xB4;
const REPORT_EVENT: u8 = 0xB6;
const CMD_STATUS: u8 = 0x06;
const EVENT_POWER: u8 = 0xE2;

const REPLY_TIMEOUT: Duration = Duration::from_secs(1);
/// Battery byte in pushes when the dongle has no mouse connected.
const NO_DATA: u8 = 0xFF;

fn main() {
    let mut devices = find_devices();
    if devices.is_empty() {
        log("no Keychron mouse channel found");
        std::process::exit(1);
    }

    for dev in &mut devices {
        log(&format!("found {}", dev.name));
        // Start reading before sending, so the reply can't be missed.
        if dev.start_read() {
            dev.request_status();
        }
    }
    // No periodic polling yet: the mouse pushes battery updates on its own. How regularly it
    // does so during normal use is still being measured (docs/protocol.md, `e2`).
    let poll_event = spawn_enter_listener();
    log("listening for pushes… (Enter to poll, Ctrl+C to stop)");

    while !devices.is_empty() {
        let timeout_ms = devices
            .iter()
            .filter_map(|d| d.awaiting_reply.map(|t| t + REPLY_TIMEOUT))
            .min()
            .map_or(INFINITE, |deadline| {
                let timeout = deadline.saturating_duration_since(Instant::now());
                u32::try_from(timeout.as_millis()).unwrap_or(INFINITE - 1)
            });

        let events: Vec<HANDLE> = devices
            .iter()
            .map(|d| d.read.hEvent)
            .chain([poll_event])
            .collect();
        let result =
            unsafe { WaitForMultipleObjects(events.len() as u32, events.as_ptr(), 0, timeout_ms) };
        if result == WAIT_FAILED {
            log(&format!("wait failed: error {}", unsafe { GetLastError() }));
            std::process::exit(1);
        }

        let index = result.wrapping_sub(WAIT_OBJECT_0) as usize;
        if index < devices.len() {
            let dev = &mut devices[index];
            let alive = match dev.finish_read() {
                Some(len) => {
                    dev.handle_report(len);
                    dev.start_read()
                }
                None => false,
            };
            if !alive {
                log(&format!("{}  disconnected", dev.name));
                devices.remove(index);
            }
        } else if index == devices.len() {
            for dev in &mut devices {
                dev.request_status();
            }
        }

        for dev in &mut devices {
            if dev
                .awaiting_reply
                .is_some_and(|t| t.elapsed() >= REPLY_TIMEOUT)
            {
                log(&format!("{}  poll  no reply", dev.name));
                dev.awaiting_reply = None;
            }
        }
    }
    log("no devices left");
}

/// Returns an auto-reset event that is signalled each time Enter is pressed.
fn spawn_enter_listener() -> HANDLE {
    let event = unsafe { CreateEventW(ptr::null(), 0, 0, ptr::null()) };
    // HANDLE is a raw pointer and not Send; event handles are safe to signal from any thread.
    let raw = event as usize;
    std::thread::spawn(move || {
        for _ in std::io::stdin().lines() {
            unsafe { SetEvent(raw as HANDLE) };
        }
    });
    event
}

struct Device {
    name: String,
    handle: HANDLE,
    read: OVERLAPPED,
    buf: [u8; REPORT_LEN],
    awaiting_reply: Option<Instant>,
}

impl Device {
    /// Boxed so the `OVERLAPPED` and buffer keep their address while a read is pending.
    fn open(path: &[u16], name: String) -> Option<Box<Self>> {
        let handle = open_path(path, GENERIC_READ | GENERIC_WRITE, FILE_FLAG_OVERLAPPED)?;
        let mut read: OVERLAPPED = unsafe { mem::zeroed() };
        read.hEvent = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        Some(Box::new(Self {
            name,
            handle,
            read,
            buf: [0; REPORT_LEN],
            awaiting_reply: None,
        }))
    }

    fn start_read(&mut self) -> bool {
        let ok = unsafe {
            ReadFile(
                self.handle,
                self.buf.as_mut_ptr(),
                REPORT_LEN as u32,
                ptr::null_mut(),
                &mut self.read,
            )
        };
        ok != 0 || unsafe { GetLastError() } == ERROR_IO_PENDING
    }

    fn finish_read(&mut self) -> Option<usize> {
        let mut len = 0;
        let ok = unsafe { GetOverlappedResult(self.handle, &self.read, &mut len, 0) };
        (ok != 0).then_some(len as usize)
    }

    fn request_status(&mut self) {
        let mut report = [0u8; REPORT_LEN];
        report[0] = REPORT_COMMAND;
        report[1] = CMD_STATUS;
        if write_report(self.handle, &report) {
            self.awaiting_reply = Some(Instant::now());
        } else {
            log(&format!("{}  poll  write failed", self.name));
        }
    }

    fn handle_report(&mut self, len: usize) {
        let report = &self.buf[..len];
        match *report {
            [REPORT_REPLY, CMD_STATUS, ..] if len > 20 => {
                self.awaiting_reply = None;
                let power = report[20];
                log(&format!(
                    "{}  poll  {}",
                    self.name,
                    battery(power & 0x7F, power & 0x80 != 0)
                ));
            }
            [REPORT_EVENT, EVENT_POWER, _, _, state, percent, ..] => {
                let status = if percent == NO_DATA {
                    "mouse asleep or disconnected".to_string()
                } else {
                    battery(percent, state == 0x03)
                };
                log(&format!(
                    "{}  push  {status}  [{}]",
                    self.name,
                    hex(&report[..8])
                ));
            }
            _ => log(&format!(
                "{}  other [{}]",
                self.name,
                hex(trim_zeros(report))
            )),
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            CancelIoEx(self.handle, &self.read);
            CloseHandle(self.handle);
            CloseHandle(self.read.hEvent);
        }
    }
}

/// Finds HID collections that are Keychron mouse channels. Nothing is sent to a collection
/// unless its VID, usage page, report sizes and declared report IDs all match, which rules
/// out e.g. the VIA collection (`0xFF60`) where command `0x06` resets the keymap.
#[allow(
    clippy::vec_box,
    reason = "pending reads must not move when the Vec shifts"
)]
fn find_devices() -> Vec<Box<Device>> {
    let mut devices = Vec::new();
    for path in hid_interface_paths() {
        // Access 0: query attributes and descriptor without being able to read or write.
        let Some(handle) = open_path(&path, 0, 0) else {
            continue;
        };
        let info = inspect(handle);
        let name = product_name(handle);
        unsafe { CloseHandle(handle) };

        let Some(info) = info else { continue };
        let label = format!("{name} (pid 0x{:04x})", info.pid);
        if info.is_mouse_channel() {
            if let Some(dev) = Device::open(&path, label) {
                devices.push(dev);
            }
        } else {
            log(&format!(
                "skip  {label}  page 0x{:04x}:0x{:04x}  in {} out {}  reports declared: {}",
                info.usage_page, info.usage, info.in_len, info.out_len, info.declares_reports
            ));
        }
    }
    devices
}

struct Info {
    pid: u16,
    usage_page: u16,
    usage: u16,
    in_len: u16,
    out_len: u16,
    declares_reports: bool,
}

impl Info {
    fn is_mouse_channel(&self) -> bool {
        self.usage_page == MOUSE_USAGE_PAGE
            && self.usage == MOUSE_USAGE
            && self.in_len as usize == REPORT_LEN
            && self.out_len as usize == REPORT_LEN
            && self.declares_reports
    }
}

/// Reads VID/PID and the parsed report descriptor. `None` if it isn't a Keychron device.
fn inspect(handle: HANDLE) -> Option<Info> {
    let mut attrs: HIDD_ATTRIBUTES = unsafe { mem::zeroed() };
    attrs.Size = mem::size_of::<HIDD_ATTRIBUTES>() as u32;
    if !unsafe { HidD_GetAttributes(handle, &mut attrs) } || attrs.VendorID != KEYCHRON_VID {
        return None;
    }

    let mut preparsed: PHIDP_PREPARSED_DATA = 0;
    if !unsafe { HidD_GetPreparsedData(handle, &mut preparsed) } {
        return None;
    }
    let mut caps: HIDP_CAPS = unsafe { mem::zeroed() };
    let info =
        (unsafe { HidP_GetCaps(preparsed, &mut caps) } == HIDP_STATUS_SUCCESS).then(|| Info {
            pid: attrs.ProductID,
            usage_page: caps.UsagePage,
            usage: caps.Usage,
            in_len: caps.InputReportByteLength,
            out_len: caps.OutputReportByteLength,
            declares_reports: declares_report(
                preparsed,
                HidP_Output,
                caps.NumberOutputValueCaps,
                REPORT_COMMAND,
            ) && declares_report(
                preparsed,
                HidP_Input,
                caps.NumberInputValueCaps,
                REPORT_REPLY,
            ),
        });
    unsafe { HidD_FreePreparsedData(preparsed) };
    info
}

fn declares_report(
    preparsed: PHIDP_PREPARSED_DATA,
    kind: HIDP_REPORT_TYPE,
    count: u16,
    report_id: u8,
) -> bool {
    let mut len = count;
    let mut caps: Vec<HIDP_VALUE_CAPS> = vec![unsafe { mem::zeroed() }; count as usize];
    let status = unsafe { HidP_GetValueCaps(kind, caps.as_mut_ptr(), &mut len, preparsed) };
    status == HIDP_STATUS_SUCCESS && caps[..len as usize].iter().any(|c| c.ReportID == report_id)
}

/// NUL-terminated device paths of all present HID collections.
fn hid_interface_paths() -> Vec<Vec<u16>> {
    let mut guid: GUID = unsafe { mem::zeroed() };
    unsafe { HidD_GetHidGuid(&mut guid) };

    let mut len = 0;
    let flags = CM_GET_DEVICE_INTERFACE_LIST_PRESENT;
    if unsafe { CM_Get_Device_Interface_List_SizeW(&mut len, &guid, ptr::null(), flags) }
        != CR_SUCCESS
    {
        return Vec::new();
    }
    let mut list = vec![0u16; len as usize];
    if unsafe { CM_Get_Device_Interface_ListW(&guid, ptr::null(), list.as_mut_ptr(), len, flags) }
        != CR_SUCCESS
    {
        return Vec::new();
    }

    list.split(|&c| c == 0)
        .filter(|s| !s.is_empty())
        .map(|s| s.iter().copied().chain([0]).collect())
        .collect()
}

fn open_path(path: &[u16], access: u32, flags: u32) -> Option<HANDLE> {
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            flags,
            ptr::null_mut(),
        )
    };
    (handle != INVALID_HANDLE_VALUE).then_some(handle)
}

fn product_name(handle: HANDLE) -> String {
    let mut buf = [0u16; 127];
    let ok = unsafe {
        HidD_GetProductString(
            handle,
            buf.as_mut_ptr().cast(),
            mem::size_of_val(&buf) as u32,
        )
    };
    if !ok {
        return "unknown".into();
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Writes one output report and waits for it to complete. Must not return while the write
/// is still pending, because `report` and the `OVERLAPPED` live on this stack frame.
fn write_report(handle: HANDLE, report: &[u8; REPORT_LEN]) -> bool {
    let mut ov: OVERLAPPED = unsafe { mem::zeroed() };
    ov.hEvent = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
    let mut written = 0;
    let ok = unsafe {
        let started = WriteFile(
            handle,
            report.as_ptr(),
            REPORT_LEN as u32,
            ptr::null_mut(),
            &mut ov,
        ) != 0
            || GetLastError() == ERROR_IO_PENDING;
        if started && WaitForSingleObject(ov.hEvent, 1000) != WAIT_OBJECT_0 {
            CancelIoEx(handle, &ov);
        }
        // Waits for completion (or cancellation) either way.
        started && GetOverlappedResult(handle, &ov, &mut written, 1) != 0
    };
    unsafe { CloseHandle(ov.hEvent) };
    ok && written as usize == REPORT_LEN
}

fn battery(percent: u8, charging: bool) -> String {
    let state = if charging { "charging" } else { "not charging" };
    format!("{percent:>3}%  {state}")
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn trim_zeros(bytes: &[u8]) -> &[u8] {
    let end = bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    &bytes[..end]
}

fn log(msg: &str) {
    let mut t: SYSTEMTIME = unsafe { mem::zeroed() };
    unsafe { GetLocalTime(&mut t) };
    println!("{:02}:{:02}:{:02}  {msg}", t.wHour, t.wMinute, t.wSecond);
}
