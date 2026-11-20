//! Go-oracle test cases that need the full bash AST engine (the mvdan-level
//! analysis `internal/base/shellparse` carries: SplitTopLevel,
//! AnalyzeApprovalFeatures, PeelWrappers, CompoundLeaves). Each is unignored
//! by the shell-analysis slice that installs that engine via
//! [`crate::permission::bash::set_bash_analysis`]; the case tables are the
//! oracle, carried verbatim.

#![allow(non_snake_case)]

use crate::permission::{Policy, bash_analysis};

fn static_engine_only() -> bool {
    // The anchor only holds while the AST engine is absent.
    let probe = bash_analysis();
    // StaticBashAnalysis decomposes nothing.
    probe.decompose("git status && ls").is_none()
}

fn cmd_raw(cmd: &str) -> Box<serde_json::value::RawValue> {
    let json = format!(r#"{{"command":{}}}"#, serde_json::to_string(cmd).unwrap());
    serde_json::value::RawValue::from_string(json).unwrap()
}

fn cmd_args(cmd: &str) -> Box<serde_json::value::RawValue> {
    cmd_raw(cmd)
}

/// wrapperVerdicts (bash_wrapper_test.go): each command's verdict against the
/// deny rule Bash(rm:*) under mode allow. Wrapper peeling lives in the AST
/// engine's raw-prefix matching.
#[test]
#[ignore = "anchor: needs the AST engine (PeelWrappers/raw prefix fields); unignore with the shell-analysis slice"]
fn wrapped_program_matches_deny_rule() {
    assert!(static_engine_only());
    let cases: &[(&str, crate::permission::Decision)] = &[
        ("rm -rf b", crate::permission::Decision::Deny),
        ("env X=1 rm -rf b", crate::permission::Decision::Deny),
        ("env -i rm b", crate::permission::Decision::Deny),
        ("sudo rm -rf b", crate::permission::Decision::Deny),
        ("command -v rm", crate::permission::Decision::Allow),
        ("(rm b)", crate::permission::Decision::Ask),
        ("xargs rm", crate::permission::Decision::Ask),
        ("echo $(rm b)", crate::permission::Decision::Ask),
        ("timeout 5 rm b", crate::permission::Decision::Allow),
        ("RM b", crate::permission::Decision::Allow),
    ];
    for (cmd, want) in cases {
        let p = Policy::new("allow", &[], &[], &["Bash(rm:*)"]);
        let got = p.decide_subject("bash", false, cmd);
        assert_eq!(got, *want, "{cmd:?}");
    }
}

/// TestPolicyDecideCompoundBash (bash_decompose_test.go): prefix rules cover
/// compound flows segment by segment.
#[test]
#[ignore = "anchor: needs the AST engine (SplitTopLevel); unignore with the shell-analysis slice"]
fn policy_decide_compound_bash() {
    assert!(static_engine_only());
    let p = Policy::new(
        "ask",
        &[
            "Bash(git add:*)",
            "Bash(git commit:*)",
            "Bash(git push:*)",
            "Bash(go test:*)",
            "Bash(sudo chmod:*)",
        ],
        &[],
        &["Bash(rm -rf*)"],
    );
    let cases: &[(&str, crate::permission::Decision)] = &[
        (
            r#"git add . && git commit -m "wip" && git push"#,
            crate::permission::Decision::Allow,
        ),
        (
            r#"git add . && git commit -m "wip" && git push && npm publish"#,
            crate::permission::Decision::Ask,
        ),
        (
            r#"git add . && rm -rf /tmp/scratch"#,
            crate::permission::Decision::Deny,
        ),
        (
            r#"echo starting && git add . && ls -la"#,
            crate::permission::Decision::Allow,
        ),
        (
            r#"sudo chmod 644 /etc/foo || echo "chmod failed""#,
            crate::permission::Decision::Allow,
        ),
        (
            r#"sudo chmod 644 /etc/foo 2>/dev/null || echo "chmod failed""#,
            crate::permission::Decision::Allow,
        ),
        (
            r#"sudo chmod 644 /etc/foo > chmod.log || echo "chmod failed""#,
            crate::permission::Decision::Ask,
        ),
        (
            r#"go test ./... 2>&1 | head -20"#,
            crate::permission::Decision::Allow,
        ),
        ("git push origin main", crate::permission::Decision::Allow),
    ];
    for (subject, want) in cases {
        assert_eq!(
            p.decide_subject("bash", false, subject),
            *want,
            "{subject:?}"
        );
    }
}

/// TestBashSubjectRequiresExplicitApproval + TestReadOnlyCompoundStatementsNeedNoApproval
/// (bash_approval_test.go): the approval classification table.
#[test]
#[ignore = "anchor: needs the AST engine (AnalyzeApprovalFeatures); unignore with the shell-analysis slice"]
fn bash_subject_requires_explicit_approval() {
    assert!(static_engine_only());
    use crate::permission::{BashApprovalBlocker, BashApprovalClass, bash_analysis};
    let human = [
        "git status $(touch /tmp/x)",
        "git status `touch /tmp/x`",
        "diff <(touch /tmp/x) expected",
        "tee >(touch /tmp/x)",
        r#"eval "touch /tmp/x""#,
        "source ./script.sh",
        ". ./script.sh",
        r#"builtin eval "touch /tmp/x""#,
        "command",
        "command eval x",
        r#"bash -lc "touch /tmp/x""#,
        r#"pwsh -Command "New-Item x""#,
        r#"cmd /c "echo x > file""#,
        r#"python3 -c "open('x','w').close()""#,
        r#"node -e "require('fs').writeFileSync('x','')""#,
        r#"node --eval="require('fs').writeFileSync('x','')""#,
        r#"find . -exec touch {} ;"#,
        "cat <<EOF\n$(touch /tmp/x)\nEOF",
        "cd /tmp/x && python3 - <<'EOF'\nimport os\nEOF",
        "cd /w && cat > pkg/x.go <<'EOF'\npackage pkg\nEOF",
        "for f in *.go; do rm $f; done",
    ];
    for subject in human {
        assert_ne!(
            bash_analysis().approval_blocker(subject),
            BashApprovalBlocker::None,
            "{subject:?} needs a human"
        );
        assert_eq!(
            bash_analysis().approval_class(subject),
            BashApprovalClass::RequireHuman,
            "{subject:?} exact"
        );
    }
    let plain = [
        "git status --short",
        "git status && npm test",
        "git status 2>/dev/null",
        "sudo chmod 644 file",
        "builtin printf '%s\\n' ok",
        "command -v uv",
        "command -V pytest",
        "command git status",
        "python3 script.py",
    ];
    for subject in plain {
        assert_eq!(
            bash_analysis().approval_blocker(subject),
            BashApprovalBlocker::None,
            "{subject:?} needs no human"
        );
    }
}

/// TestIsReadOnlyBashSubject (bash_readonly_test.go): the read-only
/// classification table from shellsafe's shared tables.
#[test]
#[ignore = "anchor: needs the shellsafe read-only tables port (ClassifyReadOnlyCommand); unignore with the shellsafe slice"]
fn is_read_only_bash_subject() {
    assert!(static_engine_only());
    use crate::permission::bash_analysis;
    let read_only = [
        "ls",
        "ls /tmp",
        "cat main.go",
        "head -n 5 file.txt",
        "find . -name '*.go'",
        "grep TODO *.go",
        "echo hello",
        "pwd",
        "cd /tmp/project",
        "whoami",
        "wc -l file.txt",
        "stat main.go",
        "du -sh .",
        "diff a.go b.go",
        "printenv PATH",
        "Get-Process -Name mongod",
        "Get-ChildItem -Path .",
        "Get-NetTCPConnection -LocalPort 6379",
        r#"basename "$(pwd)""#,
        "git log",
        "git status",
        "git diff",
        "git show HEAD",
        "git blame main.go",
        "git log 2>/dev/null",
        "git log >/dev/null",
        "git log >$null",
        "git log >NUL",
        "git log 2>&1",
        "git log &>/dev/null",
        "git remote",
        "git branch",
        "go vet ./...",
        "go doc fmt",
        "go list ./...",
        "gh pr checks 12",
        "gh api repos/o/r/actions/runs --jq .x",
    ];
    for cmd in read_only {
        assert!(bash_analysis().is_read_only(cmd), "{cmd:?} is a read");
    }
    let writers = [
        "rm file.txt",
        "rm -rf /",
        "env rm -rf /",
        "git commit -m 'msg'",
        "git branch feature/new",
        "git push",
        "git push --force",
        "cd /tmp && rm file.txt",
        "go build ./...",
        "go fmt ./...",
        "go test ./...",
        "ls; rm file.txt",
        "git status && rm file.txt",
        "cat main.go | tee copy.go",
        "cat file > output.txt",
        "git diff > changes.patch",
        "cat < input.txt",
        "diff <(sort a.txt) <(sort b.txt)",
        "ls $(touch output.txt)",
        "ls `touch output.txt`",
        "ls || rm file.txt",
        "ls & rm file.txt",
        "find . -name '*.go' -exec rm {} \\;",
        "find . -name '*.tmp' -delete",
        "awk '{print $1}' file.txt",
        "sed 's/a/b/' file.txt",
        "sort -o sorted.txt input.txt",
        "tee out.txt",
        "xargs rm",
        "make build",
        "curl https://example.com",
        "npm install",
        "chmod 777 file",
        "Start-Process mongod",
        "Stop-Process -Name mongod",
        "Set-Content style.css bad",
        "",
        "Test-NetConnection -ComputerName localhost -Port 27017",
        "git remote add origin git@example.com:x/y",
        "git config --global user.name Xinwei",
        "git stash",
        "git stash push",
        "git archive --output repo.tar HEAD",
        "git bundle create repo.bundle HEAD",
        "git diff --output changes.patch",
        "git show --output=changes.patch HEAD",
        "git log > changes.patch",
        "git log >$nullish",
        "git log >nul.txt",
        "git log < /dev/null",
        "go env -w GOPROXY=https://proxy.golang.org,direct",
        "go env -u GOPROXY",
    ];
    for cmd in writers {
        assert!(!bash_analysis().is_read_only(cmd), "{cmd:?} is not a read");
    }
}

/// TestBashReadOnlyProof (bash_readonly_test.go): the structural reason a
/// call was not proven read-only.
#[test]
#[ignore = "anchor: needs the shellsafe tables + planmode Proof port; unignore with the shellsafe slice"]
fn bash_read_only_proof_names_what_stopped_the_command() {
    assert!(static_engine_only());
    // Proof values live with the shellsafe slice; the table here mirrors
    // bash_readonly_test.go TestBashReadOnlyProofNamesWhatStoppedTheCommand.
    let cases: &[(&str, &str)] = &[
        ("ls -la", ""),
        ("ls | wc -l", ""),
        ("curl -s https://example.com", "unknown_program:curl"),
        ("git push origin main", "unknown_program:git push"),
        ("ls > out.txt", "shell_construct:shell redirection"),
        ("find . -exec rm {} ;", "write_arguments:find"),
        ("ls && sort -o f f", "write_arguments:sort"),
    ];
    for (cmd, _proof) in cases {
        let _ = cmd_args(cmd);
    }
}

/// TestNetworkPathShellCommandsAskInDefaultMode (network_operands_test.go,
/// Windows-only): operands naming network paths defeat read-only
/// classification and remembered prefix rules.
#[test]
#[cfg(windows)]
#[ignore = "anchor: needs the shellsafe AST walk (wordSkeletons/operandForms); unignore with the shellsafe slice"]
fn network_path_shell_commands_ask_in_default_mode() {
    assert!(static_engine_only());
    let p = Policy::new("ask", &[], &[], &[]);
    for cmd in [
        r"type \\evil\x",
        "cat //evil/x",
        "ls //evil/x",
        "grep -r x //evil/x",
        "cat //evil/x > out.txt",
        "cat < //evil/x",
        "env cat //evil/x",
        "command cat //evil/x",
        "xargs cat //evil/x",
        "time cat //evil/x",
    ] {
        assert_eq!(
            p.decide("bash", false, &cmd_raw(cmd)),
            crate::permission::Decision::Ask,
            "{cmd:?} must ask"
        );
    }
    for cmd in [
        "echo a,b",
        "cut -d, -f1 a.csv",
        "git log --format=%h,%s",
        "cat a.txt",
    ] {
        assert_eq!(
            p.decide("bash", false, &cmd_raw(cmd)),
            crate::permission::Decision::Allow,
            "{cmd:?} is local"
        );
    }
}

/// TestNormalizeBashSafeRedirectsForMatch (bash_redirect_test.go): safe
/// redirects are stripped for matching; file redirects never are.
#[test]
#[ignore = "anchor: needs the shellsafe AST redirect spans; unignore with the shellsafe slice"]
fn normalize_bash_safe_redirects_for_match() {
    assert!(static_engine_only());
    let safe = [
        ("git log 2>/dev/null", "git log"),
        ("git log > /dev/null", "git log"),
        ("git log >>/dev/null", "git log"),
        ("git log &>/dev/null", "git log"),
        ("git log >$null", "git log"),
        ("git log >NUL", "git log"),
        ("git log 2>&1", "git log"),
        ("git log >&2", "git log"),
    ];
    for (input, want) in safe {
        assert_eq!(
            crate::permission::network::normalize_bash_safe_redirects_for_match(input),
            Some(want.to_string()),
            "{input:?}"
        );
    }
    for input in [
        "git log > changes.patch",
        "git log >/dev/null.log",
        "git log >$nullish",
        "git log >nul.txt",
        "git log < /dev/null",
        "git log 2>&1rm",
        "cat <<'EOF'\n>/dev/null\nEOF",
    ] {
        assert_eq!(
            crate::permission::network::normalize_bash_safe_redirects_for_match(input),
            None,
            "{input:?} stays unsafe"
        );
    }
}

/// TestGateReportsMatchedPermissionRule (permission_test.go): compound bash
/// rule provenance uses raw-prefix matching over segments.
#[test]
#[ignore = "anchor: needs the AST engine (raw prefix over static fields with dynamic payloads); unignore with the shell-analysis slice"]
fn gate_reports_matched_permission_rule() {
    assert!(static_engine_only());
    let p = Policy::new("allow", &[], &["Bash(git push:*)"], &[]);
    let args = cmd_raw("git status && git push origin main");
    assert_eq!(
        p.decide("bash", false, &args),
        crate::permission::Decision::Ask
    );
    assert_eq!(
        p.matched_rule("bash", crate::permission::Decision::Ask, &args)
            .as_deref(),
        Some("Bash(git push:*)")
    );
}
