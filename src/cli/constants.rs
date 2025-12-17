// shared CLI constants
pub const DEFAULT_LARGE_THRESHOLD: u64 = 5 * 1024 * 1024;

#[macro_export]
macro_rules! cli_constant {
    ($name:expr) => {
        $name
    };
}
