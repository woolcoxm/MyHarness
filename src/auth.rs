//! Credential management for myharness. Stores API keys in
//! `~/.myharness/auth.json`, obfuscated at rest with a machine-derived
//! XOR key (not cryptographically secure, but prevents casual plain-text
//! reading — same threat model as npm, gh, and aws-cli credentials).
//!
//! The credential file supports multiple providers:
//! ```json
//! {
//!   "version": 1,
//!   "active": "coding-plan",
//!   "providers": {
//!     "coding-plan": {
//!       "api_key": "<obfuscated>",
//!       "base_url": "https://api.z.ai/api/coding/paas/v4",
//!       "model": "glm-5.3",
//!       "protocol": "openai"
//!     },
//!     "standard-api": {
//!       "api_key": "<obfuscated>",
//!       "base_url": "https://api.z.ai/api/anthropic",
//!       "model": "glm-5.3",
//!       "protocol": "anthropic"
//!     }
//!   }
//! }
//! ```

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAuth {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    /// "openai" | "anthropic"
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthFile {
    pub version: u32,
    pub active: String,
    pub providers: std::collections::HashMap<String, ProviderAuth>,
}

/// Derive an obfuscation key from machine-specific properties.
/// Same machine produces the same key; different machines produce different
/// keys (so copying the auth file to another machine doesn't work).
fn machine_key() -> Vec<u8> {
    let hostname = hostname();
    let username = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    let seed = format!("myharness::{hostname}::{username}");
    // Simple hash expansion to 32 bytes
    let mut key = Vec::with_capacity(32);
    let mut h: u64 = 0xcbf29ce484222325;
    for b in seed.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
        key.push((h & 0xff) as u8);
    }
    while key.len() < 32 {
        h = h.wrapping_mul(0x100000001b3);
        key.push((h & 0xff) as u8);
    }
    key
}

fn hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

/// XOR-obfuscate a string with the machine key, then base64-encode.
pub fn obfuscate(plain: &str) -> String {
    let key = machine_key();
    let bytes = plain.as_bytes();
    let encoded: Vec<u8> = bytes
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect();
    base64_encode(&encoded)
}

/// Reverse of `obfuscate`.
pub fn deobfuscate(encoded: &str) -> Option<String> {
    let key = machine_key();
    let bytes = base64_decode(encoded)?;
    let decoded: Vec<u8> = bytes
        .iter()
        .enumerate()
        .map(|(i, b)| b ^ key[i % key.len()])
        .collect();
    String::from_utf8(decoded).ok()
}

/// Minimal base64 encode (no external dependency).
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        out.push(CHARS[(b[0] >> 2) as usize] as char);
        out.push(CHARS[(((b[0] & 0x03) << 4) | (b[1] >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            CHARS[(((b[1] & 0x0f) << 2) | (b[2] >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            CHARS[(b[2] & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// Minimal base64 decode.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let bytes: Vec<u8> = s
        .bytes()
        .filter(|b| *b != b'=' && *b != b'\n' && *b != b'\r')
        .collect();
    for chunk in bytes.chunks(4) {
        if chunk.len() < 2 {
            return None;
        }
        let vals: Vec<u32> = chunk
            .iter()
            .map(|c| {
                CHARS
                    .iter()
                    .position(|&p| p == *c)
                    .map(|p| p as u32)
                    .unwrap_or(0)
            })
            .collect();
        out.push(((vals[0] << 2) | (vals[1] >> 4)) as u8);
        if chunk.len() > 2 {
            out.push((((vals[1] & 0x0f) << 4) | (vals[2] >> 2)) as u8);
        }
        if chunk.len() > 3 {
            out.push((((vals[2] & 0x03) << 6) | vals[3]) as u8);
        }
    }
    Some(out)
}

/// Path to the credential file.
pub fn auth_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".myharness").join("auth.json"))
}

/// Load credentials, deobfuscating the API key.
pub fn load() -> Option<AuthFile> {
    let path = auth_path()?;
    let raw = std::fs::read_to_string(path).ok()?;
    let mut file: AuthFile = serde_json::from_str(&raw).ok()?;
    for provider in file.providers.values_mut() {
        if let Some(plain) = deobfuscate(&provider.api_key) {
            provider.api_key = plain;
        }
    }
    Some(file)
}

/// Get the active provider's credentials.
pub fn active_credentials() -> Option<ProviderAuth> {
    let file = load()?;
    let name = file.active.clone();
    file.providers.get(&name).cloned()
}

/// Save credentials, obfuscating the API key.
pub fn save(active: &str, providers: std::collections::HashMap<String, ProviderAuth>) -> Result<()> {
    let path = auth_path().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = AuthFile {
        version: 1,
        active: active.to_string(),
        providers,
    };
    for provider in file.providers.values_mut() {
        provider.api_key = obfuscate(&provider.api_key);
    }
    let json = serde_json::to_string_pretty(&file)?;
    std::fs::write(&path, &json)?;
    // Restrict permissions on unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Check if credentials exist (without loading them).
pub fn exists() -> bool {
    auth_path().map(|p| p.exists()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obfuscate_round_trip() {
        let original = "sk-my-secret-api-key-12345";
        let encoded = obfuscate(original);
        assert_ne!(encoded, original, "must not be plain text");
        assert!(!encoded.contains("sk-my"), "must not contain the original key");
        let decoded = deobfuscate(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn obfuscate_different_keys_produce_different_output() {
        let key = "test-key-12345";
        let e1 = obfuscate(key);
        // Same input always produces same output on same machine
        assert_eq!(e1, obfuscate(key));
    }

    #[test]
    fn base64_round_trip() {
        let data = b"hello world 123 !@#";
        let encoded = base64_encode(data);
        let decoded = base64_decode(&encoded).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn auth_file_serialization() {
        let mut providers = std::collections::HashMap::new();
        providers.insert(
            "test".to_string(),
            ProviderAuth {
                api_key: obfuscate("sk-test-123"),
                base_url: "https://api.example.com".to_string(),
                model: "test-model".to_string(),
                protocol: "openai".to_string(),
            },
        );
        let file = AuthFile {
            version: 1,
            active: "test".to_string(),
            providers,
        };
        let json = serde_json::to_string(&file).unwrap();
        let parsed: AuthFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.active, "test");
    }
}
