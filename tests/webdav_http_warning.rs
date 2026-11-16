//! #250 item 8: `http://` to a host that is not this machine gets a warning at setup.
use save_relay::webdav::http_warning;

#[test]
fn plain_http_to_a_remote_host_warns() {
    let warning = http_warning("http://cloud.example.org/nextcloud").expect("a warning");
    assert!(warning.contains("http://"), "{warning}");
    assert_eq!(http_warning("http://192.168.1.20:8080"), None);
}

#[test]
fn https_and_loopback_hosts_do_not_warn() {
    assert_eq!(http_warning("https://cloud.example.org"), None);
    assert_eq!(http_warning("http://localhost:8080"), None);
    assert_eq!(http_warning("http://127.0.0.1:8099/"), None);
    assert_eq!(http_warning("http://[::1]:8080"), None);
    assert_eq!(http_warning("http://10.1.2.3"), None);
    assert_eq!(http_warning("http://172.31.2.3"), None);
    assert_eq!(http_warning("http://192.168.1.20"), None);
    assert_eq!(http_warning("http://[fd00::1]:8080"), None);
    assert!(http_warning("http://172.32.2.3").is_some());
}
