use logos::Logos;
use fabric_core::Span;

fn number_callback<'s>(lex: &mut logos::Lexer<'s, Token>) -> f64 {
    lex.slice().parse().unwrap_or(0.0)
}

fn duration_callback<'s>(lex: &mut logos::Lexer<'s, Token>) -> f64 {
    let s = lex.slice();
    let num_part: String = s.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    num_part.parse().unwrap_or(0.0)
}

#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\r\n]+|//[^\n]*")]
pub enum Token {
    // Keywords
    #[token("sensor")]
    Sensor,
    #[token("actuator")]
    Actuator,
    #[token("loop")]
    Loop,
    #[token("within")]
    Within,
    #[token("when")]
    When,
    #[token("unavailable")]
    Unavailable,
    #[token("fallback")]
    Fallback,
    #[token("to")]
    To,
    #[token("let")]
    Let,
    #[token("read")]
    Read,
    #[token("write")]
    Write,
    #[token("from")]
    From,
    #[token("fn")]
    Fn,
    #[token("return")]
    Return,
    #[token("if")]
    If,
    #[token("else")]
    Else,
    #[token("true")]
    True,
    #[token("false")]
    False,
    #[token("for")]
    For,
    #[token("and")]
    AndKw,
    #[token("or")]
    OrKw,
    #[token("not")]
    NotKw,
    #[token("merge")]
    Merge,
    #[token("match")]
    Match,
    #[token("ok")]
    OkKw,
    #[token("timeout")]
    TimeoutKw,
    #[token("error")]
    ErrorKw,
    #[token("probe")]
    Probe,
    #[token("drone")]
    Drone,
    #[token("formation")]
    Formation,
    #[token("count")]
    Count,
    #[token("spacing")]
    Spacing,

    // Identifiers
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*")]
    Ident,

    // Literals
    #[regex(r"[0-9]+\.[0-9]+", callback = number_callback)]
    Float(f64),

    #[regex(r"[0-9]+", callback = number_callback)]
    Integer(f64),

    // Duration: number + time unit (must come after Float/Integer)
    #[regex(r"[0-9]+(\.[0-9]+)?(ms|us|s)", callback = duration_callback)]
    DurationValue(f64),

    // Error bound: ±number or ±number%
    #[regex(r"±[0-9]+(\.[0-9]+)?%?", |lex| {
        let s = lex.slice();
        // Skip the ± character (which is multi-byte) and parse the number
        let after_pm: String = s.chars().skip(1).collect();
        let num_str: String = after_pm.trim_end_matches('%').chars().collect();
        num_str.parse::<f64>().ok()
    })]
    ErrorBound(Option<f64>),

    // Sensor type opener (must come before general < operator)
    #[token("Sensor<")]
    SensorTypeOpen,

    // Two-character operators
    #[token("==")]
    EqEq,
    #[token("!=")]
    Ne,
    #[token("<=")]
    Le,
    #[token(">=")]
    Ge,
    #[token("&&")]
    AmpAmp,
    #[token("||")]
    PipePipe,
    #[token("->")]
    Arrow,
    #[token("=>")]
    FatArrow,

    // Single-character operators
    #[token("=")]
    Equals,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token("!")]
    Bang,

    // Delimiters
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token(",")]
    Comma,
    #[token(":")]
    Colon,
    #[token(";")]
    Semicolon,
    #[token(".")]
    Dot,
}

/// Token with its source span
#[derive(Debug, Clone)]
pub struct SpannedToken<'s> {
    pub token: Token,
    pub span: Span,
    pub text: &'s str,
}

/// Tokenize source code into a list of tokens with spans
pub fn tokenize(source: &str) -> Result<Vec<SpannedToken<'_>>, String> {
    let mut tokens = Vec::new();
    let mut lexer = Token::lexer(source);
    let mut errors = Vec::new();

    while let Some(result) = lexer.next() {
        let span = lexer.span();
        let text = lexer.slice();
        let (line, col) = byte_offset_to_line_col(source, span.start);
        let span_info = Span::new(span.start, span.end, line, col);

        match result {
            Ok(token) => {
                tokens.push(SpannedToken { token, span: span_info, text });
            }
            Err(()) => {
                errors.push(format!(
                    "Unexpected character '{}' at line {}:{}",
                    text.chars().next().unwrap_or('?'),
                    line,
                    col
                ));
            }
        }
    }

    if errors.is_empty() {
        Ok(tokens)
    } else {
        Err(errors.join("\n"))
    }
}

fn byte_offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, ch) in source.char_indices() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_tokens() {
        let source = "sensor imu: IMU";
        let tokens = tokenize(source).unwrap();
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[0].token, Token::Sensor);
        assert_eq!(tokens[0].text, "sensor");
        assert_eq!(tokens[1].token, Token::Ident);
        assert_eq!(tokens[1].text, "imu");
        assert_eq!(tokens[2].token, Token::Colon);
        assert_eq!(tokens[3].token, Token::Ident);
        assert_eq!(tokens[3].text, "IMU");
    }

    #[test]
    fn test_loop_declaration() {
        let source = "loop stabilize() within 2ms { }";
        let tokens = tokenize(source).unwrap();
        assert_eq!(tokens[0].token, Token::Loop);
        assert_eq!(tokens[1].token, Token::Ident);
        assert_eq!(tokens[1].text, "stabilize");
        assert!(tokens.iter().any(|t| t.token == Token::Within));
    }

    #[test]
    fn test_duration_literal() {
        let source = "2ms";
        let tokens = tokenize(source).unwrap();
        assert_eq!(tokens.len(), 1);
        match &tokens[0].token {
            Token::DurationValue(v) => assert_eq!(*v, 2.0),
            other => panic!("expected DurationValue, got {:?}", other),
        }
    }

    #[test]
    fn test_error_bound() {
        let source = "±0.5";
        let tokens = tokenize(source).unwrap();
        assert_eq!(tokens.len(), 1);
        match &tokens[0].token {
            Token::ErrorBound(Some(v)) => assert_eq!(*v, 0.5),
            other => panic!("expected ErrorBound, got {:?}", other),
        }
    }

    #[test]
    fn test_comments_skipped() {
        let source = "// this is a comment\nsensor imu: IMU";
        let tokens = tokenize(source).unwrap();
        assert_eq!(tokens.len(), 4);
    }

    #[test]
    fn test_operators() {
        let source = "a == b != c <= d >= e && f || g";
        let tokens = tokenize(source).unwrap();
        assert!(tokens.iter().any(|t| t.token == Token::EqEq));
        assert!(tokens.iter().any(|t| t.token == Token::Ne));
        assert!(tokens.iter().any(|t| t.token == Token::Le));
        assert!(tokens.iter().any(|t| t.token == Token::Ge));
        assert!(tokens.iter().any(|t| t.token == Token::AmpAmp));
        assert!(tokens.iter().any(|t| t.token == Token::PipePipe));
    }

    #[test]
    fn test_fallback_declaration() {
        let source = "when sensor(altitude) unavailable for 200ms { fallback to estimated }";
        let tokens = tokenize(source).unwrap();
        assert!(tokens.iter().any(|t| t.token == Token::When));
        assert!(tokens.iter().any(|t| t.token == Token::Unavailable));
        assert!(tokens.iter().any(|t| t.token == Token::Fallback));
    }

    #[test]
    fn test_identifier_text() {
        let source = "let my_var = 42";
        let tokens = tokenize(source).unwrap();
        assert_eq!(tokens[0].token, Token::Let);
        assert_eq!(tokens[0].text, "let");
        assert_eq!(tokens[1].token, Token::Ident);
        assert_eq!(tokens[1].text, "my_var");
    }
}
