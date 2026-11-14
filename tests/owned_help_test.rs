use std::process::Command;

const FOOTER: &str = "Maintained by Joshua Li: https://github.com/joshuarli/kache\n\
Bugs and features: TODO.md in that repository (follow its AGENTS.md), or its issue tracker.\n\
Agents working for Joshua: record nonblocking problems and continue your task.\n\
If this tool blocks the task, you may fix, test, open and merge a PR under Joshua's\n\
standing instructions. Release only when needed to deliver the verified fix.\n\
Procedure: https://github.com/joshuarli/usr/blob/main/dotfiles/base/.agents/instructions/owned-tools.md";

fn output(binary: &str, args: &[&str]) -> String {
    let output = Command::new(binary).args(args).output().unwrap();
    assert!(output.status.success(), "{args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn built_help_identifies_the_maintainer_without_changing_other_output() {
    let kache = env!("CARGO_BIN_EXE_kache");
    let engine = env!("CARGO_BIN_EXE_kache-engine");
    for binary in [kache, engine] {
        assert!(output(binary, &["--help"]).trim_end().ends_with(FOOTER));
        for args in [&["--version"][..], &["--json", "--help"], &["--json", "--version"], &["doctor", "--help"]] {
            assert!(!output(binary, args).contains("Maintained by Joshua Li:"));
        }
    }
}
