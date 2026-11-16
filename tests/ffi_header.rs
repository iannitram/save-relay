//! `include/save_relay.h` is written by hand: every `#[no_mangle]` function in `src/ffi.rs`
//! must appear in it, with the same name.

#[test]
fn every_exported_function_is_in_the_header() {
    let source = std::fs::read_to_string("src/ffi.rs").unwrap();
    let header = std::fs::read_to_string("include/save_relay.h").unwrap();
    let mut seen = 0;
    let mut lines = source.lines();
    while let Some(line) = lines.next() {
        if line.trim() != "#[no_mangle]" {
            continue;
        }
        let signature = lines.next().expect("a function after #[no_mangle]");
        let name = signature
            .split("fn ")
            .nth(1)
            .and_then(|rest| rest.split('(').next())
            .expect("a function name")
            .trim();
        assert!(
            header.contains(&format!("{name}(")),
            "include/save_relay.h lacks {name}"
        );
        seen += 1;
    }
    assert!(seen >= 7, "found only {seen} exported functions");
}
