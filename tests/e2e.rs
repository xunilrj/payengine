use insta::{Settings, assert_snapshot};
use libtest_mimic::{Arguments, Failed, Trial};
use serde::Deserialize;
use std::{
    collections::HashMap, ffi::OsString, path::PathBuf, process::Output, str::FromStr as _,
    sync::Once, time::Duration,
};

#[derive(Deserialize)]
struct TestSpec {
    args: Option<Vec<String>>,
    env: Option<HashMap<String, String>>,
}

static EXE_COMPILATION: Once = Once::new();

fn compile_exe() {
    let args = vec!["b", "-r"];
    let o = std::process::Command::new("cargo")
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success());
}

fn run_exe(
    args: impl IntoIterator<Item = impl Into<OsString>>,
    env_overrides: HashMap<String, String>,
) -> String {
    let cargo_manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let repo_root = PathBuf::from_str(&cargo_manifest_dir)
        .unwrap()
        .to_path_buf();
    let exe_path = repo_root.join("target/release/payengine");

    // On duct, first env var write, wins.
    let mut cmd = duct::cmd(&exe_path, args).dir(repo_root.clone());
    for (k, v) in &env_overrides {
        cmd = cmd.env(k, v);
    }
    for (k, v) in [("RUST_LOG", "trace"), ("RUST_LOG_NO_TIMESTAMP", "1")] {
        cmd = cmd.env(k, v);
    }

    let o = cmd
        .stderr_to_stdout()
        .stdout_capture()
        .unchecked()
        .start()
        .unwrap();

    let Some(Output { status, stdout, .. }) = o.wait_timeout(Duration::from_secs(30)).unwrap()
    else {
        panic!("test timeout");
    };

    let mut snapshot = format!("{status}\n\n");
    snapshot.push_str(str::from_utf8(&stdout).unwrap());
    snapshot
}

fn main() {
    let args = Arguments::from_args();

    let mut tests = vec![];

    for pattern in ["tests/**/*.csv", "tests/**/*.toml"] {
        let mut iter = glob::glob(pattern)
            .expect("Failed to read glob pattern")
            .into_iter();
        while let Some(entry) = iter.next() {
            match entry {
                Ok(path) => {
                    let test_name = path.display().to_string();
                    let trial = if path.extension().is_some_and(|e| e == "toml") {
                        Trial::test(test_name, move || run_toml(path.clone()))
                    } else {
                        Trial::test(test_name, move || run_csv(path.clone()))
                    };
                    tests.push(trial);
                }
                Err(_) => todo!(),
            }
        }
    }

    libtest_mimic::run(&args, tests).exit();
}

fn run_csv(path: PathBuf) -> Result<(), Failed> {
    EXE_COMPILATION.call_once(|| {
        compile_exe();
    });
    let output = run_exe([path.as_os_str()], HashMap::new());
    let output = clean_output(&output);
    assert_snapshot_for(&path, output);
    Ok(())
}

fn run_toml(path: PathBuf) -> Result<(), Failed> {
    EXE_COMPILATION.call_once(|| {
        compile_exe();
    });
    let content = std::fs::read_to_string(&path).unwrap();
    let spec: TestSpec = toml::from_str(&content).unwrap();
    let args = spec.args.unwrap_or_default();
    let env = spec.env.unwrap_or_default();

    let output = run_exe(args, env);
    let output = clean_output(&output);
    assert_snapshot_for(&path, output);
    Ok(())
}

fn assert_snapshot_for(path: &PathBuf, output: String) {
    let snapshot_name = path.file_name().unwrap().display().to_string();

    let mut snapshot_path = path.parent().unwrap().components();
    snapshot_path.next();
    let snapshot_path = snapshot_path.as_path();

    let mut settings = Settings::clone_current();
    settings.set_snapshot_path(snapshot_path);
    settings.set_prepend_module_to_snapshot(false);
    settings.set_omit_expression(true);
    settings.bind(|| {
        assert_snapshot!(snapshot_name, output);
    });
}

fn clean_output(output: &str) -> String {
    #[derive(Default)]
    struct RawText(String);

    impl vte::Perform for RawText {
        fn print(&mut self, c: char) {
            self.0.push(c);
        }

        fn execute(&mut self, _: u8) {}

        fn hook(&mut self, _: &vte::Params, _: &[u8], _: bool, _: char) {}

        fn put(&mut self, b: u8) {
            self.0.push(b as char);
        }

        fn unhook(&mut self) {}

        fn osc_dispatch(&mut self, _: &[&[u8]], _: bool) {}

        fn csi_dispatch(&mut self, _: &vte::Params, _: &[u8], _: bool, _: char) {}

        fn esc_dispatch(&mut self, _: &[u8], _: bool, _: u8) {}
    }

    let mut raw = String::new();
    for line in output.lines() {
        let mut performer = RawText::default();
        let mut p = vte::Parser::new();
        p.advance(&mut performer, line.as_bytes());
        raw.push_str(&performer.0);
        raw.push('\n');
    }

    raw.to_string()
}
