pub mod lex;

// LALRPOP-generated parser
#[allow(clippy::all)]
#[allow(unused)]
pub mod meerkat {
    include!(concat!(env!("OUT_DIR"), "/runtime/parser/meerkat.rs"));
}

/// Result of attempting to parse a REPL input buffer.
pub enum ReplParseResult {
    /// Input parsed successfully into one or more statements.
    Complete(Vec<crate::ast::Stmt>),
    /// Input is syntactically incomplete (e.g. an open brace with no matching close).
    /// The REPL should prompt for more input and append it to the buffer.
    Incomplete,
    /// Input has a real syntax error that won't be resolved by adding more text.
    Error(String),
}

pub mod parser {
    use logos::Logos;

    use crate::ast::Stmt;

    pub fn parse_string(input: &str) -> Result<Vec<Stmt>, String> {
        let lex_stream = super::lex::Token::lexer(input)
            .spanned()
            .map(|(t, span)| (span.start, t, span.end));

        super::meerkat::ProgParser::new()
            .parse(lex_stream)
            .map_err(|e| format!("Parse error: {:?}", e))
    }

    pub fn parse_file(filename: &str) -> Result<Vec<Stmt>, String> {
        let content = std::fs::read_to_string(filename)
            .map_err(|e| format!("Failed to read file: {}", e))?;
        parse_string(&content)
    }

    /// Try to parse accumulated REPL input, distinguishing incomplete input from real errors.
    ///
    /// Returns `Incomplete` when the grammar signals `UnrecognizedEof`, meaning the user
    /// is mid-statement and the REPL should collect more lines before evaluating.
    pub fn parse_repl(input: &str) -> super::ReplParseResult {
        use super::ReplParseResult;
        use lalrpop_util::ParseError;

        if input.trim().is_empty() {
            return ReplParseResult::Incomplete;
        }

        let lex_stream = super::lex::Token::lexer(input)
            .spanned()
            .map(|(t, span)| (span.start, t, span.end));

        match super::meerkat::ProgParser::new().parse(lex_stream) {
            Ok(stmts) if !stmts.is_empty() => ReplParseResult::Complete(stmts),
            Ok(_) => ReplParseResult::Incomplete,
            Err(ParseError::UnrecognizedEof { .. }) => ReplParseResult::Incomplete,
            Err(e) => ReplParseResult::Error(format!("{:?}", e)),
        }
    }
}
