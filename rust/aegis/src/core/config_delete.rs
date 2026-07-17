use std::error::Error;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteStage {
    Inspect,
    Prepare,
    Remove,
}

impl fmt::Display for DeleteStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inspect => f.write_str("inspect"),
            Self::Prepare => f.write_str("prepare"),
            Self::Remove => f.write_str("remove"),
        }
    }
}

#[derive(Debug)]
pub struct FileDeleteFailure {
    pub path: PathBuf,
    pub stage: DeleteStage,
    pub source: anyhow::Error,
}

#[derive(Debug)]
pub enum BulkDeleteError {
    Discovery {
        operation: &'static str,
        source: anyhow::Error,
    },
    Incomplete {
        operation: &'static str,
        target: usize,
        deleted: usize,
        failures: Vec<FileDeleteFailure>,
        reload_error: Option<anyhow::Error>,
    },
}

pub type BulkDeleteResult = Result<usize, BulkDeleteError>;

impl BulkDeleteError {
    pub fn discovery(operation: &'static str, source: anyhow::Error) -> Self {
        Self::Discovery { operation, source }
    }

    pub fn deleted(&self) -> usize {
        match self {
            Self::Discovery { .. } => 0,
            Self::Incomplete { deleted, .. } => *deleted,
        }
    }

    pub fn failures(&self) -> &[FileDeleteFailure] {
        match self {
            Self::Discovery { .. } => &[],
            Self::Incomplete { failures, .. } => failures,
        }
    }

    pub fn reload_error(&self) -> Option<&anyhow::Error> {
        match self {
            Self::Discovery { .. } => None,
            Self::Incomplete { reload_error, .. } => reload_error.as_ref(),
        }
    }
}

impl fmt::Display for BulkDeleteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Discovery { operation, source } => {
                write!(f, "{operation} discovery failed: {source}")
            }
            Self::Incomplete {
                operation,
                target,
                deleted,
                failures,
                reload_error,
            } => {
                write!(
                    f,
                    "{operation} incomplete: target={target}, deleted={deleted}, failed={}",
                    failures.len()
                )?;
                for failure in failures {
                    write!(
                        f,
                        "; {} {}: {}",
                        failure.stage,
                        failure.path.display(),
                        failure.source
                    )?;
                }
                if let Some(source) = reload_error {
                    write!(f, "; reload: {source}")?;
                }
                Ok(())
            }
        }
    }
}

impl Error for BulkDeleteError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Discovery { source, .. } => Some(source.as_ref()),
            Self::Incomplete {
                failures,
                reload_error,
                ..
            } => failures
                .first()
                .map(|failure| failure.source.as_ref() as &(dyn Error + 'static))
                .or_else(|| {
                    reload_error
                        .as_ref()
                        .map(|source| source.as_ref() as &(dyn Error + 'static))
                }),
        }
    }
}

#[derive(Debug)]
pub(crate) struct BulkDeleteTracker {
    operation: &'static str,
    target: usize,
    deleted: usize,
    failures: Vec<FileDeleteFailure>,
}

impl BulkDeleteTracker {
    pub(crate) fn new(operation: &'static str, target: usize) -> Self {
        Self {
            operation,
            target,
            deleted: 0,
            failures: Vec::new(),
        }
    }

    pub(crate) fn deleted(&self) -> usize {
        self.deleted
    }

    pub(crate) fn record_deleted(&mut self) {
        self.deleted += 1;
    }

    pub(crate) fn record_failure(
        &mut self,
        path: PathBuf,
        stage: DeleteStage,
        source: impl Into<anyhow::Error>,
    ) {
        self.failures.push(FileDeleteFailure {
            path,
            stage,
            source: source.into(),
        });
    }

    pub(crate) fn finish(self, reload_error: Option<anyhow::Error>) -> BulkDeleteResult {
        if self.failures.is_empty() && reload_error.is_none() {
            return Ok(self.deleted);
        }
        Err(BulkDeleteError::Incomplete {
            operation: self.operation,
            target: self.target,
            deleted: self.deleted,
            failures: self.failures,
            reload_error,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::path::PathBuf;

    use anyhow::anyhow;

    use super::*;

    #[test]
    fn tracker_returns_exact_success_count() {
        let mut tracker = BulkDeleteTracker::new("xray bulk delete", 2);
        tracker.record_deleted();
        tracker.record_deleted();
        assert_eq!(tracker.finish(None).unwrap(), 2);
    }

    #[test]
    fn incomplete_error_retains_file_and_reload_failures() {
        let mut tracker = BulkDeleteTracker::new("xray bulk delete", 3);
        tracker.record_deleted();
        tracker.record_failure(
            PathBuf::from("/tmp/missing.json"),
            DeleteStage::Remove,
            anyhow!("remove failed"),
        );

        let error = tracker.finish(Some(anyhow!("reload failed"))).unwrap_err();
        assert_eq!(error.deleted(), 1);
        assert_eq!(error.failures().len(), 1);
        assert!(error.reload_error().is_some());
        assert!(error.source().is_some());
        let display = error.to_string();
        assert!(display.contains("/tmp/missing.json"));
        assert!(display.contains("remove failed"));
        assert!(display.contains("reload failed"));
    }

    #[test]
    fn discovery_error_preserves_source() {
        let error = BulkDeleteError::discovery("sing-box bulk delete", anyhow!("read dir"));
        assert_eq!(error.deleted(), 0);
        assert!(error.source().is_some());
        assert!(error.to_string().contains("read dir"));
    }
}
