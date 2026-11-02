use save_relay::oauth::{base64url, challenge, verifier_from_bytes};

/// RFC 7636 appendix B: the verifier's 32 octets.
const RFC_OCTETS: [u8; 32] = [
    116, 24, 223, 180, 151, 153, 224, 37, 79, 250, 96, 125, 216, 173, 187, 186, 22, 212, 37, 77,
    105, 214, 191, 240, 91, 88, 5, 88, 83, 132, 141, 121,
];
const RFC_VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const RFC_CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

#[test]
fn base64url_rfc4648_vectors_without_padding() {
    let cases: [(&[u8], &str); 7] = [
        (b"", ""),
        (b"f", "Zg"),
        (b"fo", "Zm8"),
        (b"foo", "Zm9v"),
        (b"foob", "Zm9vYg"),
        (b"fooba", "Zm9vYmE"),
        (b"foobar", "Zm9vYmFy"),
    ];
    for (bytes, want) in cases {
        assert_eq!(base64url(bytes), want, "{bytes:?}");
    }
}

#[test]
fn base64url_uses_dash_and_underscore() {
    assert_eq!(base64url(b"hi?"), "aGk_");
    assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    assert_eq!(base64url(&[0xfb, 0xef, 0xbe]), "----");
    assert_eq!(base64url(&[0xff, 0xff, 0xff]), "____");
}

#[test]
fn verifier_is_43_unreserved_characters() {
    assert_eq!(verifier_from_bytes(&RFC_OCTETS), RFC_VERIFIER);
    let zeros = verifier_from_bytes(&[0u8; 32]);
    assert_eq!(zeros, "A".repeat(43));
    let ones = verifier_from_bytes(&[0xffu8; 32]);
    assert_eq!(ones.len(), 43);
    assert!(ones
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b)));
}

#[test]
fn challenge_is_s256_of_the_verifier() {
    assert_eq!(challenge(RFC_VERIFIER), RFC_CHALLENGE);
    assert_eq!(challenge(""), "47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU");
    assert_ne!(challenge("a"), challenge("b"));
    assert_eq!(challenge(RFC_VERIFIER).len(), 43);
}
