//! The launch contract (plan D6), owned here so its three consumers cannot
//! drift: the CLI prints it as its single stdout line, the PyO3 wheel returns
//! it as a dict, and an agent or CI harness parses it instead of racing a
//! hard-coded port.

use serde::Serialize;

/// What a started server tells its caller.
///
/// Field names are the wire contract — an agent parses them by name — so a
/// rename here is a breaking change to every harness, not a refactor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LaunchInfo {
    /// Fully-qualified URL to open, e.g. `http://127.0.0.1:8731/`.
    pub url: String,
    /// The bound port. Resolved, never the requested `0`.
    pub port: u16,
    /// PID of the process owning the server, so a harness can kill it.
    pub pid: u32,
    /// The graph being served, as the caller named it.
    pub graph: String,
}

impl LaunchInfo {
    /// Render the single line the CLI writes to stdout.
    ///
    /// No trailing newline: the caller decides between `println!` and a
    /// writer, and a line ending baked in here would double up.
    ///
    /// `LaunchInfo` has no field type that can fail to serialize, so the
    /// `expect` is unreachable rather than a swallowed error path.
    pub fn to_json_line(&self) -> String {
        serde_json::to_string(self).expect("LaunchInfo is plain data and always serializes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_line_is_one_line_with_the_contract_keys() {
        let info = LaunchInfo {
            url: "http://127.0.0.1:8731/".to_string(),
            port: 8731,
            pid: 4242,
            graph: "demo.kgl".to_string(),
        };
        let line = info.to_json_line();
        assert!(!line.contains('\n'), "stdout contract is exactly one line");

        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["url"], "http://127.0.0.1:8731/");
        assert_eq!(parsed["port"], 8731);
        assert_eq!(parsed["pid"], 4242);
        assert_eq!(parsed["graph"], "demo.kgl");
        assert_eq!(
            parsed.as_object().unwrap().len(),
            4,
            "an extra key is a contract change, not an addition"
        );
    }
}
