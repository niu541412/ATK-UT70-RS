//! UT70 application-mode HID reports. All offsets exclude the zero report ID.

pub const VID: u16 = 0x19F5;
pub const PID: u16 = 0x6666;
pub const VERSION: u8 = 0x02;
pub const FACE: u8 = 0x03;
pub const CV_DATA: u8 = 0x08;
pub const FREQUENCY: u8 = 0x70;
pub const FAST_CHARGE: u8 = 0x20;
pub const FAST_CHARGE_DESCRIPTION: u8 = 0x21;
pub const EMARKER_RESULT_SCREEN: u8 = 21;
pub const OTHER_SCREEN: u8 = 255;
pub const PROTOCOL_TYPES: [u8; 13] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13, 20];
pub const RATES: [u16; 6] = [4, 10, 50, 200, 500, 1000];
pub const SCREENS: &[(u8, &str, &str)] = &[
    (0, "基本测量", "Basic measurement"),
    (1, "波形显示", "Waveform"),
    (2, "数据记录", "Data logging"),
    (3, "功能应用", "Functions"),
    (4, "手动触发", "Manual trigger"),
    (5, "自动检测", "Auto detection"),
    (11, "监控模式", "Monitor mode"),
    (18, "线阻模式", "Line resistance"),
    (19, "DASH 检测", "DASH detection"),
    (20, "E-Marker 检测", "E-Marker detection"),
    (29, "设置", "Settings"),
];

pub fn screen_names(screen: u8) -> (&'static str, &'static str) {
    if screen == EMARKER_RESULT_SCREEN {
        ("E-Marker 结果", "E-Marker result")
    } else {
        SCREENS
            .iter()
            .find(|(id, _, _)| *id == screen)
            .map(|(_, chinese, english)| (*chinese, *english))
            .unwrap_or(("其他", "Other"))
    }
}

pub fn selected_screen_names(screen: u8) -> (&'static str, &'static str) {
    SCREENS
        .iter()
        .find(|(id, _, _)| *id == screen)
        .map(|(_, chinese, english)| (*chinese, *english))
        .unwrap_or(("其他", "Other"))
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub command: u8,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub voltage: f32,
    pub current: f32,
    pub dp: f32,
    pub dm: f32,
}

#[derive(Clone, Debug)]
pub struct VersionInfo {
    pub application: u8,
    pub hardware: u8,
    pub bootloader: u8,
    pub serial: String,
    pub boot_mode: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtocolReport {
    Mask {
        kind: u8,
        mask: u32,
    },
    Description {
        kind: u8,
        supported: bool,
        text: String,
    },
}

pub fn protocol_report(frame: &Frame) -> Result<ProtocolReport, String> {
    match frame.command {
        FAST_CHARGE if frame.payload.len() >= 6 => Ok(ProtocolReport::Mask {
            kind: frame.payload[1],
            mask: u32::from_le_bytes(frame.payload[2..6].try_into().unwrap()),
        }),
        FAST_CHARGE_DESCRIPTION if frame.payload.len() >= 2 => {
            let (decoded, _, _) = encoding_rs::GBK.decode(&frame.payload[2..]);
            Ok(ProtocolReport::Description {
                kind: frame.payload[0],
                supported: frame.payload[1] == 1,
                text: decoded.trim_end_matches('\0').to_owned(),
            })
        }
        _ => Err("not a supported protocol scan report".into()),
    }
}

pub fn protocol_name(kind: u8) -> &'static str {
    match kind {
        0 => "BC1.2",
        1 => "QC 2.0",
        2 => "QC 3.0",
        3 => "FCP",
        4 => "SCP",
        5 => "AFC",
        6 => "PE 1.0",
        7 => "PE 2.0",
        8 => "VOOC",
        9 => "PD",
        10 => "DASH",
        11 => "E-Marker",
        12 => "Apple 2.4A",
        13 => "VFCP",
        20 => "UFCS",
        _ => "Unknown",
    }
}

pub fn encode(command: u8, payload: &[u8]) -> Result<[u8; 65], String> {
    if payload.len() > 59 {
        return Err("application payload exceeds 59 bytes".into());
    }
    let mut out = [0u8; 65];
    out[1] = 0xEE;
    out[2] = command;
    out[3] = payload.len() as u8;
    out[4..4 + payload.len()].copy_from_slice(payload);
    let checksum_at = 4 + payload.len();
    out[checksum_at] = out[1..checksum_at]
        .iter()
        .fold(0u8, |acc, x| acc.wrapping_add(*x));
    out[checksum_at + 1] = 0xFF;
    Ok(out)
}

pub fn decode(report: &[u8], expected: Option<(u8, u8)>) -> Result<Frame, String> {
    let data = if report.first() == Some(&0) && report.get(1) == Some(&0xEE) {
        &report[1..]
    } else {
        report
    };
    if data.len() < 4 || data[0] != 0xEE {
        return Err("missing application frame header".into());
    }
    let n = data[2] as usize;
    let mut check_at = 3 + n;
    if check_at >= data.len() {
        return Err("truncated application frame".into());
    }
    let mut computed = data[..check_at]
        .iter()
        .fold(0u8, |acc, x| acc.wrapping_add(*x));
    // 实机 0x21 文本帧的长度字段比实际载荷少 1；仅在下一字节校验成立时接受。
    if data[1] == FAST_CHARGE_DESCRIPTION && data[check_at] != computed {
        let longer = check_at + 1;
        if longer < data.len() {
            let next_sum = computed.wrapping_add(data[check_at]);
            if data[longer] == next_sum {
                check_at = longer;
                computed = next_sum;
            }
        }
    }
    let special_rate_reply = expected.is_some_and(|(cmd, sent_checksum)| {
        cmd == FREQUENCY && data[1] == cmd && data[check_at] == sent_checksum
    });
    if data[check_at] != computed && !special_rate_reply {
        return Err("application frame checksum mismatch".into());
    }
    if data.get(check_at + 1).is_some_and(|end| *end != 0xFF) {
        return Err("application frame terminator mismatch".into());
    }
    Ok(Frame {
        command: data[1],
        payload: data[3..check_at].to_vec(),
    })
}

pub fn samples(frame: &Frame, rate: u16) -> Result<Vec<Sample>, String> {
    if frame.command != CV_DATA || frame.payload.len() != 60 {
        return Err("expected a 60-byte measurement payload".into());
    }
    if !RATES.contains(&rate) {
        return Err("unknown sample rate".into());
    }
    let count = if rate >= 500 { 5 } else { 1 };
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let voltage = f32::from_le_bytes(frame.payload[4 * i..4 * i + 4].try_into().unwrap());
        let current = f32::from_le_bytes(frame.payload[20 + 4 * i..24 + 4 * i].try_into().unwrap());
        if !voltage.is_finite() || !current.is_finite() {
            return Err("measurement contains a non-finite value".into());
        }
        let dp = u16::from_le_bytes(frame.payload[40 + 2 * i..42 + 2 * i].try_into().unwrap())
            as f32
            / 10_000.0;
        let dm = u16::from_le_bytes(frame.payload[50 + 2 * i..52 + 2 * i].try_into().unwrap())
            as f32
            / 10_000.0;
        out.push(Sample {
            voltage,
            current,
            dp,
            dm,
        });
    }
    Ok(out)
}

pub fn version(frame: &Frame) -> Result<VersionInfo, String> {
    if frame.command != VERSION || frame.payload.len() < 13 {
        return Err("incomplete version response".into());
    }
    let serial = String::from_utf8_lossy(&frame.payload[4..12])
        .trim_end_matches('\0')
        .to_owned();
    Ok(VersionInfo {
        bootloader: frame.payload[0],
        application: frame.payload[1],
        hardware: frame.payload[2],
        serial,
        boot_mode: frame.payload[12] == 1,
    })
}

pub fn rate(frame: &Frame) -> Result<u16, String> {
    if frame.command != FREQUENCY || frame.payload.len() < 2 {
        return Err("incomplete frequency response".into());
    }
    RATES
        .get(frame.payload[1] as usize)
        .copied()
        .ok_or_else(|| "unknown rate index".into())
}

pub fn rate_payload(rate: u16) -> Result<[u8; 4], String> {
    let index = RATES
        .iter()
        .position(|candidate| *candidate == rate)
        .ok_or("unsupported rate")?;
    let [lo, hi] = rate.to_le_bytes();
    Ok([1, index as u8, lo, hi])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_frame_matches_capture() {
        let report = encode(VERSION, &[0, 0]).unwrap();
        assert_eq!(&report[..8], &[0, 0xEE, 0x02, 0x02, 0, 0, 0xF2, 0xFF]);
        assert_eq!(decode(&report[1..], None).unwrap().command, VERSION);
    }

    #[test]
    fn line_resistance_command_and_unknown_screen_label() {
        let report = encode(FACE, &[18, 1, 1, 1]).unwrap();
        assert_eq!(
            &report[..10],
            &[0, 0xEE, 0x03, 0x04, 0x12, 1, 1, 1, 0x0A, 0xFF]
        );
        assert_eq!(selected_screen_names(18), ("线阻模式", "Line resistance"));
        assert_eq!(selected_screen_names(17), ("其他", "Other"));
        assert_eq!(selected_screen_names(21), ("其他", "Other"));
        assert_eq!(selected_screen_names(OTHER_SCREEN), ("其他", "Other"));
    }

    #[test]
    fn frequency_reply_accepts_only_matching_echoed_checksum() {
        let reply = [0xEE, 0x70, 4, 1, 4, 0, 0, 0x62, 0xFF];
        assert!(decode(&reply, None).is_err());
        assert_eq!(
            rate(&decode(&reply, Some((FREQUENCY, 0x62))).unwrap()).unwrap(),
            500
        );
        assert!(decode(&reply, Some((FREQUENCY, 0x63))).is_err());
    }

    #[test]
    fn manually_selected_rates_match_meter_reports() {
        let reports = [
            ([0xEE, 0x70, 4, 0, 1, 0x0A, 0, 0x6D, 0xFF], 10),
            ([0xEE, 0x70, 4, 0, 2, 0x32, 0, 0x96, 0xFF], 50),
            ([0xEE, 0x70, 4, 0, 3, 0xC8, 0, 0x2D, 0xFF], 200),
            ([0xEE, 0x70, 4, 1, 4, 0xF4, 1, 0x5C, 0xFF], 500),
            ([0xEE, 0x70, 4, 1, 5, 0xE8, 3, 0x53, 0xFF], 1000),
            ([0xEE, 0x70, 4, 0, 0, 4, 0, 0x66, 0xFF], 4),
        ];
        for (report, expected_rate) in reports {
            assert_eq!(
                rate(&decode(&report, None).unwrap()).unwrap(),
                expected_rate
            );
        }
    }

    #[test]
    fn measurement_without_terminator_is_valid() {
        let mut report = [0u8; 64];
        report[..3].copy_from_slice(&[0xEE, CV_DATA, 60]);
        report[3..7].copy_from_slice(&5.0f32.to_le_bytes());
        report[23..27].copy_from_slice(&1.0f32.to_le_bytes());
        report[63] = report[..63].iter().fold(0u8, |acc, x| acc.wrapping_add(*x));
        let decoded = decode(&report, None).unwrap();
        let points = samples(&decoded, 4).unwrap();
        assert_eq!(points[0].voltage, 5.0);
        assert_eq!(points[0].current, 1.0);
    }

    #[test]
    fn protocol_scan_frames_match_windows_capture() {
        let request = encode(FAST_CHARGE, &[0; 4]).unwrap();
        assert_eq!(&request[1..10], &[0xEE, 0x20, 4, 0, 0, 0, 0, 0x12, 0xFF]);

        let description = [
            0xEE, 0x21, 0x0E, 9, 0, b'P', b'D', b' ', b'n', b'o', b't', b' ', b'e', b'x', b'i',
            b't', 13, 10, 0x1C, 0xFF,
        ];
        assert_eq!(
            protocol_report(&decode(&description, None).unwrap()).unwrap(),
            ProtocolReport::Description {
                kind: 9,
                supported: false,
                text: "PD not exit\r\n".into()
            }
        );

        let mask = [0xEE, 0x20, 7, 0, 3, 0x84, 0x48, 0, 0, 1, 0xE5, 0xFF];
        assert_eq!(
            protocol_report(&decode(&mask, None).unwrap()).unwrap(),
            ProtocolReport::Mask {
                kind: 3,
                mask: 0x0000_4884
            }
        );
    }
}
