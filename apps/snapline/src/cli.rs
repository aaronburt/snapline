#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetWindow {
    Panel,
    Main,
}

impl BudgetWindow {
    #[must_use]
    pub fn from_argument(value: &str) -> Option<Self> {
        match value {
            "panel" => Some(Self::Panel),
            "main" => Some(Self::Main),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    pub budget_run: bool,
    pub budget_open: Option<BudgetWindow>,
    pub action: Option<String>,
    pub autostart: bool,
    pub uninstall_cleanup: bool,
}

impl Args {
    #[must_use]
    pub fn from_env() -> Self {
        Self::parse(std::env::args().skip(1))
    }

    #[must_use]
    pub fn parse<I, S>(arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let values: Vec<String> = arguments
            .into_iter()
            .map(|value| value.as_ref().to_string())
            .collect();
        let mut args = Self::default();
        let mut index = 0;
        while index < values.len() {
            match values[index].as_str() {
                "--budget-run" => args.budget_run = true,
                "--budget-open" => {
                    index += 1;
                    if let Some(value) = values.get(index) {
                        args.budget_open = BudgetWindow::from_argument(value);
                    }
                }
                "--action" => {
                    index += 1;
                    if let Some(value) = values.get(index) {
                        args.action = Some(value.clone());
                    }
                }
                "--autostart" => args.autostart = true,
                "--uninstall-cleanup" => args.uninstall_cleanup = true,
                _ => {}
            }
            index += 1;
        }
        args
    }

    #[must_use]
    pub fn parse_forwarded(argv: &[String]) -> Self {
        let skip = if argv.first().is_some_and(|first| !first.starts_with('-')) {
            1
        } else {
            0
        };
        Self::parse(argv.iter().skip(skip))
    }
}

#[cfg(test)]
mod tests {
    use super::{Args, BudgetWindow};

    fn parse(arguments: &[&str]) -> Args {
        Args::parse(arguments.iter().copied())
    }

    #[test]
    fn defaults_to_no_flags() {
        assert_eq!(parse(&[]), Args::default());
    }

    #[test]
    fn parses_budget_run() {
        assert!(parse(&["--budget-run"]).budget_run);
    }

    #[test]
    fn parses_budget_open_panel() {
        assert_eq!(
            parse(&["--budget-open", "panel"]).budget_open,
            Some(BudgetWindow::Panel)
        );
    }

    #[test]
    fn parses_budget_open_main() {
        assert_eq!(
            parse(&["--budget-open", "main"]).budget_open,
            Some(BudgetWindow::Main)
        );
    }

    #[test]
    fn ignores_an_unknown_budget_window() {
        let args = parse(&["--budget-open", "settings"]);
        assert_eq!(args.budget_open, None);
    }

    #[test]
    fn ignores_budget_open_without_a_value() {
        assert_eq!(parse(&["--budget-open"]).budget_open, None);
    }

    #[test]
    fn parses_action() {
        let args = parse(&["--action", "capture_region"]);
        assert_eq!(args.action.as_deref(), Some("capture_region"));
    }

    #[test]
    fn ignores_action_without_a_value() {
        assert_eq!(parse(&["--action"]).action, None);
    }

    #[test]
    fn parses_reserved_flags() {
        let args = parse(&["--autostart", "--uninstall-cleanup"]);
        assert!(args.autostart);
        assert!(args.uninstall_cleanup);
    }

    #[test]
    fn ignores_unknown_arguments() {
        let args = parse(&["--colour", "blue", "--budget-run", "extra"]);
        assert!(args.budget_run);
        assert_eq!(args.action, None);
        assert_eq!(args.budget_open, None);
        assert!(!args.autostart);
        assert!(!args.uninstall_cleanup);
    }

    #[test]
    fn parses_flags_in_any_order() {
        let args = parse(&[
            "--action",
            "app_open",
            "--budget-run",
            "--budget-open",
            "main",
        ]);
        assert!(args.budget_run);
        assert_eq!(args.action.as_deref(), Some("app_open"));
        assert_eq!(args.budget_open, Some(BudgetWindow::Main));
    }

    #[test]
    fn forwarded_arguments_skip_the_executable_path() {
        let argv = vec![
            "C:\\Program Files\\Snapline\\snapline.exe".to_string(),
            "--budget-open".to_string(),
            "panel".to_string(),
        ];
        assert_eq!(
            Args::parse_forwarded(&argv).budget_open,
            Some(BudgetWindow::Panel)
        );
    }

    #[test]
    fn forwarded_arguments_without_an_executable_path_still_parse() {
        let argv = vec!["--budget-open".to_string(), "main".to_string()];
        assert_eq!(
            Args::parse_forwarded(&argv).budget_open,
            Some(BudgetWindow::Main)
        );
    }
}
