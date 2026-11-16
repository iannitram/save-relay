//! `include/save_relay.h` is written by hand: every `#[no_mangle]` function in `src/ffi.rs`
//! must appear in it, with the same signature.

fn c_type(rust: &str) -> String {
    let rust = rust.trim();
    if let Some(inner) = rust.strip_prefix("*const ") {
        return format!("const {} *", c_type(inner));
    }
    if let Some(inner) = rust.strip_prefix("*mut ") {
        return format!("{} *", c_type(inner));
    }
    match rust {
        "c_char" => "char",
        "c_int" => "int",
        "u8" => "uint8_t",
        "u16" => "uint16_t",
        "usize" => "size_t",
        "SaveRelayHost" => "SaveRelayHost",
        "SaveRelayHttpSink" => "SaveRelayHttpSink",
        "" => "void",
        other => panic!("unhandled exported type: {other}"),
    }
    .into()
}

fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

#[test]
fn every_exported_signature_matches_the_header() {
    let source = std::fs::read_to_string("src/ffi.rs").unwrap();
    let header = std::fs::read_to_string("include/save_relay.h").unwrap();
    let mut seen = 0;
    for export in source.split("#[no_mangle]").skip(1) {
        let signature = export.split('{').next().unwrap();
        let (name, rest) = signature
            .split_once("fn ")
            .unwrap()
            .1
            .split_once('(')
            .unwrap();
        let (args, result) = rest.split_once(')').unwrap();
        let args = args
            .split(',')
            .filter(|arg| !arg.trim().is_empty())
            .map(|arg| {
                let (name, rust) = arg.split_once(':').unwrap();
                format!("{} {}", c_type(rust), name.trim())
            })
            .collect::<Vec<_>>()
            .join(", ");
        let result = c_type(result.trim().strip_prefix("->").unwrap_or(""));
        let declaration = format!("{result} {name}({args});");
        assert!(
            compact(&header).contains(&compact(&declaration)),
            "include/save_relay.h lacks generated declaration: {declaration}"
        );
        seen += 1;
    }
    assert!(seen >= 7, "found only {seen} exported functions");
}
