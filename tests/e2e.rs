use insta::{Settings, assert_debug_snapshot, assert_snapshot};
use libtest_mimic::{Arguments, Failed, Trial};
use std::{path::PathBuf, process::Output, str::FromStr as _, sync::Once, time::Duration};

static EXE_COMPILATION: Once = Once::new();

fn compile_exe() {
    let args = vec!["b", "-r"];
    let o = std::process::Command::new("cargo")
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success());
}

fn run_exe(arg: String) -> String {
    let cargo_manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let repo_root = PathBuf::from_str(&cargo_manifest_dir)
        .unwrap()
        .to_path_buf();
    let exe_path = repo_root.join("target/release/payengine");
    let o = duct::cmd!(&exe_path, &arg)
        .dir(repo_root.clone())
        .env("RUST_LOG", "trace")
        .env("RUST_LOG_NO_TIMESTAMP", "1")
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
    let mut iter = glob::glob("tests/**/*.csv")
        .expect("Failed to read glob pattern")
        .into_iter();
    while let Some(entry) = iter.next() {
        match entry {
            Ok(path) => {
                let test_name = path.display().to_string();
                tests.push(Trial::test(test_name, move || run_csv(path.clone())));
            }
            Err(_) => {
                todo!()
            }
        }
    }

    libtest_mimic::run(&args, tests).exit();
}

fn run_csv(path: PathBuf) -> Result<(), Failed> {
    EXE_COMPILATION.call_once(|| {
        compile_exe();
    });
    let output = run_exe(path.display().to_string());
    let output = clean_output(&output);

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

    Ok(())
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
