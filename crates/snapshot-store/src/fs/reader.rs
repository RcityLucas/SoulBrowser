use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

use crate::model::{PixClip, SnapRef, StructSnap};

pub fn read_struct(path: &Path) -> io::Result<StructSnap> {
    read_and_decode(path)
}

pub fn read_clip(path: &Path) -> io::Result<PixClip> {
    read_and_decode(path)
}

pub fn read_action(path: &Path) -> io::Result<SnapRef> {
    read_and_decode(path)
}

fn read_and_decode<T>(path: &Path) -> io::Result<T>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    serde_json::from_slice(&buf)
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))
}
