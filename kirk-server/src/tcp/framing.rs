//! 16-byte fixed-header parser / serializer for the custom TCP protocol.

use crate::error::ServerError;

pub const MAGIC: u32 = 0x4B49_524B; // "KIRK" in big-endian text, stored LE bytes [0x4B,0x49,0x52,0x4B]
pub const VERSION: u8 = 1;
pub const HEADER_LEN: usize = 16;
pub const MAX_PAYLOAD: usize = 64 * 1024 * 1024;

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Forward = 0x01,
    InferenceEntropy = 0x02,
    InferenceFeatures = 0x03,
    ActiveInference = 0x04,
    ActiveInferenceEntropy = 0x05,
    ActiveInferenceFeatures = 0x06,
    ForwardSample = 0x07,
    Ping = 0xFE,
    Error = 0xFF,
}

impl Opcode {
    pub fn from_u8(b: u8) -> Result<Self, ServerError> {
        Ok(match b {
            0x01 => Opcode::Forward,
            0x02 => Opcode::InferenceEntropy,
            0x03 => Opcode::InferenceFeatures,
            0x04 => Opcode::ActiveInference,
            0x05 => Opcode::ActiveInferenceEntropy,
            0x06 => Opcode::ActiveInferenceFeatures,
            0x07 => Opcode::ForwardSample,
            0xFE => Opcode::Ping,
            0xFF => Opcode::Error,
            _ => return Err(ServerError::BadRequest(format!("unknown opcode 0x{b:02X}"))),
        })
    }
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpErrorCode {
    BadMagic = 0x01,
    PayloadTooLarge = 0x02,
    UnsupportedVersion = 0x03,
    UnknownOpcode = 0x04,
    BadPayload = 0x05,
    MatrixDimExceeded = 0x06,
    ComputeError = 0x07,
    ShutdownInProgress = 0x08,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // magic/version/flags retained for diagnostics + future use.
pub struct Header {
    pub magic: u32,
    pub version: u8,
    pub opcode: u8,
    pub flags: u16,
    pub req_id: u32,
    pub payload_len: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum FramingError {
    #[error("truncated header (need {HEADER_LEN}, got {0})")]
    TruncatedHeader(usize),
    #[error("bad magic 0x{0:08X}")]
    BadMagic(u32),
    #[error("unsupported version {0}")]
    UnsupportedVersion(u8),
    #[error("payload too large: {0} (limit {MAX_PAYLOAD})")]
    PayloadTooLarge(u32),
}

/// Parse the 16-byte header without copying. Returns the parsed header if all
/// invariants hold; otherwise a specific framing error.
pub fn parse_header(buf: &[u8]) -> Result<Header, FramingError> {
    if buf.len() < HEADER_LEN {
        return Err(FramingError::TruncatedHeader(buf.len()));
    }
    let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if magic != MAGIC {
        return Err(FramingError::BadMagic(magic));
    }
    let version = buf[4];
    if version != VERSION {
        return Err(FramingError::UnsupportedVersion(version));
    }
    let opcode = buf[5];
    let flags = u16::from_le_bytes([buf[6], buf[7]]);
    let req_id = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    let payload_len = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
    if payload_len as usize > MAX_PAYLOAD {
        return Err(FramingError::PayloadTooLarge(payload_len));
    }
    Ok(Header {
        magic,
        version,
        opcode,
        flags,
        req_id,
        payload_len,
    })
}

/// Write a 16-byte header into `out`.
pub fn write_header(out: &mut Vec<u8>, opcode: u8, req_id: u32, payload_len: u32) {
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.push(VERSION);
    out.push(opcode);
    out.extend_from_slice(&0u16.to_le_bytes()); // flags
    out.extend_from_slice(&req_id.to_le_bytes());
    out.extend_from_slice(&payload_len.to_le_bytes());
}

pub fn write_error_frame(out: &mut Vec<u8>, req_id: u32, code: TcpErrorCode, msg: &str) {
    let msg_bytes = msg.as_bytes();
    let payload_len = 8 + msg_bytes.len();
    write_header(out, Opcode::Error as u8, req_id, payload_len as u32);
    out.extend_from_slice(&(code as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&(msg_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(msg_bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_header() {
        let mut buf = Vec::new();
        write_header(&mut buf, Opcode::Forward as u8, 42, 1024);
        let h = parse_header(&buf).unwrap();
        assert_eq!(h.magic, MAGIC);
        assert_eq!(h.version, 1);
        assert_eq!(h.opcode, Opcode::Forward as u8);
        assert_eq!(h.req_id, 42);
        assert_eq!(h.payload_len, 1024);
    }

    #[test]
    fn truncated_rejected() {
        let buf = [0u8; 8];
        assert!(matches!(
            parse_header(&buf),
            Err(FramingError::TruncatedHeader(_))
        ));
    }

    #[test]
    fn bad_magic_rejected() {
        let mut buf = vec![0u8; HEADER_LEN];
        buf[0] = 0xFF;
        assert!(matches!(parse_header(&buf), Err(FramingError::BadMagic(_))));
    }

    #[test]
    fn version_mismatch_rejected() {
        let mut buf = Vec::new();
        write_header(&mut buf, Opcode::Forward as u8, 1, 0);
        buf[4] = 2;
        assert!(matches!(
            parse_header(&buf),
            Err(FramingError::UnsupportedVersion(2))
        ));
    }

    #[test]
    fn payload_cap_enforced() {
        let mut buf = Vec::new();
        write_header(&mut buf, Opcode::Forward as u8, 1, (MAX_PAYLOAD as u32) + 1);
        assert!(matches!(
            parse_header(&buf),
            Err(FramingError::PayloadTooLarge(_))
        ));
    }
}
