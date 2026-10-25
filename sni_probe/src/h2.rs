//! 最小 HTTP/2 客户端：只为在**同一条已握手连接**上取回响应头。
//!
//! REALITY 目标审查只需要 `:status` 与少量头部（`server` / `cf-ray` / `cf-cache-status`），
//! 为此引入完整 HTTP 客户端库没有意义。
//!
//! 约束：
//! - 使用 HPACK 的 "literal header field without indexing, new name"（0x00 前缀），
//!   对端只需基础的 HPACK 解码器即可处理，不需要 Huffman 支持；
//! - 响应头块中 0x8X 为静态表索引（0x88 = `:status 200`），仅解析静态表覆盖的部分。

use std::collections::HashMap;
use std::io::{Read, Write};

/// HTTP/2 connection preface。
const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";

const FRAME_HEADERS: u8 = 0x1;
const FRAME_SETTINGS: u8 = 0x4;
const FLAG_END_STREAM: u8 = 0x1;
const FLAG_END_HEADERS: u8 = 0x4;
const FLAG_ACK: u8 = 0x1;

/// HPACK 静态表（索引 1..=61）中与探测相关的表项。
const STATIC_TABLE: &[(&str, &str)] = &[
    (":authority", ""),       // 1
    (":method", "GET"),       // 2
    (":method", "POST"),      // 3
    (":path", "/"),           // 4
    (":path", "/index.html"), // 5
    (":scheme", "http"),      // 6
    (":scheme", "https"),     // 7
    (":status", "200"),       // 8
    (":status", "204"),       // 9
    (":status", "206"),       // 10
    (":status", "304"),       // 11
    (":status", "400"),       // 12
    (":status", "404"),       // 13
    (":status", "500"),       // 14
    ("accept-charset", ""),   // 15
];

fn write_frame(out: &mut Vec<u8>, typ: u8, flags: u8, sid: u32, payload: &[u8]) {
    let len = payload.len() as u32;
    out.extend_from_slice(&len.to_be_bytes()[1..4]);
    out.push(typ);
    out.push(flags);
    out.extend_from_slice(&sid.to_be_bytes());
    out.extend_from_slice(payload);
}

fn literal_header(out: &mut Vec<u8>, name: &[u8], value: &[u8]) {
    out.push(0x00); // literal, without indexing, new name
    out.push(name.len() as u8);
    out.extend_from_slice(name);
    out.push(value.len() as u8);
    out.extend_from_slice(value);
}

/// 解析响应头块中的静态表索引项，返回 `(name, value)`。
fn decode_static(block: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < block.len() {
        let b = block[i];
        // 0b1xxxxxxx = indexed header field
        if b & 0x80 != 0 {
            let idx = (b & 0x7F) as usize;
            if idx > 0
                && let Some((name, value)) = STATIC_TABLE.get(idx - 1)
            {
                out.push(((*name).to_string(), (*value).to_string()));
            }
            i += 1;
        } else {
            // 其余形式（含 Huffman 字符串）本实现不解析，交给调用方按需扩展
            break;
        }
    }
    out
}

/// 在已握手的 h2 连接上请求 `HEAD /`，返回响应头（name 小写）。
pub fn head<S: Read + Write>(
    stream: &mut S,
    authority: &str,
    user_agent: &str,
) -> Result<HashMap<String, String>, Box<dyn std::error::Error + Send + Sync>> {
    let mut out = Vec::new();
    out.extend_from_slice(PREFACE);
    write_frame(&mut out, FRAME_SETTINGS, 0, 0, &[]);

    let mut hb = Vec::new();
    literal_header(&mut hb, b":method", b"HEAD");
    literal_header(&mut hb, b":scheme", b"https");
    literal_header(&mut hb, b":authority", authority.as_bytes());
    literal_header(&mut hb, b":path", b"/");
    literal_header(&mut hb, b"user-agent", user_agent.as_bytes());
    write_frame(
        &mut out,
        FRAME_HEADERS,
        FLAG_END_HEADERS | FLAG_END_STREAM,
        1,
        &hb,
    );
    stream.write_all(&out)?;

    let mut acc: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            return Err("h2: connection closed before headers".into());
        }
        acc.extend_from_slice(&buf[..n]);

        let mut i = 0;
        while i + 9 <= acc.len() {
            let len =
                ((acc[i] as usize) << 16) | ((acc[i + 1] as usize) << 8) | acc[i + 2] as usize;
            if i + 9 + len > acc.len() {
                break;
            }
            let typ = acc[i + 3];
            let flags = acc[i + 4];

            match typ {
                FRAME_HEADERS => {
                    let block = &acc[i + 9..i + 9 + len];
                    let headers: HashMap<String, String> =
                        decode_static(block).into_iter().collect();
                    if headers.contains_key(":status") {
                        return Ok(headers);
                    }
                }
                FRAME_SETTINGS if flags & FLAG_ACK == 0 => {
                    let mut ack = Vec::new();
                    write_frame(&mut ack, FRAME_SETTINGS, FLAG_ACK, 0, &[]);
                    stream.write_all(&ack)?;
                }
                _ => {}
            }
            i += 9 + len;
        }
        acc.drain(..i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_static_status_index() {
        // 0x88 = indexed header field, index 8 (:status 200)
        assert_eq!(
            decode_static(&[0x88]),
            vec![(":status".to_string(), "200".to_string())]
        );
        // 0x8d = index 13 (:status 404)
        assert_eq!(
            decode_static(&[0x8d]),
            vec![(":status".to_string(), "404".to_string())]
        );
    }

    #[test]
    fn stops_on_unsupported_literal_forms() {
        // 0x00 开头是 literal（本实现不解析），应安全返回空而不 panic
        assert!(decode_static(&[0x00, 0x03, b'f', b'o', b'o']).is_empty());
        assert!(decode_static(&[]).is_empty());
    }

    #[test]
    fn frame_header_layout_matches_rfc9113() {
        let mut out = Vec::new();
        write_frame(&mut out, FRAME_SETTINGS, FLAG_ACK, 0, &[]);
        // 9 字节帧头：3 长度 + 1 类型 + 1 标志 + 4 流 ID
        assert_eq!(out.len(), 9);
        assert_eq!(&out[0..3], &[0, 0, 0]);
        assert_eq!(out[3], FRAME_SETTINGS);
        assert_eq!(out[4], FLAG_ACK);
        assert_eq!(&out[5..9], &[0, 0, 0, 0]);
    }
}
