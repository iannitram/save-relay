use save_relay::remote::{MemoryRemote, Precondition, Remote, RemoteEntry};

const ANY: &Precondition = &Precondition::Any;

fn sample() -> MemoryRemote {
    let remote = MemoryRemote::new();
    for dir in ["g", "g/sub", "gx"] {
        remote.mkcol(dir).unwrap();
    }
    remote.put("g/a", b"A", ANY).unwrap();
    remote.put("g/sub/b", b"BB", ANY).unwrap();
    remote.put("gx/a", b"X", ANY).unwrap();
    remote
}

fn listing(remote: &MemoryRemote) -> Vec<(String, Vec<RemoteEntry>)> {
    fn visit(remote: &MemoryRemote, dir: &str, entries: &mut Vec<(String, Vec<RemoteEntry>)>) {
        let children = remote.list(dir).unwrap();
        for child in &children {
            if child.is_dir {
                let path = if dir.is_empty() {
                    child.name.clone()
                } else {
                    format!("{dir}/{}", child.name)
                };
                visit(remote, &path, entries);
            }
        }
        entries.push((dir.to_string(), children));
    }

    let mut entries = Vec::new();
    visit(remote, "", &mut entries);
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}

#[test]
fn moving_a_folder_into_itself_or_its_subtree_refuses_without_changes() {
    let remote = sample();

    for to in ["g", "g/sub/x"] {
        let before = listing(&remote);
        assert!(remote.move_to("g", to).is_err(), "move to {to} should fail");
        assert_eq!(listing(&remote), before, "move to {to} changed the tree");
    }
}

#[test]
fn moving_a_folder_to_a_sibling_with_a_shared_prefix_works() {
    let remote = sample();

    remote.move_to("g", "gx/y").unwrap();

    assert_eq!(remote.get("gx/y/a").unwrap().0, b"A");
    assert_eq!(remote.get("gx/y/sub/b").unwrap().0, b"BB");
    assert_eq!(remote.get("gx/a").unwrap().0, b"X");
    assert!(remote.list("g").is_err());
}
