use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum DispatchError {
    Internal { source: anyhow::Error },
}

impl fmt::Display for DispatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DispatchError::Internal { source } => {
                write!(f, "dispatch internal error: {source}")
            }
        }
    }
}

impl Error for DispatchError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            DispatchError::Internal { source } => Some(source.as_ref()),
        }
    }
}

impl From<anyhow::Error> for DispatchError {
    fn from(source: anyhow::Error) -> Self {
        DispatchError::Internal { source }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn dispatch_error_from_anyhow_preserves_source() {
        let inner = anyhow!("something went wrong: {}", 42);
        let err = DispatchError::from(inner);
        let display = err.to_string();
        assert!(
            display.contains("42"),
            "display should contain inner details: {display}"
        );
        assert!(err.source().is_some(), "source chain must be preserved");
    }

    #[test]
    fn dispatch_error_display_internal() {
        let err = DispatchError::Internal {
            source: anyhow!("test error"),
        };
        let s = err.to_string();
        assert!(s.contains("test error"));
    }

    #[test]
    fn dispatch_error_into_via_trait() {
        fn accepts_from_anyhow<E: Into<DispatchError>>(_e: E) {}
        accepts_from_anyhow(anyhow!("ok"));
    }
}
