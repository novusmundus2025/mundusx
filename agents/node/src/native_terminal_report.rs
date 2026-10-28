//! Recognized test-runner evidence. A report is not an invented process exit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Report { pub passed: bool, pub executed: bool }

pub(super) fn clean(text: &str) -> String {
    let mut result=String::new();let mut chars=text.chars().peekable();
    while let Some(c)=chars.next() {
        if c=='\u{1b}'&&chars.peek()==Some(&'[') {chars.next();for c in chars.by_ref() {if c.is_ascii_alphabetic() {break;}}}
        else {result.push(c);}
    }
    result.to_ascii_lowercase()
}
fn counter(text: &str, key: &str) -> Option<u64> {
    text.lines().rev().find_map(|line|line.trim().trim_start_matches(['#','ℹ',' ']).strip_prefix(&format!("{key} "))?.trim().parse().ok())
}
pub(super) fn partial_node_pass(text: &str) -> Option<bool> {
    let (total, passed)=(counter(text,"tests")?,counter(text,"pass")?);
    Some(total>0&&passed>0&&counter(text,"fail").unwrap_or(0)==0&&counter(text,"cancelled").unwrap_or(0)==0)
}

pub(super) fn complete(text: &str) -> Option<Report> {
    let text=clean(text);
    if ["still running","has not completed","timed out","output exceeded terminal scrollback","failed to retrieve command output"].iter().any(|marker|text.contains(marker)) {return None;}
    let last=text.lines().rev().find(|line|!line.trim().is_empty())?.trim();
    if let (Some(total),Some(passed),Some(failed),Some(cancelled),Some(skipped),Some(todo))=(counter(&text,"tests"),counter(&text,"pass"),counter(&text,"fail"),counter(&text,"cancelled"),counter(&text,"skipped"),counter(&text,"todo")) {
        let duration=last.trim_start_matches(['#','ℹ',' ']).strip_prefix("duration_ms ")?.parse::<f64>().ok()?;
        if !duration.is_finite()||duration<0.0||passed.checked_add(failed)?.checked_add(cancelled)?.checked_add(skipped)?.checked_add(todo)?!=total {return None;}
        return Some(Report {passed:failed==0&&cancelled==0,executed:passed>0||failed>0||cancelled>0});
    }
    let rust=text.lines().filter(|line|line.starts_with("test result:")).collect::<Vec<_>>();
    if !rust.is_empty() {
        if rust.iter().any(|line|line.starts_with("test result: failed")) {return Some(Report {passed:false,executed:true});}
        let mut report=Report {passed:true,executed:false};
        for line in rust {
            let passed=line.strip_prefix("test result: ok. ")?.split_whitespace().next()?.parse::<u64>().ok()?;
            let failed=line.split_once(" passed; ")?.1.split_whitespace().next()?.parse::<u64>().ok()?;
            report.passed&=failed==0;report.executed|=passed>0||failed>0;
        }
        return Some(report);
    }
    if text.contains("build success")||text.contains("build failure") {
        if text.rfind("build failure")>text.rfind("build success") {return Some(Report {passed:false,executed:true});}
        let mut report=None;
        for line in text.lines().filter(|line|line.contains("tests run:")) {
            let count=|key:&str|->Option<u64>{line.split_once(key)?.1.trim().split([',',' ']).next()?.parse().ok()};
            let current=report.get_or_insert(Report {passed:true,executed:false});
            current.passed&=count("failures:")?==0&&count("errors:")?==0;
            current.executed|=count("tests run:")?>count("skipped:")?;
        }
        return report;
    }
    let summary=last.trim_matches('=').trim();
    let words=summary.split_whitespace().collect::<Vec<_>>();
    if words.windows(2).any(|w|w[0]=="in"&&w[1].trim_end_matches('s').parse::<f64>().is_ok()) {
        let count=|label:&str|words.windows(2).any(|w|w[0].parse::<u64>().is_ok_and(|n|n>0)&&w[1].trim_end_matches(',')==label);
        let failed=count("failed")||count("error")||count("errors");let passed=count("passed");
        if failed||passed||summary.contains("skipped") {return Some(Report {passed:!failed,executed:failed||passed});}
    }
    let mut report=None;
    for line in text.lines().filter(|line|line.starts_with("passed!")||line.starts_with("failed!")) {
        let count=|key:&str|->Option<u64>{line.split_once(key)?.1.trim().split([',',' ']).next()?.parse().ok()};
        let failed=count("failed:")?;let passed=count("passed:")?;count("total:")?;
        let current=report.get_or_insert(Report {passed:true,executed:false});
        current.passed&=failed==0;current.executed|=failed>0||passed>0;
    }
    if report.is_some() {return report;}
    if text.lines().any(|line|line=="fail"||line.starts_with("fail\t")) {return Some(Report {passed:false,executed:true});}
    let go=text.lines().filter(|line|line.starts_with("ok\t")||line.starts_with("ok  ")||line.starts_with("?\t")).collect::<Vec<_>>();
    if !go.is_empty() {return Some(Report {passed:true,executed:go.iter().any(|line|!line.starts_with('?')&&!line.contains("[no tests to run]")&&!line.contains("[no test files]"))});}
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) const NODE: &str="Server listening on port: 60140\n✔ Course API CRUD operations (724.0725ms)\nℹ tests 1\nℹ suites 0\nℹ pass 1\nℹ fail 0\nℹ cancelled 0\nℹ skipped 0\nℹ todo 0\nℹ duration_ms 1408.9589";
    #[test] fn actual_native_node_report_is_complete_but_partial_or_skipped_output_is_not_a_pass() {
        assert_eq!(complete(NODE),Some(Report {passed:true,executed:true}));
        assert!(complete("# tests 1\n# pass 1").is_none());
        let skipped=NODE.replace("pass 1","pass 0").replace("skipped 0","skipped 1");assert_eq!(complete(&skipped),Some(Report {passed:true,executed:false}));
        assert!(complete(&format!("{NODE}\nCommand is still running")).is_none());
    }
    #[test] fn known_runners_distinguish_real_passes_failures_and_empty_suites() {
        for output in ["test result: ok. 2 passed; 0 failed; 0 ignored", "=== 2 passed in 0.1s ===", "[INFO] Tests run: 2, Failures: 0, Errors: 0, Skipped: 0\n[INFO] BUILD SUCCESS", "Passed! - Failed: 0, Passed: 2, Skipped: 0, Total: 2, Duration: 1 ms", "ok\texample/pkg\t0.2s\n?\texample/empty\t[no test files]"] {assert_eq!(complete(output),Some(Report {passed:true,executed:true}),"{output}");}
        for output in ["test result: FAILED. 0 passed; 1 failed", "=== 1 failed, 2 passed in 0.1s ===", "[INFO] BUILD FAILURE", "Failed! - Failed: 1, Passed: 2, Skipped: 0, Total: 3", "FAIL\texample/pkg"] {assert_eq!(complete(output),Some(Report {passed:false,executed:true}),"{output}");}
        for output in ["test result: ok. 0 passed; 0 failed; 1 ignored", "[INFO] Tests run: 1, Failures: 0, Errors: 0, Skipped: 1\n[INFO] BUILD SUCCESS", "ok\texample/pkg\t0.2s [no tests to run]"] {assert_eq!(complete(output),Some(Report {passed:true,executed:false}),"{output}");}
    }
}
