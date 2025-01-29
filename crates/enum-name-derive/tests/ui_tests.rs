#[test]
fn ui_tests() {
	let t = trybuild::TestCases::new();
	t.pass("tests/ui/valid_enum.rs");
	t.compile_fail("tests/ui/invalid_enum.rs"); // Ensures failure cases are handled
}
