//! 豆包 IME 手写 protobuf 编解码（从 Swift Demo 移植）。
//!
//! AsrRequest 字段：2=token, 3=serviceName, 5=methodName, 6=payload(JSON),
//! 7=audioData(Opus 字节), 8=requestId, 9=frameState(varint)。
//! AsrResponse 字段：1=requestId, 2=taskId, 3=serviceName, 4=messageType,
//! 5=statusCode(varint), 6=statusMessage, 7=resultJson。
//!
//! 注意：varint 低字节写入时必须显式 `& 0x7F`——C++ 版 `static_cast<BYTE>` 是静默
//! 截断，Rust `as u8` 同样截断，但必须在截断前取低 7 位再置 continuation 位。

/// 编码一条 AsrRequest。空字段自动跳过。
pub fn encode_request(
    token: &str,
    method: &str,
    payload: &str,
    audio: &[u8],
    request_id: &str,
    frame_state: u64,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(128 + audio.len());
    encode_string(&mut out, 2, token);
    encode_string(&mut out, 3, "ASR");
    encode_string(&mut out, 5, method);
    encode_string(&mut out, 6, payload);
    encode_bytes(&mut out, 7, audio);
    encode_string(&mut out, 8, request_id);
    if frame_state != 0 {
        field_tag(&mut out, 9, 0);
        encode_varint(&mut out, frame_state);
    }
    out
}

/// 解码一条 AsrResponse；不是合法的 AsrResponse 时返回 None。
pub fn decode_response(data: &[u8]) -> Option<AsrResponse> {
    let mut response = AsrResponse::default();
    let mut cursor = 0usize;
    while cursor < data.len() {
        let tag = decode_varint(data, &mut cursor)?;
        let field = tag >> 3;
        let wire_type = tag & 0x7;
        match field {
            1 => response.request_id = decode_string(data, &mut cursor)?,
            2 => response.task_id = decode_string(data, &mut cursor)?,
            3 => response.service_name = decode_string(data, &mut cursor)?,
            4 => response.message_type = decode_string(data, &mut cursor)?,
            5 => {
                if wire_type != 0 {
                    return None;
                }
                response.status_code = decode_varint(data, &mut cursor)? as i32;
            }
            6 => response.status_message = decode_string(data, &mut cursor)?,
            7 => response.result_json = decode_string(data, &mut cursor)?,
            _ => skip_field(data, &mut cursor, wire_type)?,
        }
    }
    Some(response)
}

#[derive(Debug, Default, Clone)]
pub struct AsrResponse {
    pub request_id: String,
    pub task_id: String,
    pub service_name: String,
    pub message_type: String,
    pub status_code: i32,
    pub status_message: String,
    pub result_json: String,
}

// MARK: - varint 基础

/// 写入 varint。低字节取 `v & 0x7F | 0x80`（模拟 C++ 截断语义，见模块注释）。
fn encode_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v & 0x7F) as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn field_tag(out: &mut Vec<u8>, field: u32, wire_type: u32) {
    encode_varint(out, ((field as u64) << 3) | wire_type as u64);
}

fn encode_string(out: &mut Vec<u8>, field: u32, value: &str) {
    if value.is_empty() {
        return;
    }
    let bytes = value.as_bytes();
    field_tag(out, field, 2);
    encode_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn encode_bytes(out: &mut Vec<u8>, field: u32, value: &[u8]) {
    if value.is_empty() {
        return;
    }
    field_tag(out, field, 2);
    encode_varint(out, value.len() as u64);
    out.extend_from_slice(value);
}

fn decode_varint(data: &[u8], cursor: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    let mut shift = 0u32;
    while *cursor < data.len() {
        let b = data[*cursor];
        *cursor += 1;
        value |= u64::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

fn decode_string(data: &[u8], cursor: &mut usize) -> Option<String> {
    let len = decode_varint(data, cursor)? as usize;
    // checked_add 防畸形帧的 len 溢出（len 来自 varint，可为 2^64-1）。
    let end = cursor.checked_add(len)?;
    if end > data.len() {
        return None;
    }
    let s = std::str::from_utf8(&data[*cursor..end]).ok()?.to_string();
    *cursor = end;
    Some(s)
}

fn skip_field(data: &[u8], cursor: &mut usize, wire_type: u64) -> Option<()> {
    match wire_type {
        0 => {
            decode_varint(data, cursor)?;
        }
        2 => {
            let len = decode_varint(data, cursor)? as usize;
            let end = cursor.checked_add(len)?;
            if end > data.len() {
                return None;
            }
            *cursor = end;
        }
        5 => {
            if *cursor + 4 > data.len() {
                return None;
            }
            *cursor += 4;
        }
        1 => {
            if *cursor + 8 > data.len() {
                return None;
            }
            *cursor += 8;
        }
        _ => return None,
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_roundtrip_small() {
        let mut out = Vec::new();
        encode_varint(&mut out, 0);
        assert_eq!(out, vec![0]);
        let mut out = Vec::new();
        encode_varint(&mut out, 150);
        assert_eq!(out, vec![0x96, 0x01]);
        let mut cursor = 0;
        assert_eq!(decode_varint(&out, &mut cursor), Some(150));
    }

    #[test]
    fn varint_roundtrip_large() {
        // 128 字节长的字符串会走到多字节 varint 路径（Swift Demo 曾在此溢出崩溃）
        let long = "豆".repeat(43); // 43 * 3 = 129 字节
        let mut out = Vec::new();
        encode_string(&mut out, 2, &long);
        let cursor = 0;
        // field 2 tag + varint len + bytes
        let decoded = decode_response(&out).unwrap_or_default();
        assert!(decoded.request_id.is_empty());
        // 直接用 decode_string 验证
        let mut c = cursor;
        let _tag = decode_varint(&out, &mut c).unwrap();
        let s = decode_string(&out, &mut c).unwrap();
        assert_eq!(s, long);
    }

    #[test]
    fn encode_decode_roundtrip() {
        let req = encode_request(
            "token-token",
            "StartTask",
            "",
            &[0x9C, 0x80, 0x01], // 随便几字节 Opus
            "req-123",
            0,
        );
        // 手工验证结构：tag(2<<3|2)=0x12, len, token
        assert_eq!(req[0], 0x12);
        assert_eq!(&req[2..2 + 11], b"token-token");

        // StartTask 响应（手工构造最小消息）
        let mut resp = Vec::new();
        encode_string(&mut resp, 1, "req-123");
        encode_string(&mut resp, 4, "TaskStarted");
        encode_string(&mut resp, 6, "ok");
        let parsed = decode_response(&resp).unwrap();
        assert_eq!(parsed.request_id, "req-123");
        assert_eq!(parsed.message_type, "TaskStarted");
        assert_eq!(parsed.status_message, "ok");
    }
}
