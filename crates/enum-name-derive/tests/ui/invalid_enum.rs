use enum_name_derive::EnumFilenameAndFromString;

#[derive(EnumFilenameAndFromString)]
pub struct NotAnEnum {} // ❌ Should fail: the derive only supports enums!

fn main() {}
