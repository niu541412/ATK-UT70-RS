//! Read-only .atk validation and bootloader report framing.

use std::fs;
use std::path::Path;

pub const CHUNK_SIZE: usize = 58;

#[derive(Clone, Debug)]
pub struct Firmware {
    pub bytes: Vec<u8>,
    pub hardware: u16,
    pub application: u16,
    pub image_size: usize,
    pub build_date: String,
}

impl Firmware {
    pub fn read(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path).map_err(|e| format!("cannot read firmware: {e}"))?;
        if bytes.len() < 32 {
            return Err("firmware is shorter than the 32-byte header".into());
        }
        let name = bytes[..16].split(|b| *b == 0).next().unwrap_or_default();
        if name != b"UT70" {
            return Err("firmware device type is not UT70".into());
        }
        if bytes[16] as usize != CHUNK_SIZE {
            return Err("firmware chunk size differs from the verified 58-byte protocol".into());
        }
        let hardware = u16::from_le_bytes([bytes[18], bytes[19]]);
        let application = u16::from_le_bytes([bytes[20], bytes[21]]);
        let image_size = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
        if image_size == 0 || image_size != bytes.len() - 32 {
            return Err("firmware image length does not match the file".into());
        }
        let year = u16::from_le_bytes([bytes[28], bytes[29]]);
        let build_date = format!("{year:04}-{:02}-{:02}", bytes[30], bytes[31]);
        Ok(Self {
            bytes,
            hardware,
            application,
            image_size,
            build_date,
        })
    }

    pub fn header(&self) -> &[u8] {
        &self.bytes[..32]
    }
    pub fn image(&self) -> &[u8] {
        &self.bytes[32..]
    }
}

pub fn crc16_modbus(data: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for byte in data {
        crc ^= *byte as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

pub fn encode(command: u8, sequence: u8, payload: &[u8]) -> Result<[u8; 65], String> {
    if payload.len() > CHUNK_SIZE {
        return Err("bootloader payload exceeds 58 bytes".into());
    }
    let mut out = [0u8; 65];
    out[1..5].copy_from_slice(&[0xFB, command, sequence, payload.len() as u8]);
    out[5..5 + payload.len()].copy_from_slice(payload);
    let crc = crc16_modbus(&out[1..5 + payload.len()]).to_le_bytes();
    out[5 + payload.len()..7 + payload.len()].copy_from_slice(&crc);
    Ok(out)
}

#[derive(Debug)]
pub struct Reply {
    pub command: u8,
    pub sequence: u8,
    pub payload: Vec<u8>,
}

pub fn decode(report: &[u8]) -> Result<Reply, String> {
    let data = if report.len() == 65 && report[0] == 0 {
        &report[1..]
    } else {
        report
    };
    if data.len() != 64 || data[0] != 0xFA {
        return Err("invalid bootloader reply".into());
    }
    let n = data[3] as usize;
    if n > CHUNK_SIZE {
        return Err("bootloader reply exceeds 58 bytes".into());
    }
    let received = u16::from_le_bytes(data[4 + n..6 + n].try_into().unwrap());
    if received != crc16_modbus(&data[..4 + n]) {
        return Err("bootloader CRC mismatch".into());
    }
    Ok(Reply {
        command: data[1],
        sequence: data[2],
        payload: data[4..4 + n].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_request_match_capture() {
        let report = encode(0x10, 0, &[]).unwrap();
        assert_eq!(&report[..7], &[0, 0xFB, 0x10, 0, 0, 0x30, 0xC5]);
    }

    #[test]
    fn invalid_reply_crc_rejected() {
        let mut report = [0u8; 64];
        report[..4].copy_from_slice(&[0xFA, 0x12, 0, 1]);
        report[4] = 1;
        assert!(decode(&report).is_err());
        let crc = crc16_modbus(&report[..5]).to_le_bytes();
        report[5..7].copy_from_slice(&crc);
        assert_eq!(decode(&report).unwrap().payload, vec![1]);
    }
}
