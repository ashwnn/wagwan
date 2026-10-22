//! Startup configuration is exercised in child processes so tests never mutate
//! the shared process environment or make network requests.
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "wagwan-config-cli-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn import(&self, variables: &[(&str, String)]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_wagwan"));
        command
            .env_clear()
            .env("WAGWAN_DATA_DIR", self.0.join("default-data"));
        for (key, value) in variables {
            command.env(key, value);
        }
        command
            .arg("import-opml")
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nested-opml.xml"));
        command.output().unwrap()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn data_directory_secret_file_takes_precedence_in_actual_cli_import() {
    let sandbox = Sandbox::new();
    let chosen = sandbox.0.join("chosen-data");
    let ignored = sandbox.0.join("ignored-data");
    let secret = sandbox.0.join("data-dir.secret");
    fs::write(&secret, format!("{}\r\n", chosen.display())).unwrap();
    let variables = [
        ("WAGWAN_DATA_DIR", ignored.display().to_string()),
        ("WAGWAN_DATA_DIR_FILE", secret.display().to_string()),
    ];
    let first = sandbox.import(&variables);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(String::from_utf8_lossy(&first.stdout).contains("Imported 3 feeds"));
    assert!(chosen.join("wagwan.sqlite3").is_file());
    assert!(!ignored.exists());
    let second = sandbox.import(&variables);
    assert!(second.status.success());
    assert!(String::from_utf8_lossy(&second.stdout).contains("Imported 0 feeds"));
}

#[test]
fn invalid_configuration_is_rejected_before_creating_the_database() {
    let sandbox = Sandbox::new();
    for (key, value) in [
        ("WAGWAN_TZ", "Not/A_Time_Zone"),
        ("WAGWAN_MIN_SCORE", "100"),
        ("WAGWAN_FETCH_CONCURRENCY", "0"),
        ("WAGWAN_FETCH_MIN_INTERVAL", "0s"),
        ("WAGWAN_LLM_BASE_URL", "http://unused.example.test/v1"),
    ] {
        let output = sandbox.import(&[(key, value.to_owned())]);
        assert!(!output.status.success(), "accepted invalid {key}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(key),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!sandbox.0.join("default-data").exists());
    }
}

#[test]
fn missing_secret_file_is_an_error_without_falling_back_to_direct_value() {
    let sandbox = Sandbox::new();
    let missing = sandbox.0.join("missing-secret");
    let output = sandbox.import(&[("WAGWAN_DATA_DIR_FILE", missing.display().to_string())]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("could not read WAGWAN_DATA_DIR_FILE")
    );
    assert!(!sandbox.0.join("default-data").exists());
}
