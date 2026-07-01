use anyhow::{Result, anyhow};
use std::fmt;
use std::str::FromStr;

/// Whitelisted systemctl actions to prevent command injection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
    Reload,
    Enable,
    Disable,
    Status,
}

impl ServiceAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServiceAction::Start => "start",
            ServiceAction::Stop => "stop",
            ServiceAction::Restart => "restart",
            ServiceAction::Reload => "reload",
            ServiceAction::Enable => "enable",
            ServiceAction::Disable => "disable",
            ServiceAction::Status => "status",
        }
    }
}

impl FromStr for ServiceAction {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "start" => Ok(ServiceAction::Start),
            "stop" => Ok(ServiceAction::Stop),
            "restart" => Ok(ServiceAction::Restart),
            "reload" => Ok(ServiceAction::Reload),
            "enable" => Ok(ServiceAction::Enable),
            "disable" => Ok(ServiceAction::Disable),
            "status" => Ok(ServiceAction::Status),
            _ => Err(anyhow!("Invalid service action: {}", s)),
        }
    }
}

impl fmt::Display for ServiceAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_action_roundtrip() {
        for action in &[
            ServiceAction::Start,
            ServiceAction::Stop,
            ServiceAction::Restart,
            ServiceAction::Reload,
            ServiceAction::Enable,
            ServiceAction::Disable,
            ServiceAction::Status,
        ] {
            let s = action.as_str();
            let parsed: ServiceAction = s.parse().unwrap();
            assert_eq!(*action, parsed);
        }
    }

    #[test]
    fn test_service_action_rejects_command_injection() {
        assert!("start; rm -rf /".parse::<ServiceAction>().is_err());
        assert!("".parse::<ServiceAction>().is_err());
    }

    #[test]
    fn test_service_action_display() {
        assert_eq!(ServiceAction::Start.to_string(), "start");
        assert_eq!(ServiceAction::Restart.to_string(), "restart");
    }
}
