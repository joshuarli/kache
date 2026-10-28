/// Whether this process runs in CI: GitHub Actions, GitLab CI, or any
/// system that sets the common `CI` variable.
pub fn running_in_ci_with(get_env: impl Fn(&str) -> Option<String>) -> bool {
    ["GITHUB_ACTIONS", "GITLAB_CI", "CI"]
        .into_iter()
        .any(|name| env_truthy(get_env(name)))
}

fn env_truthy(value: Option<String>) -> bool {
    value.is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ci_is_github_gitlab_or_the_common_variable() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        assert!(!running_in_ci_with(env(&[])));
        assert!(running_in_ci_with(env(&[("GITHUB_ACTIONS", "true")])));
        assert!(running_in_ci_with(env(&[("GITLAB_CI", "true")])));
        assert!(running_in_ci_with(env(&[("CI", "true")])));
        assert!(running_in_ci_with(env(&[("CI", "1")])));
        assert!(!running_in_ci_with(env(&[("CI", "false")])));
        assert_eq!(crate::config::default_recovery_floor(true), 0);
        assert_eq!(
            crate::config::default_recovery_floor(false),
            crate::config::DEFAULT_RECOVERY_FLOOR
        );
    }
}
