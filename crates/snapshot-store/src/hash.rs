use blake3::Hasher;

pub fn hash_bytes(prefix: &str, bytes: &[u8]) -> String {
    let mut hasher = Hasher::new();
    hasher.update(bytes);
    format!("{}_{}", prefix, hasher.finalize().to_hex())
}
