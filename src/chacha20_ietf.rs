use crate::ChachaPolyError;
use crypto_api::{
	cipher::{ CipherInfo, Cipher },
	rng::{ SecureRng, SecKeyGen }
};
use std::{ cmp::min, error::Error };


/// The maximum amount of bytes that can be processed with one key/nonce combination
#[cfg(target_pointer_width = "64")]
const CHACHA20_MAX: usize = 4_294_967_296 * 64; // 2^32 * BLOCK_SIZE
/// The maximum amount of bytes that can be processed with one key/nonce combination
#[cfg(target_pointer_width = "32")]
const CHACHA20_MAX: usize = usize::max_value(); // 2^32 - 1


/// Computes the `n`th ChaCha20 block with `key` and `nonce` into `buf`
fn chacha20(key: &[u8], nonce: &[u8], n: u32, buf: &mut[u8]) {
	// ChaCha20 constants
	const CONSTANTS: [u32; 4] = [0x61707865, 0x3320646e, 0x79622d32, 0x6b206574];
	
	// Read key and nonce
	let mut key_words = vec![0; 8];
	for i in 0..8 { key_words[i] =   read32_le!(  &key[i * 4..]) }
	
	let mut nonce_words = vec![0; 3];
	for i in 0..3 { nonce_words[i] = read32_le!(&nonce[i * 4..]) }
	
	
	// Compute block
	let mut state = vec![0u32; 16];
	state[ 0.. 4].copy_from_slice(&CONSTANTS);
	state[ 4..12].copy_from_slice(&key_words);
	state[12] = n;
	state[13..16].copy_from_slice(&nonce_words);
	
	// Compute double-rounds
	for _ in 0..10 {
		/// A ChaCha20 quarterround
		macro_rules! quarterround {
			($a:expr, $b:expr, $c:expr, $d:expr) => ({
				state[$a] = add!(state[$a], state[$b]);
				state[$d] = xor!(state[$d], state[$a]);
				state[$d] = or!(shl!(state[$d], 16), shr!(state[$d], 16));
				state[$c] = add!(state[$c], state[$d]);
				state[$b] = xor!(state[$b], state[$c]);
				state[$b] = or!(shl!(state[$b], 12), shr!(state[$b], 20));
				state[$a] = add!(state[$a], state[$b]);
				state[$d] = xor!(state[$d], state[$a]);
				state[$d] = or!(shl!(state[$d],  8), shr!(state[$d], 24));
				state[$c] = add!(state[$c], state[$d]);
				state[$b] = xor!(state[$b], state[$c]);
				state[$b] = or!(shl!(state[$b],  7), shr!(state[$b], 25));
			});
		}
		
		// Perform 8 quarterrounds (2 rounds)
		quarterround!( 0,  4,  8, 12);
		quarterround!( 1,  5,  9, 13);
		quarterround!( 2,  6, 10, 14);
		quarterround!( 3,  7, 11, 15);
		quarterround!( 0,  5, 10, 15);
		quarterround!( 1,  6, 11, 12);
		quarterround!( 2,  7,  8, 13);
		quarterround!( 3,  4,  9, 14);
	}
	
	// Finalize block
	for i in  0.. 4 { write32_le!(add!(state[i],   CONSTANTS[i	 ]) => &mut buf[i * 4..]) }
	for i in  4..12 { write32_le!(add!(state[i],   key_words[i -  4]) => &mut buf[i * 4..]) }
	write32_le!(add!(state[12], n) => &mut buf[48..]);
	for i in 13..16 { write32_le!(add!(state[i], nonce_words[i - 13]) => &mut buf[i * 4..]) }
}


/// An implementation of [ChaCha20 (IETF-version)](https://tools.ietf.org/html/rfc8439)
pub struct ChaCha20Ietf;
impl ChaCha20Ietf {
	/// Creates a `Cipher` instance with `ChaCha20Ietf` as underlying cipher
	pub fn cipher() -> Box<dyn Cipher> {
		Box::new(Self)
	}
	
	/// XORs the bytes in `data` with the ChaCha20-keystream for `key` and `nonce` starting at the
	/// `n`th block
	pub(in crate) fn xor(key: &[u8], nonce: &[u8], mut n: u32, mut data: &mut[u8]) {
		let mut buf = vec![0; 64];
		while !data.is_empty() {
			// Compute next block
			chacha20(key, nonce, n, &mut buf);
			n += 1;
			
			// Xor block
			let to_xor = min(data.len(), buf.len());
			for i in 0..to_xor { data[i] = xor!(data[i], buf[i]) }
			data = &mut data[to_xor..];
		}
	}
}
impl SecKeyGen for ChaCha20Ietf {
	fn new_sec_key(&self, buf: &mut[u8], rng: &mut SecureRng)
		-> Result<usize, Box<dyn Error + 'static>>
	{
		// Validate buffer and generate key
		if buf.len() < 32 { Err(ChachaPolyError::ApiMisuse("Buffer is too small"))? }
		rng.random(&mut buf[..32])?;
		Ok(32)
	}
}
impl Cipher for ChaCha20Ietf {
	fn info(&self) -> CipherInfo {
		CipherInfo {
			name: "ChaCha20Ietf", is_otc: true,
			key_len_r: 32..32, nonce_len_r: 12..12, aead_tag_len_r: 0..0
		}
	}
	
	fn encrypted_len_max(&self, plaintext_len: usize) -> usize {
		plaintext_len
	}
	
	fn encrypt(&self, buf: &mut[u8], plaintext_len: usize, key: &[u8], nonce: &[u8])
		-> Result<usize, Box<dyn Error + 'static>>
	{
		// Check input
		if key.len() != 32 { Err(ChachaPolyError::ApiMisuse("Invalid key length"))? }
		if nonce.len() != 12 { Err(ChachaPolyError::ApiMisuse("Invalid nonce length"))? }
		if plaintext_len > CHACHA20_MAX { Err(ChachaPolyError::ApiMisuse("Too much data"))? }
		if plaintext_len > buf.len() { Err(ChachaPolyError::ApiMisuse("Buffer is too small"))? }
		
		// Encrypt the data
		Self::xor(key, nonce, 0, &mut buf[..plaintext_len]);
		Ok(plaintext_len)
	}
	fn encrypt_to(&self, buf: &mut[u8], plaintext: &[u8], key: &[u8], nonce: &[u8])
		-> Result<usize, Box<dyn Error + 'static>>
	{
		// Check input
		if key.len() != 32 { Err(ChachaPolyError::ApiMisuse("Invalid key length"))? }
		if nonce.len() != 12 { Err(ChachaPolyError::ApiMisuse("Invalid nonce length"))? }
		if plaintext.len() > CHACHA20_MAX { Err(ChachaPolyError::ApiMisuse("Too much data"))? }
		if plaintext.len() > buf.len() { Err(ChachaPolyError::ApiMisuse("Buffer is too small"))? }
		
		// Fill `buf` and encrypt the data in place
		buf[..plaintext.len()].copy_from_slice(plaintext);
		Self::xor(key, nonce, 0, &mut buf[..plaintext.len()]);
		Ok(plaintext.len())
	}
	
	fn decrypt(&self, buf: &mut[u8], ciphertext_len: usize, key: &[u8], nonce: &[u8])
		-> Result<usize, Box<dyn Error + 'static>>
	{
		self.encrypt(buf, ciphertext_len, key, nonce)
	}
	fn decrypt_to(&self, buf: &mut[u8], ciphertext: &[u8], key: &[u8], nonce: &[u8])
		-> Result<usize, Box<dyn Error + 'static>>
	{
		self.encrypt_to(buf, ciphertext, key, nonce)
	}
}
