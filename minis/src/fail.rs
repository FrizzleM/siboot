//! One failure type: what the user can change, and the raw cause for
//! `--verbose`.

use std::fmt;

#[derive(Debug)]
pub struct Fail {
    /// What the user can do about it. Always shown.
    pub advice: String,
    /// The underlying protocol or library error. Shown with `--verbose`.
    pub raw: String,
    /// The user stopped the run (Ctrl-C, nothing typed, a cancelled
    /// sign-in): nothing went wrong, so siboot doesn't offer its log.
    pub by_user: bool,
    /// The cause is known and so is the fix (an Open Minis that's too old),
    /// so there's no log to offer either.
    pub known: bool,
}

impl Fail {
    pub fn new(advice: impl Into<String>, raw: impl Into<String>) -> Self {
        Self {
            advice: advice.into(),
            raw: raw.into(),
            by_user: false,
            known: false,
        }
    }

    /// A failure whose cause siboot knows for certain, and says: there's no
    /// underlying error to show.
    pub fn known(advice: impl Into<String>) -> Self {
        Self {
            known: true,
            ..Self::new(advice, "")
        }
    }

    /// The user stopped here.
    pub fn by_user(advice: impl Into<String>) -> Self {
        Self {
            by_user: true,
            ..Self::new(advice, "")
        }
    }
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.advice)
    }
}

pub type Result<T> = std::result::Result<T, Fail>;

/// An error with everything it wraps. idevice's `IdeviceError::Socket` shows
/// only "device socket io failed" and keeps the `io::Error` that says *reset*
/// or *timeout* in `source()`.
pub fn chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        out.push_str(&format!(": {s}"));
        source = s.source();
    }
    out
}
