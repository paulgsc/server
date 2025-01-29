
use enum_name_derive::EnumFilename;

#[derive(EnumFilename)]
pub struct NotAnEnum {} // ❌ Should fail: EnumFilename only supports enums!

fn main() {}

