// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Mutex;

use fos_core::SecretKey;

use super::*;

/// The .app folders that `apps` (scripts/apps.py) builds.
fn bundle(name: &str) -> Bundle {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/apps")
        .join(name);
    assert!(
        dir.exists(),
        "{} is missing: run `apps` first",
        dir.display()
    );
    Bundle::read_dir(&dir).unwrap()
}

#[test]
fn the_capability_engine_runs() {
    let runtime = Runtime::new().unwrap();
    let mut engine = runtime.engine(&bundle("Capability.app")).unwrap();
    let (alice, bob) = (
        SecretKey::from_seed(&[1; 32]).public(),
        SecretKey::from_seed(&[2; 32]).public(),
    );
    let folder = Hash::of(b"folder");

    let state = engine.init(folder, alice);
    assert_eq!(
        engine.check(&state, alice, Ability::Write, folder, "/a"),
        Decision::Allow
    );
    assert!(matches!(
        engine.check(&state, bob, Ability::Read, folder, "/a"),
        Decision::Deny(_)
    ));

    let policy = engine.grant(bob, &[Ability::Read, Ability::Store], "/");
    assert_eq!(
        String::from_utf8(policy.clone()).unwrap(),
        format!("grant {bob} read,store /")
    );
    let state = engine.apply(&state, alice, &policy).unwrap();
    assert_eq!(
        engine.check(&state, bob, Ability::Read, folder, "/a"),
        Decision::Allow
    );
    assert_eq!(engine.storers(&state), vec![alice, bob]);
    engine.apply(&state, bob, b"nonsense").unwrap_err();
    // An engine is not an app.
    runtime
        .app(&bundle("Capability.app"), Arc::new(Host::default()))
        .unwrap_err();
}

#[derive(Default)]
struct Host {
    files: BTreeMap<String, Vec<u8>>,
    kv: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl AppHost for Host {
    fn read(&self, _: &str, path: &str) -> Result<Vec<u8>, FsError> {
        self.files.get(path).cloned().ok_or(FsError::NotFound)
    }
    fn write(&self, _: &str, _: &str, _: Vec<u8>) -> Result<(), FsError> {
        Err(FsError::Denied("read only".into()))
    }
    fn append(&self, _: &str, _: &str, _: Vec<u8>) -> Result<(), FsError> {
        Err(FsError::Denied("read only".into()))
    }
    fn delete(&self, _: &str, _: &str) -> Result<(), FsError> {
        Err(FsError::Denied("read only".into()))
    }
    fn list(&self, _: &str, _: &str) -> Result<Vec<String>, FsError> {
        Ok(Vec::new())
    }
    fn kv_get(&self, _: &str, key: &str) -> Option<Vec<u8>> {
        self.kv.lock().unwrap().get(key).cloned()
    }
    fn kv_set(&self, _: &str, key: &str, value: Vec<u8>) {
        self.kv.lock().unwrap().insert(key.into(), value);
    }
    fn kv_delete(&self, _: &str, key: &str) {
        self.kv.lock().unwrap().remove(key);
    }
    fn kv_keys(&self, _: &str, prefix: &str) -> Vec<String> {
        self.kv
            .lock()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }
    fn log(&self, _: &str, _: &str) {}
}

#[test]
fn the_photos_app_indexes_a_photo() {
    let runtime = Runtime::new().unwrap();
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend_from_slice(&64u32.to_be_bytes());
    png.extend_from_slice(&32u32.to_be_bytes());
    let mut host = Host::default();
    host.files.insert("/photos/a.png".into(), png);
    let host = Arc::new(host);

    let mut app = runtime
        .app(&bundle("Photos.app"), Arc::<Host>::clone(&host))
        .unwrap();
    assert_eq!(app.manifest().name, "Photos");
    assert!(app.watches("/photos/a.png"));
    assert!(!app.watches("/photosx"));
    app.on_change("/photos/a.png").unwrap();
    assert_eq!(
        host.kv_get("photos", "photo:/photos/a.png").as_deref(),
        Some(&b"64x32"[..])
    );
    // A deleted file leaves the index.
    app.on_change("/photos/b.png").unwrap();
    assert_eq!(host.kv_keys("photos", "photo:"), ["photo:/photos/a.png"]);
}
