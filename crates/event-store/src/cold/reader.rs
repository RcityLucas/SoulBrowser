use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use crate::model::EventEnvelope;

/// Sequential reader for exported cold logs.
pub fn read_range(path: &Path) -> io::Result<Vec<EventEnvelope>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    reader
        .lines()
        .filter_map(|line| {
            line.ok()
                .and_then(|raw| serde_json::from_str::<EventEnvelope>(&raw).ok())
        })
        .collect::<Vec<_>>()
        .pipe(Ok)
}

trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}

impl<T> Pipe for T {}
