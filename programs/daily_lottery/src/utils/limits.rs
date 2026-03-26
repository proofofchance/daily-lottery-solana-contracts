//! Size limits for lucky words and reveals

/// Max bytes for the UTF-8 lucky words string
pub const MAX_LUCKY_WORD_BYTES: usize = 32;
/// Max bytes for salt (raw bytes represented as hex off-chain)
pub const MAX_SALT_BYTES: usize = 16;
/// Max bytes for reveal plaintext: lucky_words || 0x1f || salt
pub const MAX_REVEAL_PLAINTEXT_BYTES: usize = MAX_LUCKY_WORD_BYTES + 1 + MAX_SALT_BYTES;
