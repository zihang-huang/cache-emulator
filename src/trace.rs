use std::{
    fmt,
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    Read,
    Write,
}

impl fmt::Display for AccessKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AccessKind::Read => write!(f, "R"),
            AccessKind::Write => write!(f, "W"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TraceAccess {
    pub kind: AccessKind,
    pub address: u64,
}

#[derive(Debug, Clone)]
pub struct TraceFile {
    pub name: String,
    pub entries: Vec<TraceAccess>,
}

impl TraceFile {
    pub fn load(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let file = File::open(path).expect("trace file missing");
        let reader = BufReader::new(file);
        let mut entries = Vec::new();
        for line in reader.lines() {
            let line = line.expect("failed to read trace line");
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let op = parts.next().unwrap();
            let addr = parts.next().unwrap();
            let kind = match op.to_ascii_lowercase().chars().next().unwrap_or('r') {
                'r' => AccessKind::Read,
                'w' => AccessKind::Write,
                _ => AccessKind::Read,
            };
            let address = parse_address(addr);
            entries.push(TraceAccess { kind, address });
        }
        Self {
            name: path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            entries,
        }
    }
}

fn parse_address(token: &str) -> u64 {
    let token = token.trim();
    if let Some(hex) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        return u64::from_str_radix(hex, 16).unwrap();
    }
    if let Some(bin) = token
        .strip_prefix("0b")
        .or_else(|| token.strip_prefix("0B"))
    {
        return u64::from_str_radix(bin, 2).unwrap();
    }
    if let Some(oct) = token
        .strip_prefix("0o")
        .or_else(|| token.strip_prefix("0O"))
    {
        return u64::from_str_radix(oct, 8).unwrap();
    }
    u64::from_str_radix(token, 16).unwrap_or_else(|_| u64::from_str_radix(token, 10).unwrap())
}
