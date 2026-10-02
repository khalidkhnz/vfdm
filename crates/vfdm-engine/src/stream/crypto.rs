use super::fetch;
use super::Resource;
use crate::download::RunCtx;
use crate::error::{EngineError, Result};
use crate::types::DownloadRequest;
use aes::cipher::{Array, BlockCipherDecrypt, KeyInit};
use aes::Aes128;
use std::collections::HashMap;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use url::Url;

/// HLS default IV: the media sequence number as a 128-bit big-endian integer.
pub fn iv_from_sequence(seq: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[8..].copy_from_slice(&seq.to_be_bytes());
    iv
}

pub fn parse_iv(s: &str) -> Result<[u8; 16]> {
    let h = s.trim();
    let h = h
        .strip_prefix("0x")
        .or_else(|| h.strip_prefix("0X"))
        .unwrap_or(h);
    if h.len() != 32 {
        return Err(EngineError::Other(format!("bad IV length: {}", h.len())));
    }
    let mut iv = [0u8; 16];
    for (i, byte) in iv.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16)
            .map_err(|_| EngineError::Other("IV is not hex".into()))?;
    }
    Ok(iv)
}

/// AES-128-CBC with PKCS#7 padding, as used by HLS `METHOD=AES-128`.
pub fn decrypt_aes128_cbc(data: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Result<Vec<u8>> {
    if data.is_empty() || !data.len().is_multiple_of(16) {
        return Err(EngineError::Other(format!(
            "encrypted segment length {} is not a multiple of 16",
            data.len()
        )));
    }
    let cipher = Aes128::new(&Array::from(*key));
    let mut out = Vec::with_capacity(data.len());
    let mut prev = *iv;
    for chunk in data.chunks_exact(16) {
        let cur: [u8; 16] = chunk.try_into().expect("16-byte chunk");
        let mut block = Array::from(cur);
        cipher.decrypt_block(&mut block);
        let plain: [u8; 16] = block.0;
        for i in 0..16 {
            out.push(plain[i] ^ prev[i]);
        }
        prev = cur;
    }
    let pad = *out.last().unwrap() as usize;
    if pad == 0
        || pad > 16
        || out.len() < pad
        || !out[out.len() - pad..].iter().all(|&b| b as usize == pad)
    {
        return Err(EngineError::Other("bad PKCS#7 padding (wrong key?)".into()));
    }
    out.truncate(out.len() - pad);
    Ok(out)
}

/// Fetches each key URL once per download.
#[derive(Default)]
pub struct KeyCache {
    map: Mutex<HashMap<String, [u8; 16]>>,
}

impl KeyCache {
    pub async fn get(
        &self,
        ctx: &RunCtx,
        cancel: &CancellationToken,
        req: &DownloadRequest,
        url: &Url,
    ) -> Result<[u8; 16]> {
        let mut map = self.map.lock().await;
        if let Some(k) = map.get(url.as_str()) {
            return Ok(*k);
        }
        let bytes = fetch::fetch_bytes(
            ctx,
            cancel,
            req,
            &Resource {
                url: url.clone(),
                range: None,
            },
        )
        .await?;
        let key: [u8; 16] = bytes.as_ref().try_into().map_err(|_| {
            EngineError::Other(format!("key is {} bytes, expected 16", bytes.len()))
        })?;
        map.insert(url.to_string(), key);
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockCipherEncrypt;

    fn encrypt(plain: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Vec<u8> {
        let cipher = Aes128::new(&Array::from(*key));
        let pad = 16 - plain.len() % 16;
        let mut buf = plain.to_vec();
        buf.extend(std::iter::repeat_n(pad as u8, pad));
        let mut prev = *iv;
        for chunk in buf.chunks_exact_mut(16) {
            for i in 0..16 {
                chunk[i] ^= prev[i];
            }
            let mut block = Array::from(<[u8; 16]>::try_from(&*chunk).unwrap());
            cipher.encrypt_block(&mut block);
            chunk.copy_from_slice(&block.0);
            prev.copy_from_slice(chunk);
        }
        buf
    }

    #[test]
    fn roundtrip() {
        let key = [9u8; 16];
        let iv = iv_from_sequence(7);
        assert_eq!(iv[15], 7);
        for len in [0usize, 1, 15, 16, 17, 1000] {
            let plain: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let enc = encrypt(&plain, &key, &iv);
            assert_eq!(decrypt_aes128_cbc(&enc, &key, &iv).unwrap(), plain);
        }
        assert!(
            decrypt_aes128_cbc(&[1u8; 15], &key, &iv).is_err(),
            "length check"
        );
    }

    #[test]
    fn iv_parsing() {
        assert_eq!(
            parse_iv("0x000102030405060708090a0b0c0d0e0f").unwrap()[15],
            0x0f
        );
        assert!(parse_iv("0x00").is_err());
    }
}
