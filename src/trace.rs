use std::{
    fmt,
    fs::File,
    io::{self, BufRead, BufReader},
    path::Path,
};

use anyhow::{Context, Result, bail};

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
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)
            .with_context(|| format!("Unable to open trace file {}", path.display()))?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();
        for (idx, line) in reader.lines().enumerate() {
            let line = line.context("Failed to read line from trace")?;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let mut parts = trimmed.split_whitespace();
            let op = parts
                .next()
                .with_context(|| format!("Trace line {} missing op", idx + 1))?;
            let addr = parts
                .next()
                .with_context(|| format!("Trace line {} missing address", idx + 1))?;
            if parts.next().is_some() {
                bail!("Trace line {} has extra tokens", idx + 1);
            }
            let kind = match op.to_ascii_lowercase().chars().next() {
                Some('r') => AccessKind::Read,
                Some('w') => AccessKind::Write,
                _ => bail!("Trace line {} has invalid op '{}'", idx + 1, op),
            };
            let address = parse_address(addr).with_context(|| {
                format!("Trace line {}: invalid address literal '{}'", idx + 1, addr)
            })?;
            entries.push(TraceAccess { kind, address });
        }
        Ok(Self {
            name: path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
            entries,
        })
    }
}

fn parse_address(token: &str) -> io::Result<u64> {
    let token = token.trim();
    if let Some(hex) = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
    {
        return u64::from_str_radix(hex, 16)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e));
    }
    if let Some(bin) = token
        .strip_prefix("0b")
        .or_else(|| token.strip_prefix("0B"))
    {
        return u64::from_str_radix(bin, 2)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e));
    }
    if let Some(oct) = token
        .strip_prefix("0o")
        .or_else(|| token.strip_prefix("0O"))
    {
        return u64::from_str_radix(oct, 8)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e));
    }
    u64::from_str_radix(token, 16).or_else(|_| {
        u64::from_str_radix(token, 10).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    })
}
