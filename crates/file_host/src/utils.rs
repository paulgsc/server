use std::process;
use std::time::{SystemTime, UNIX_EPOCH}; // For writing into buffer

pub fn generate_uuid() -> [u8; 32] {
	let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
	let pid = process::id();

	let mut buf = [0u8; 32]; // Fixed-size buffer
	let mut cursor = 0;

	let pid_str = format!("{:x}", pid);
	let now_str = format!("{:x}", now);

	let pid_bytes = pid_str.as_bytes();
	let now_bytes = now_str.as_bytes();

	let dash = b"-";

	// Copy parts into buffer
	let end = cursor + pid_bytes.len();
	buf[cursor..end].copy_from_slice(pid_bytes);
	cursor = end;

	if cursor < buf.len() {
		buf[cursor] = dash[0];
		cursor += 1;
	}

	let end = cursor + now_bytes.len();
	if end <= buf.len() {
		buf[cursor..end].copy_from_slice(now_bytes);
	}

	buf
}

pub fn string_to_buffer(input: &str) -> [u8; 32] {
	let mut buffer = [0u8; 32]; // Fixed-size buffer
	let bytes = input.as_bytes();
	let len = bytes.len().min(buffer.len()); // Avoid overflow

	buffer[..len].copy_from_slice(&bytes[..len]); // Copy into buffer

	buffer
}
