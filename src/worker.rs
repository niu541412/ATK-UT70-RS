//! 独占 HID 句柄；界面线程不直接读写设备。

use crate::{firmware, i18n::Message, protocol};
use hidapi::{HidApi, HidDevice};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

pub enum Command {
    Connect,
    Disconnect,
    SetRate(u16),
    SetScreen(u8),
    ScanProtocols,
    StartCsv(PathBuf),
    StopCsv,
    Flash(PathBuf),
    Quit,
}

pub enum Event {
    Status(Message),
    Connected(Option<protocol::VersionInfo>, u16, bool),
    Bootloader(protocol::VersionInfo),
    Disconnected,
    Rate(u16),
    Screen(u8),
    ScreenCommandSent(u8),
    ProtocolScanStarted,
    ProtocolReport(protocol::ProtocolReport),
    ProtocolScanFinished,
    Samples(Vec<TimedSample>),
    Recording(bool),
    FlashProgress(usize, usize),
    FlashDone,
    Error(Message),
}

fn send_status(tx: &Sender<Event>, chinese: impl Into<String>, english: impl Into<String>) {
    let _ = tx.send(Event::Status(Message::new(chinese, english)));
}

fn send_error(tx: &Sender<Event>, chinese: impl Into<String>, english: impl Into<String>) {
    let _ = tx.send(Event::Error(Message::new(chinese, english)));
}

fn send_failure(tx: &Sender<Event>, detail: &str) {
    send_error(
        tx,
        format!("操作失败：{detail}"),
        format!("Operation failed: {detail}"),
    );
}

#[derive(Clone, Copy)]
pub struct TimedSample {
    pub seconds: f64,
    pub sample: protocol::Sample,
}

#[derive(Default)]
struct SampleClock {
    last_seconds: Option<f64>,
}

impl SampleClock {
    fn stamp(
        &mut self,
        samples: Vec<protocol::Sample>,
        rate: u16,
        received_at: f64,
    ) -> Vec<TimedSample> {
        let count = samples.len();
        let interval = 1.0 / f64::from(rate);
        let first = received_at - (count.saturating_sub(1) as f64) * interval;
        let first = self
            .last_seconds
            .map_or(first, |last| first.max(last + interval));
        let batch = samples
            .into_iter()
            .enumerate()
            .map(|(index, sample)| TimedSample {
                seconds: first + index as f64 * interval,
                sample,
            })
            .collect::<Vec<_>>();
        self.last_seconds = batch.last().map(|sample| sample.seconds);
        batch
    }
}

pub fn run(rx: Receiver<Command>, tx: Sender<Event>) {
    let start = Instant::now();
    let mut sample_clock = SampleClock::default();
    let mut device: Option<HidDevice> = None;
    let mut rate = 4u16;
    let mut csv: Option<BufWriter<File>> = None;
    let mut last_flush = Instant::now();
    let mut scan_deadline: Option<Instant> = None;
    let mut scan_last_report: Option<Instant> = None;
    loop {
        if let Some(deadline) = scan_deadline {
            if Instant::now() >= deadline
                || scan_last_report.is_some_and(|last| last.elapsed() >= Duration::from_secs(18))
            {
                scan_deadline = None;
                scan_last_report = None;
                let _ = tx.send(Event::ProtocolScanFinished);
            }
        }
        match rx.try_recv() {
            Ok(Command::Quit) | Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            Ok(Command::Connect) => {
                if device.is_some() {
                    continue;
                }
                let result = (|| -> Result<Option<HidDevice>, String> {
                    let api = HidApi::new().map_err(|e| e.to_string())?;
                    let hid = api
                        .open(protocol::VID, protocol::PID)
                        .map_err(|e| e.to_string())?;
                    let version = request(
                        &hid,
                        protocol::VERSION,
                        &[0, 0],
                        2,
                        &mut rate,
                        start,
                        &mut sample_clock,
                        &tx,
                    )
                    .and_then(|f| protocol::version(&f));
                    match version {
                        Ok(v) if v.boot_mode => {
                            let _ = tx.send(Event::Bootloader(v));
                            Ok(Some(hid))
                        }
                        maybe_version => {
                            if let Err(e) = &maybe_version {
                                send_status(
                                    &tx,
                                    format!("版本查询失败，仍尝试接收测量数据：{e}"),
                                    format!(
                                        "Version query failed; still trying to receive measurements: {e}"
                                    ),
                                );
                            }
                            let mut rate_confirmed = false;
                            if let Ok(frame) = request(
                                &hid,
                                protocol::FREQUENCY,
                                &[0; 4],
                                2,
                                &mut rate,
                                start,
                                &mut sample_clock,
                                &tx,
                            ) {
                                if let Ok(current) = protocol::rate(&frame) {
                                    rate = current;
                                    rate_confirmed = true;
                                }
                            }
                            let _ =
                                tx.send(Event::Connected(maybe_version.ok(), rate, rate_confirmed));
                            if let Ok(frame) = request(
                                &hid,
                                protocol::FACE,
                                &[0; 4],
                                1,
                                &mut rate,
                                start,
                                &mut sample_clock,
                                &tx,
                            ) {
                                if let Some(screen) = frame.payload.first() {
                                    let _ = tx.send(Event::Screen(*screen));
                                }
                            }
                            Ok(Some(hid))
                        }
                    }
                })();
                match result {
                    Ok(opened) => device = opened,
                    Err(e) => {
                        send_error(
                            &tx,
                            format!("连接失败：{e}"),
                            format!("Connection failed: {e}"),
                        );
                    }
                }
            }
            Ok(Command::Disconnect) => {
                device = None;
                sample_clock = SampleClock::default();
                scan_deadline = None;
                scan_last_report = None;
                if let Some(mut writer) = csv.take() {
                    let _ = writer.flush();
                    let _ = tx.send(Event::Recording(false));
                }
                let _ = tx.send(Event::Disconnected);
            }
            Ok(Command::SetRate(requested)) => {
                let result = device
                    .as_ref()
                    .ok_or("device is not connected".to_string())
                    .and_then(|hid| {
                        let payload = protocol::rate_payload(requested)?;
                        let frame = request(
                            hid,
                            protocol::FREQUENCY,
                            &payload,
                            2,
                            &mut rate,
                            start,
                            &mut sample_clock,
                            &tx,
                        )?;
                        protocol::rate(&frame)
                    });
                match result {
                    Ok(actual) => {
                        rate = actual;
                        let _ = tx.send(Event::Rate(actual));
                    }
                    Err(e) => {
                        send_failure(&tx, &e);
                    }
                }
            }
            Ok(Command::SetScreen(screen)) => {
                if !protocol::SCREENS.iter().any(|(id, _, _)| *id == screen) {
                    send_error(
                        &tx,
                        "该界面尚未确认适合远程切换",
                        "This screen has not been verified for remote switching",
                    );
                    continue;
                }
                // These modes can start a workflow or disrupt HID, so do not wait for a query.
                if matches!(screen, 4 | 5 | 18 | 20) {
                    let result = device
                        .as_ref()
                        .ok_or("device is not connected".to_string())
                        .and_then(|hid| {
                            write_hid(hid, &protocol::encode(protocol::FACE, &[screen, 1, 1, 1])?)
                        });
                    match result {
                        Ok(()) => {
                            let _ = tx.send(Event::ScreenCommandSent(screen));
                        }
                        Err(e) => send_failure(&tx, &e),
                    }
                    continue;
                }
                let result = device
                    .as_ref()
                    .ok_or("device is not connected".to_string())
                    .and_then(|hid| {
                        request(
                            hid,
                            protocol::FACE,
                            &[screen, 1, 1, 1],
                            2,
                            &mut rate,
                            start,
                            &mut sample_clock,
                            &tx,
                        )?;
                        let frame = request(
                            hid,
                            protocol::FACE,
                            &[0; 4],
                            2,
                            &mut rate,
                            start,
                            &mut sample_clock,
                            &tx,
                        )?;
                        frame
                            .payload
                            .first()
                            .copied()
                            .ok_or("empty screen response".into())
                    });
                match result {
                    Ok(actual) if actual == screen => {
                        let _ = tx.send(Event::Screen(actual));
                    }
                    Ok(actual) => {
                        send_error(
                            &tx,
                            format!("设备当前报告的界面为 {actual}，不是 {screen}"),
                            format!("Meter reports screen {actual}, not {screen}"),
                        );
                    }
                    Err(e) => {
                        send_failure(&tx, &e);
                    }
                }
            }
            Ok(Command::ScanProtocols) => {
                if scan_deadline.is_some() {
                    continue;
                }
                let result = device
                    .as_ref()
                    .ok_or("device is not connected".to_string())
                    .and_then(|hid| {
                        write_hid(hid, &protocol::encode(protocol::FAST_CHARGE, &[0; 4])?)
                    });
                match result {
                    Ok(()) => {
                        scan_deadline = Some(Instant::now() + Duration::from_secs(120));
                        scan_last_report = None;
                        let _ = tx.send(Event::ProtocolScanStarted);
                    }
                    Err(e) => send_failure(&tx, &e),
                }
            }
            Ok(Command::StartCsv(path)) => {
                if device.is_none() {
                    send_error(&tx, "请先连接设备", "Connect the meter first");
                    continue;
                }
                if csv.is_some() {
                    send_error(
                        &tx,
                        "正在记录，请先停止",
                        "Stop the current recording first",
                    );
                    continue;
                }
                match File::create(&path) {
                    Ok(file) => {
                        let mut writer = BufWriter::new(file);
                        match writer.write_all(b"time_s,voltage_v,current_a,power_w,dp_v,dm_v\n") {
                            Ok(()) => {
                                csv = Some(writer);
                                last_flush = Instant::now();
                                let _ = tx.send(Event::Recording(true));
                            }
                            Err(e) => {
                                send_error(
                                    &tx,
                                    format!("CSV 写入失败：{e}"),
                                    format!("CSV write failed: {e}"),
                                );
                            }
                        }
                    }
                    Err(e) => {
                        send_error(
                            &tx,
                            format!("无法创建 CSV：{e}"),
                            format!("Cannot create CSV: {e}"),
                        );
                    }
                }
            }
            Ok(Command::StopCsv) => {
                if let Some(mut writer) = csv.take() {
                    let _ = writer.flush();
                }
                let _ = tx.send(Event::Recording(false));
            }
            Ok(Command::Flash(path)) => {
                device = None;
                scan_deadline = None;
                scan_last_report = None;
                if let Some(mut writer) = csv.take() {
                    let _ = writer.flush();
                    let _ = tx.send(Event::Recording(false));
                }
                let _ = tx.send(Event::Disconnected);
                send_status(
                    &tx,
                    "正在校验固件和升级模式…",
                    "Checking firmware and upgrade mode…",
                );
                match flash(&path, &tx, start) {
                    Ok(_) => {
                        let _ = tx.send(Event::FlashDone);
                    }
                    Err(e) => {
                        send_error(
                            &tx,
                            format!("升级已停止：{e}"),
                            format!("Upgrade stopped: {e}"),
                        );
                    }
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if let Some(hid) = device.as_ref() {
                    let mut buf = [0u8; 64];
                    match hid.read_timeout(&mut buf, 30) {
                        Ok(0) => {}
                        Ok(n) => {
                            if let Ok(frame) = protocol::decode(&buf[..n], None) {
                                observe_control_frame(&frame, &mut rate, &tx);
                                if scan_deadline.is_some() {
                                    if let Ok(report) = protocol::protocol_report(&frame) {
                                        scan_last_report = Some(Instant::now());
                                        let _ = tx.send(Event::ProtocolReport(report));
                                    }
                                }
                            }
                            if let Some(batch) =
                                process_sample_report(&buf[..n], rate, start, &mut sample_clock)
                            {
                                if let Some(writer) = csv.as_mut() {
                                    if let Err(e) = write_csv(writer, &batch) {
                                        csv = None;
                                        let _ = tx.send(Event::Recording(false));
                                        send_error(
                                            &tx,
                                            format!("CSV 写入失败：{e}"),
                                            format!("CSV write failed: {e}"),
                                        );
                                    } else if last_flush.elapsed() >= Duration::from_secs(1) {
                                        if let Err(e) = writer.flush() {
                                            csv = None;
                                            let _ = tx.send(Event::Recording(false));
                                            send_error(
                                                &tx,
                                                format!("CSV 刷盘失败：{e}"),
                                                format!("CSV flush failed: {e}"),
                                            );
                                        }
                                        last_flush = Instant::now();
                                    }
                                }
                                let _ = tx.send(Event::Samples(batch));
                            }
                        }
                        Err(e) => {
                            device = None;
                            sample_clock = SampleClock::default();
                            scan_deadline = None;
                            scan_last_report = None;
                            if let Some(mut writer) = csv.take() {
                                let _ = writer.flush();
                                let _ = tx.send(Event::Recording(false));
                            }
                            let _ = tx.send(Event::Disconnected);
                            send_error(
                                &tx,
                                format!("HID 已断开：{e}"),
                                format!("HID disconnected: {e}"),
                            );
                        }
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(30));
                }
            }
        }
    }
}

fn write_csv(writer: &mut BufWriter<File>, batch: &[TimedSample]) -> std::io::Result<()> {
    for item in batch {
        let s = item.sample;
        writeln!(
            writer,
            "{:.6},{:.8},{:.8},{:.8},{:.6},{:.6}",
            item.seconds,
            s.voltage,
            s.current,
            s.voltage * s.current,
            s.dp,
            s.dm
        )?;
    }
    Ok(())
}

fn process_sample_report(
    report: &[u8],
    rate: u16,
    start: Instant,
    sample_clock: &mut SampleClock,
) -> Option<Vec<TimedSample>> {
    let frame = protocol::decode(report, None).ok()?;
    if frame.command != protocol::CV_DATA {
        return None;
    }
    let points = protocol::samples(&frame, rate).ok()?;
    let now = start.elapsed().as_secs_f64();
    Some(sample_clock.stamp(points, rate, now))
}

fn observe_control_frame(frame: &protocol::Frame, rate: &mut u16, tx: &Sender<Event>) {
    match frame.command {
        protocol::FACE => {
            if let Some(screen) = frame.payload.first() {
                let _ = tx.send(Event::Screen(*screen));
            }
        }
        protocol::FREQUENCY => {
            if let Ok(actual) = protocol::rate(frame) {
                if *rate != actual {
                    *rate = actual;
                    let _ = tx.send(Event::Rate(actual));
                }
            }
        }
        _ => {}
    }
}

fn request(
    hid: &HidDevice,
    command: u8,
    payload: &[u8],
    timeout_secs: u64,
    rate: &mut u16,
    start: Instant,
    sample_clock: &mut SampleClock,
    tx: &Sender<Event>,
) -> Result<protocol::Frame, String> {
    let report = protocol::encode(command, payload)?;
    write_hid(hid, &report)?;
    let sent_checksum = report[4 + payload.len()];
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    while Instant::now() < deadline {
        let mut buf = [0u8; 64];
        let n = hid.read_timeout(&mut buf, 100).map_err(|e| e.to_string())?;
        if n == 0 {
            continue;
        }
        if let Ok(frame) = protocol::decode(&buf[..n], Some((command, sent_checksum))) {
            if frame.command == command {
                return Ok(frame);
            }
            observe_control_frame(&frame, rate, tx);
            if frame.command == protocol::CV_DATA {
                if let Some(batch) = process_sample_report(&buf[..n], *rate, start, sample_clock) {
                    let _ = tx.send(Event::Samples(batch));
                }
            }
        }
    }
    Err(format!("command 0x{command:02X} timed out"))
}

fn write_hid(hid: &HidDevice, report: &[u8; 65]) -> Result<(), String> {
    let n = hid.write(report).map_err(|e| e.to_string())?;
    if n != report.len() {
        return Err(format!("short HID write: {n}/65"));
    }
    Ok(())
}

fn boot_transact(
    hid: &HidDevice,
    command: u8,
    sequence: u8,
    payload: &[u8],
    timeout_secs: u64,
) -> Result<firmware::Reply, String> {
    write_hid(hid, &firmware::encode(command, sequence, payload)?)?;
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    while Instant::now() < deadline {
        let mut buf = [0u8; 64];
        let n = hid.read_timeout(&mut buf, 100).map_err(|e| e.to_string())?;
        if n == 0 {
            continue;
        }
        let reply = firmware::decode(&buf[..n])?;
        if reply.command != command || reply.sequence != sequence {
            return Err(format!(
                "bootloader reply command or sequence mismatch: 0x{command:02X}/{sequence}"
            ));
        }
        if command != 0x10 && reply.payload != [1] {
            return Err(format!(
                "meter rejected command 0x{command:02X}: {:02X?}",
                reply.payload
            ));
        }
        return Ok(reply);
    }
    Err(format!("upgrade command 0x{command:02X} timed out"))
}

fn flash(
    path: &PathBuf,
    tx: &Sender<Event>,
    start: Instant,
) -> Result<protocol::VersionInfo, String> {
    let mut rate = 4;
    let mut sample_clock = SampleClock::default();
    let fw = firmware::Firmware::read(path)?;
    let api = HidApi::new().map_err(|e| e.to_string())?;
    let hid = api
        .open(protocol::VID, protocol::PID)
        .map_err(|e| e.to_string())?;
    let mode = protocol::version(&request(
        &hid,
        protocol::VERSION,
        &[0, 0],
        2,
        &mut rate,
        start,
        &mut sample_clock,
        tx,
    )?)?;
    if !mode.boot_mode {
        return Err("meter is not in upgrade mode; hold the right button while reconnecting the side HID cable".into());
    }
    let info = boot_transact(&hid, 0x10, 0, &[], 2)?;
    if info.payload.len() != 40 {
        return Err("invalid bootloader device info length".into());
    }
    let name = info.payload[..16]
        .split(|b| *b == 0)
        .next()
        .unwrap_or_default();
    let hardware = u16::from_le_bytes(info.payload[16..18].try_into().unwrap());
    if name != b"UT70" || hardware != fw.hardware {
        return Err(format!(
            "firmware hardware version {} does not match meter version {}",
            fw.hardware, hardware
        ));
    }
    let total = fw.image().len().div_ceil(firmware::CHUNK_SIZE);
    send_status(
        tx,
        format!(
            "正在烧录 {} 字节，目标固件 V{}",
            fw.image_size, fw.application
        ),
        format!(
            "Flashing {} bytes, target app V{}",
            fw.image_size, fw.application
        ),
    );
    boot_transact(&hid, 0x11, 0, fw.header(), 2)?;
    boot_transact(&hid, 0x12, 0, &[], 2)?;
    for (index, chunk) in fw.image().chunks(firmware::CHUNK_SIZE).enumerate() {
        boot_transact(&hid, 0x13, index as u8, chunk, 2)?;
        if index == 0 || (index + 1) % 53 == 0 || index + 1 == total {
            let _ = tx.send(Event::FlashProgress(index + 1, total));
        }
    }
    boot_transact(&hid, 0x14, 0, &[], 10)?;
    drop(hid);
    send_status(
        tx,
        "传输完成，等待仪表重启并校验版本…",
        "Transfer finished. Waiting for the meter to reboot and verifying its version…",
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Ok(api) = HidApi::new() {
            if let Ok(device) = api.open(protocol::VID, protocol::PID) {
                if let Ok(frame) = request(
                    &device,
                    protocol::VERSION,
                    &[0, 0],
                    2,
                    &mut rate,
                    start,
                    &mut sample_clock,
                    tx,
                ) {
                    if let Ok(version) = protocol::version(&frame) {
                        if !version.boot_mode {
                            if version.application as u16 != fw.application {
                                return Err("application version after reboot does not match the firmware file".into());
                            }
                            return Ok(version);
                        }
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err("transfer ended, but the meter did not return to application mode".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn bursty_high_rate_reports_keep_strictly_increasing_timestamps() {
        let sample = protocol::Sample {
            voltage: 5.0,
            current: 1.0,
            dp: 0.0,
            dm: 0.0,
        };
        let mut clock = SampleClock::default();
        let first = clock.stamp(vec![sample; 5], 1000, 1.000);
        let second = clock.stamp(vec![sample; 5], 1000, 1.003);
        let third = clock.stamp(vec![sample; 5], 1000, 1.020);
        let all = [first, second, third].concat();
        assert!(all.windows(2).all(|pair| pair[0].seconds < pair[1].seconds));
        assert!((all[5].seconds - all[4].seconds - 0.001).abs() < 1e-9);
        assert!((all.last().unwrap().seconds - 1.020).abs() < 1e-9);
    }

    #[test]
    fn manual_screen_and_rate_reports_update_worker_state() {
        let (tx, rx) = mpsc::channel();
        let mut rate = 4;
        let screen_report = protocol::encode(protocol::FACE, &[17, 0, 0, 0]).unwrap();
        let screen = protocol::decode(&screen_report[1..], None).unwrap();
        observe_control_frame(&screen, &mut rate, &tx);
        assert!(matches!(rx.try_recv(), Ok(Event::Screen(17))));

        let rate_report = [0xEE, 0x70, 4, 0, 3, 0xC8, 0, 0x2D, 0xFF];
        let frequency = protocol::decode(&rate_report, None).unwrap();
        observe_control_frame(&frequency, &mut rate, &tx);
        assert_eq!(rate, 200);
        assert!(matches!(rx.try_recv(), Ok(Event::Rate(200))));

        observe_control_frame(&frequency, &mut rate, &tx);
        assert!(rx.try_recv().is_err());
    }
}
