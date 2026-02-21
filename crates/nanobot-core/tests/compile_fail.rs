#[test]
fn capability_boundary_compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
