//! Test proof: which tests a commit adds, removes or rewrites, and which
//! tests a test run reports failed, per project stack.

pub mod rust;

/// The tests a diff adds, removes or rewrites, by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TestChanges {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub rewritten: Vec<String>,
}

/// A stack's way to read tests out of a diff and a test run.
pub trait TestScanner {
    /// The tests a full-context unified diff of `paths()` adds, removes or
    /// rewrites; a test whose body is unchanged in another file is neither.
    fn changes(&self, diff: &str) -> TestChanges;
    /// The tests a test run's output reports failed, as the runner names them.
    fn failed(&self, output: &str) -> Vec<String>;
    /// Program and arguments that run the tests matching `test`.
    fn command(&self, test: &str) -> Vec<String>;
    /// The pathspecs the diff is limited to.
    fn paths(&self) -> &'static [&'static str];
}

/// The Rust scanner for `stack = rust`; `None` on any other stack or none.
#[must_use]
pub fn scanner(stack: Option<&str>) -> Option<Box<dyn TestScanner>> {
    (stack == Some("rust")).then(|| Box::new(rust::RustScanner) as Box<dyn TestScanner>)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ut_scanner_is_rust_only() {
        assert!(scanner(Some("rust")).is_some());
        assert!(scanner(Some("node")).is_none());
        assert!(scanner(None).is_none());
    }
}
