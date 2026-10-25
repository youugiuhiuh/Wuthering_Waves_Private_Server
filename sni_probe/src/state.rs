//! 阶段间状态存储：一个 `state.bin` 文件替代三段 CSV 传参。
//!
//! 布局（proto3，与 `pb.rs` 同一套长度前缀语义）：
//!
//! ```proto
//! message State {
//!     repeated Entry phase1 = 1;  // 预解析结果 domain,ip
//!     repeated Entry phase2 = 2;  // 存活候选 domain,ip
//!     repeated Entry phase3 = 3;  // 深度质检通过 domain（ip 留空）
//!     uint32 completed = 4;       // 已完成阶段位掩码
//! }
//! message Entry { string domain = 1; string ip = 2; }
//! ```

use std::io::Write;
use std::net::IpAddr;
use std::path::Path;

/// 阶段位掩码。
pub const PHASE_RESOLVE: u32 = 1 << 0;
pub const PHASE_SURVIVAL: u32 = 1 << 1;
pub const PHASE_BUILD: u32 = 1 << 2;

const TAG_PHASE1: u8 = 0x0A;
const TAG_PHASE2: u8 = 0x12;
const TAG_PHASE3: u8 = 0x1A;
const TAG_COMPLETED: u8 = 0x20;

const TAG_DOMAIN: u8 = 0x0A;
const TAG_IP: u8 = 0x12;

/// 一条 `domain,ip` 记录（`ip` 可为空）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub domain: String,
    pub ip: Option<IpAddr>,
}

impl Entry {
    pub fn new(domain: impl Into<String>, ip: IpAddr) -> Self {
        Self {
            domain: domain.into(),
            ip: Some(ip),
        }
    }
}

/// 全流程状态。
#[derive(Debug, Default, Clone)]
pub struct State {
    pub phase1: Vec<Entry>,
    pub phase2: Vec<Entry>,
    pub phase3: Vec<Entry>,
    pub completed: u32,
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_bytes_field(out: &mut Vec<u8>, tag: u8, bytes: &[u8]) {
    out.push(tag);
    put_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn encode_entry(e: &Entry) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    put_bytes_field(&mut out, TAG_DOMAIN, e.domain.as_bytes());
    if let Some(ip) = e.ip {
        put_bytes_field(&mut out, TAG_IP, ip.to_string().as_bytes());
    }
    out
}

fn decode_entry(buf: &[u8]) -> Result<Entry, String> {
    let mut domain = String::new();
    let mut ip = None;
    let mut pos = 0usize;
    while pos < buf.len() {
        let tag = buf[pos];
        pos += 1;
        let len = read_varint(buf, &mut pos)? as usize;
        let end = pos.checked_add(len).ok_or("length overflow")?;
        let slice = buf.get(pos..end).ok_or("truncated field")?;
        match tag {
            TAG_DOMAIN => domain = String::from_utf8(slice.to_vec()).map_err(|e| e.to_string())?,
            TAG_IP => {
                let s = std::str::from_utf8(slice).map_err(|e| e.to_string())?;
                ip = s.parse().ok();
            }
            _ => {}
        }
        pos = end;
    }
    Ok(Entry { domain, ip })
}

fn read_varint(buf: &[u8], pos: &mut usize) -> Result<u64, String> {
    let mut result = 0u64;
    let mut shift = 0;
    loop {
        let b = *buf.get(*pos).ok_or("truncated varint")?;
        *pos += 1;
        result |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            return Ok(result);
        }
        shift += 7;
        if shift >= 64 {
            return Err("varint too long".into());
        }
    }
}

/// 序列化。
pub fn encode(state: &State) -> Vec<u8> {
    let mut out = Vec::with_capacity(256);
    for e in &state.phase1 {
        put_bytes_field(&mut out, TAG_PHASE1, &encode_entry(e));
    }
    for e in &state.phase2 {
        put_bytes_field(&mut out, TAG_PHASE2, &encode_entry(e));
    }
    for e in &state.phase3 {
        put_bytes_field(&mut out, TAG_PHASE3, &encode_entry(e));
    }
    if state.completed != 0 {
        out.push(TAG_COMPLETED);
        put_varint(&mut out, state.completed as u64);
    }
    out
}

/// 反序列化。
pub fn decode(buf: &[u8]) -> Result<State, String> {
    let mut state = State::default();
    let mut pos = 0usize;
    while pos < buf.len() {
        let tag = buf[pos];
        pos += 1;
        match tag {
            TAG_PHASE1 | TAG_PHASE2 | TAG_PHASE3 => {
                let len = read_varint(buf, &mut pos)? as usize;
                let end = pos.checked_add(len).ok_or("length overflow")?;
                let slice = buf.get(pos..end).ok_or("truncated entry")?;
                let entry = decode_entry(slice)?;
                match tag {
                    TAG_PHASE1 => state.phase1.push(entry),
                    TAG_PHASE2 => state.phase2.push(entry),
                    _ => state.phase3.push(entry),
                }
                pos = end;
            }
            TAG_COMPLETED => {
                state.completed = read_varint(buf, &mut pos)? as u32;
            }
            other => {
                let wire_type = other & 0x07;
                match wire_type {
                    0 => {
                        read_varint(buf, &mut pos)?;
                    }
                    2 => {
                        let len = read_varint(buf, &mut pos)? as usize;
                        pos = pos.checked_add(len).ok_or("length overflow")?;
                    }
                    5 => pos += 4,
                    1 => pos += 8,
                    _ => return Err(format!("unsupported wire type {wire_type}")),
                }
                if pos > buf.len() {
                    return Err("truncated unknown field".into());
                }
            }
        }
    }
    Ok(state)
}

impl State {
    pub fn is_done(&self, phase: u32) -> bool {
        self.completed & phase != 0
    }

    pub fn mark_done(&mut self, phase: u32) {
        self.completed |= phase;
    }

    /// 从磁盘读取；文件不存在返回空状态。
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(data) => decode(&data),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("read {}: {e}", path.display())),
        }
    }

    /// 原子写入（先写临时文件再 rename），避免中断留下半截状态。
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("bin.tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(&encode(self))?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn roundtrips_all_phases_and_bitmask() {
        let mut s = State::default();
        s.phase1.push(Entry::new("a.com", ip("1.2.3.4")));
        s.phase2
            .push(Entry::new("b.com", ip("2606:4700:4700::1111")));
        s.phase3.push(Entry {
            domain: "c.com".into(),
            ip: None,
        });
        s.mark_done(PHASE_RESOLVE);
        s.mark_done(PHASE_SURVIVAL);

        let decoded = decode(&encode(&s)).unwrap();
        assert_eq!(decoded.phase1, s.phase1);
        assert_eq!(decoded.phase2, s.phase2);
        assert_eq!(decoded.phase3, s.phase3);
        assert_eq!(decoded.completed, s.completed);
        assert!(decoded.is_done(PHASE_RESOLVE));
        assert!(decoded.is_done(PHASE_SURVIVAL));
        assert!(!decoded.is_done(PHASE_BUILD));
    }

    #[test]
    fn empty_state_is_empty_bytes() {
        assert!(encode(&State::default()).is_empty());
        let d = decode(&[]).unwrap();
        assert!(d.phase1.is_empty() && d.phase2.is_empty() && d.phase3.is_empty());
        assert_eq!(d.completed, 0);
    }

    #[test]
    fn entry_without_ip_roundtrips() {
        let e = Entry {
            domain: "x.com".into(),
            ip: None,
        };
        assert_eq!(decode_entry(&encode_entry(&e)).unwrap(), e);
    }

    #[test]
    fn rejects_malformed_input() {
        assert!(decode(&[TAG_PHASE1, 0x05, b'a']).is_err());
        assert!(decode(&[TAG_PHASE1, 0xFF]).is_err());
    }

    #[test]
    fn skips_unknown_fields_for_forward_compat() {
        let mut buf = encode(&State {
            phase1: vec![Entry::new("a.com", ip("1.1.1.1"))],
            ..Default::default()
        });
        buf.extend_from_slice(&[0x28, 0x2A]); // field 5, varint
        let d = decode(&buf).unwrap();
        assert_eq!(d.phase1.len(), 1);
    }

    #[test]
    fn save_load_roundtrip_creates_dir_and_is_atomic() {
        let dir = std::env::temp_dir().join(format!("state_test_{}", std::process::id()));
        let path = dir.join("nested").join("state.bin");

        assert_eq!(
            State::load(&path).unwrap().phase1.len(),
            0,
            "缺失文件应为空状态"
        );

        let mut s = State::default();
        s.phase1.push(Entry::new("a.com", ip("1.1.1.1")));
        s.mark_done(PHASE_RESOLVE);
        s.save(&path).unwrap();

        assert!(
            !path.with_extension("bin.tmp").exists(),
            "临时文件应已 rename"
        );

        let loaded = State::load(&path).unwrap();
        assert_eq!(loaded.phase1, s.phase1);
        assert!(loaded.is_done(PHASE_RESOLVE));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bitmask_combines_independently() {
        let mut s = State::default();
        assert!(!s.is_done(PHASE_RESOLVE));
        s.mark_done(PHASE_RESOLVE);
        s.mark_done(PHASE_BUILD);
        assert!(s.is_done(PHASE_RESOLVE));
        assert!(!s.is_done(PHASE_SURVIVAL));
        assert!(s.is_done(PHASE_BUILD));
        assert_eq!(s.completed, PHASE_RESOLVE | PHASE_BUILD);
    }
}
