use std::fmt;

/// Source location in a file
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub col: usize,
}

impl Span {
    pub fn new(start: usize, end: usize, line: usize, col: usize) -> Self {
        Self { start, end, line, col }
    }

    pub fn dummy() -> Self {
        Self { start: 0, end: 0, line: 0, col: 0 }
    }

    pub fn merge(&self, other: &Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            line: self.line,
            col: self.col,
        }
    }
}

/// Identifier with source location
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

impl Ident {
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Self { name: name.into(), span }
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

/// Duration value (e.g., 2ms, 500us, 1s)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Duration {
    pub value: f64,
    pub unit: TimeUnit,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeUnit {
    Milliseconds,
    Microseconds,
    Seconds,
}

impl Duration {
    pub fn to_ms(&self) -> f64 {
        match self.unit {
            TimeUnit::Milliseconds => self.value,
            TimeUnit::Microseconds => self.value / 1000.0,
            TimeUnit::Seconds => self.value * 1000.0,
        }
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.unit {
            TimeUnit::Milliseconds => write!(f, "{}ms", self.value),
            TimeUnit::Microseconds => write!(f, "{}us", self.value),
            TimeUnit::Seconds => write!(f, "{}s", self.value),
        }
    }
}

/// Fabric compiler errors
#[derive(Debug, Clone)]
pub struct Error {
    pub kind: ErrorKind,
    pub span: Span,
    pub message: String,
    pub note: Option<String>,
    pub help: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ErrorKind {
    Syntax,
    Type,
    Fallback,
    Timing,
    CodeGen,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::Syntax => write!(f, "syntax error"),
            ErrorKind::Type => write!(f, "type error"),
            ErrorKind::Fallback => write!(f, "fallback error"),
            ErrorKind::Timing => write!(f, "timing error"),
            ErrorKind::CodeGen => write!(f, "codegen error"),
        }
    }
}

impl Error {
    pub fn new(kind: ErrorKind, span: Span, message: impl Into<String>) -> Self {
        Self {
            kind,
            span,
            message: message.into(),
            note: None,
            help: None,
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

/// Source file representation
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub name: String,
    pub contents: String,
}

impl SourceFile {
    pub fn new(name: impl Into<String>, contents: impl Into<String>) -> Self {
        Self { name: name.into(), contents: contents.into() }
    }

    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let mut line = 1;
        let mut col = 1;
        for (i, ch) in self.contents.char_indices() {
            if i >= offset {
                break;
            }
            if ch == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        (line, col)
    }
}
