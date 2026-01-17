use crate::errors::TlError;
use crate::model::JsonlLine;
use serde_json::to_string;
use std::fs::{create_dir_all, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub fn serialize_lines(
    lines: &[JsonlLine],
    max_payload_bytes: usize,
) -> Result<Vec<String>, TlError> {
    let mut serialized = Vec::with_capacity(lines.len());
    for line in lines {
        let json = to_string(line).map_err(|err| TlError::Internal(err.to_string()))?;
        if json.as_bytes().len() > max_payload_bytes {
            return Err(TlError::Oversize);
        }
        serialized.push(json);
    }
    Ok(serialized)
}

pub fn write_lines(base_path: &str, lines: &[String]) -> Result<String, TlError> {
    let path = Path::new(base_path);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            create_dir_all(parent).map_err(|err| TlError::Internal(err.to_string()))?;
        }
    }

    let mut writer = BufWriter::new(File::create(path).map_err(TlError::Io)?);
    for line in lines {
        writer
            .write_all(line.as_bytes())
            .and_then(|_| writer.write_all(b"\n"))
            .map_err(TlError::Io)?;
    }
    writer.flush().map_err(TlError::Io)?;

    Ok(path_to_string(path))
}

fn path_to_string(path: &Path) -> String {
    path.to_path_buf()
        .components()
        .collect::<PathBuf>()
        .to_string_lossy()
        .to_string()
}
