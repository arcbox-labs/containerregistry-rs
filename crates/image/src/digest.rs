//! Content-addressable digest type.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest as Sha2Digest, Sha256, Sha384, Sha512};

use crate::Error;

/// A content-addressable digest in the format `algorithm:hex`.
///
/// Currently only SHA256 is supported as per OCI specification.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Digest {
    algorithm: Algorithm,
    hex: String,
}

/// Supported digest algorithms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Algorithm {
    Sha256,
    Sha384,
    Sha512,
}

impl Digest {
    /// Creates a new digest from algorithm and hex string.
    ///
    /// The hex string is normalized to lowercase per OCI specification.
    pub fn new(algorithm: Algorithm, hex: impl Into<String>) -> Result<Self, Error> {
        let hex = hex.into().to_ascii_lowercase();
        Self::validate_hex(&algorithm, &hex)?;
        Ok(Self { algorithm, hex })
    }

    /// Computes the SHA256 digest of the given data.
    pub fn sha256(data: &[u8]) -> Self {
        Self::compute(Algorithm::Sha256, data)
    }

    /// Computes the SHA384 digest of the given data.
    pub fn sha384(data: &[u8]) -> Self {
        Self::compute(Algorithm::Sha384, data)
    }

    /// Computes the SHA512 digest of the given data.
    pub fn sha512(data: &[u8]) -> Self {
        Self::compute(Algorithm::Sha512, data)
    }

    /// Computes a digest using the requested algorithm.
    pub fn compute(algorithm: Algorithm, data: &[u8]) -> Self {
        let hex = match algorithm {
            Algorithm::Sha256 => {
                let mut hasher = Sha256::new();
                hasher.update(data);
                hex::encode(hasher.finalize())
            }
            Algorithm::Sha384 => {
                let mut hasher = Sha384::new();
                hasher.update(data);
                hex::encode(hasher.finalize())
            }
            Algorithm::Sha512 => {
                let mut hasher = Sha512::new();
                hasher.update(data);
                hex::encode(hasher.finalize())
            }
        };
        Self { algorithm, hex }
    }

    /// Returns the algorithm component.
    pub fn algorithm(&self) -> Algorithm {
        self.algorithm
    }

    /// Returns the hex component.
    pub fn hex(&self) -> &str {
        &self.hex
    }

    fn validate_hex(algorithm: &Algorithm, hex: &str) -> Result<(), Error> {
        let expected_len = match algorithm {
            Algorithm::Sha256 => 64,
            Algorithm::Sha384 => 96,
            Algorithm::Sha512 => 128,
        };

        if hex.len() != expected_len {
            return Err(Error::InvalidDigest(format!(
                "expected {} hex characters, got {}",
                expected_len,
                hex.len()
            )));
        }

        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::InvalidDigest(
                "digest contains non-hexadecimal characters".to_string(),
            ));
        }

        Ok(())
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let alg = match self.algorithm {
            Algorithm::Sha256 => "sha256",
            Algorithm::Sha384 => "sha384",
            Algorithm::Sha512 => "sha512",
        };
        write!(f, "{}:{}", alg, self.hex)
    }
}

impl FromStr for Digest {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (alg, hex) = s
            .split_once(':')
            .ok_or_else(|| Error::InvalidDigest("missing ':' separator".to_string()))?;

        let algorithm = match alg.to_ascii_lowercase().as_str() {
            "sha256" => Algorithm::Sha256,
            "sha384" => Algorithm::Sha384,
            "sha512" => Algorithm::Sha512,
            _ => return Err(Error::UnsupportedAlgorithm(alg.to_string())),
        };

        Self::new(algorithm, hex)
    }
}

impl Serialize for Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for Algorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Algorithm::Sha256 => write!(f, "sha256"),
            Algorithm::Sha384 => write!(f, "sha384"),
            Algorithm::Sha512 => write!(f, "sha512"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_computation() {
        let digest = Digest::sha256(b"hello world");
        assert_eq!(digest.algorithm(), Algorithm::Sha256);
        assert_eq!(
            digest.hex(),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_digest_parse() {
        let s = "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        let digest: Digest = s.parse().unwrap();
        assert_eq!(digest.to_string(), s);
    }

    #[test]
    fn test_digest_parse_is_case_insensitive() {
        let s = "SHA256:B94D27B9934D3E08A52E52D7DA7DABFAC484EFE37A5380EE9088F7ACE2EFCDE9";
        let digest: Digest = s.parse().unwrap();
        assert_eq!(
            digest.to_string(),
            "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_digest_invalid_algorithm() {
        let s = "md5:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        let result: Result<Digest, _> = s.parse();
        assert!(result.is_err());
    }

    #[test]
    fn test_digest_sha384_sha512_lengths() {
        let data = b"hello";
        let sha384 = Digest::sha384(data);
        let sha512 = Digest::sha512(data);

        assert_eq!(sha384.hex().len(), 96);
        assert_eq!(sha512.hex().len(), 128);
        assert_eq!(sha384.algorithm(), Algorithm::Sha384);
        assert_eq!(sha512.algorithm(), Algorithm::Sha512);
    }

    #[test]
    fn test_digest_invalid_hex_length() {
        let s = "sha256:abcd";
        let result: Result<Digest, _> = s.parse();
        assert!(result.is_err());
    }

    #[test]
    fn test_digest_serde_roundtrip() {
        let digest = Digest::sha256(b"test");
        let json = serde_json::to_string(&digest).unwrap();
        let parsed: Digest = serde_json::from_str(&json).unwrap();
        assert_eq!(digest, parsed);
    }

    #[test]
    fn test_digest_hex_normalized_to_lowercase() {
        // Uppercase hex should be normalized to lowercase
        let upper = "sha256:B94D27B9934D3E08A52E52D7DA7DABFAC484EFE37A5380EE9088F7ACE2EFCDE9";
        let lower = "sha256:b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

        let digest_upper: Digest = upper.parse().unwrap();
        let digest_lower: Digest = lower.parse().unwrap();

        assert_eq!(digest_upper, digest_lower);
        assert_eq!(digest_upper.hex(), digest_lower.hex());
        assert_eq!(digest_upper.to_string(), lower);
    }

    #[test]
    fn test_digest_mixed_case_normalized() {
        let mixed = "sha256:B94d27B9934D3e08a52E52d7DA7dabFAC484efe37a5380EE9088f7ACE2efcDE9";
        let digest: Digest = mixed.parse().unwrap();

        // Should be all lowercase
        assert!(
            digest
                .hex()
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        );
    }
}
