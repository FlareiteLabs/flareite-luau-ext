//! `end` typed for the user, as Roblox Studio and luau-lsp do: Enter after
//! a line that opens a block (`function f()`, `if x then`, `for ... do`,
//! `while ... do`, `do`) whose `end` is missing puts the cursor on an
//! indented line with `end` below it.
//!
//! The `end` is missing if the document has more blocks opened than ended
//! (counting `function`, `if` statements, and `do` against `end`, past
//! strings and comments, and leaving out `if ... then ... else`
//! expressions), and the block has no body below it yet, so pressing Enter
//! in code that's already there adds nothing.

/// A token, as far as blocks care.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
  /// A keyword or a name.
  Word(String),
  /// A string, a number, or a comment's absence: something that ends an
  /// expression.
  Value,
  /// Punctuation or an operator.
  Punct(String),
}

/// A token and the line it starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Located {
  token: Token,
  line: usize,
}

/// Operators and punctuation, longest first.
const PUNCTUATION: &[&str] = &[
  "...", "//=", "..=", "==", "~=", "<=", ">=", "..", "::", "->", "//", "+=", "-=", "*=", "/=",
  "%=", "^=",
];

/// The tokens of Luau `text`, without comments.
fn tokens(text: &str) -> Vec<Located> {
  let bytes = text.as_bytes();
  let mut found = Vec::new();
  let mut line = 0;
  let mut i = 0;
  // Counts the line breaks in `bytes[from..to]`.
  let breaks = |from: usize, to: usize| bytes[from..to].iter().filter(|&&b| b == b'\n').count();
  while i < bytes.len() {
    let c = bytes[i];
    if c == b'\n' {
      line += 1;
      i += 1;
    } else if c.is_ascii_whitespace() {
      i += 1;
    } else if text[i..].starts_with("--") {
      let end = match long_bracket(bytes, i + 2) {
        Some(level) => close_long_bracket(text, i + 2, level),
        None => text[i..].find('\n').map_or(bytes.len(), |n| i + n),
      };
      line += breaks(i, end);
      i = end;
    } else if c == b'"' || c == b'\'' || c == b'`' {
      let start = i;
      i += 1;
      while i < bytes.len() && bytes[i] != c {
        if bytes[i] == b'\\' {
          i += 1;
        } else if bytes[i] == b'\n' && c != b'`' {
          // Unterminated.
          break;
        }
        i += 1;
      }
      i = (i + 1).min(bytes.len());
      found.push(Located {
        token: Token::Value,
        line,
      });
      line += breaks(start, i);
    } else if let Some(level) = long_bracket(bytes, i) {
      let end = close_long_bracket(text, i, level);
      found.push(Located {
        token: Token::Value,
        line,
      });
      line += breaks(i, end);
      i = end;
    } else if c.is_ascii_alphabetic() || c == b'_' {
      let start = i;
      while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
      }
      found.push(Located {
        token: Token::Word(text[start..i].to_owned()),
        line,
      });
    } else if c.is_ascii_digit() {
      while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'.' | b'_'))
      {
        i += 1;
      }
      found.push(Located {
        token: Token::Value,
        line,
      });
    } else {
      let punct = PUNCTUATION
        .iter()
        .find(|p| text[i..].starts_with(*p))
        .map_or_else(
          || {
            text[i..]
              .chars()
              .next()
              .map(String::from)
              .unwrap_or_default()
          },
          |p| (*p).to_owned(),
        );
      i += punct.len().max(1);
      found.push(Located {
        token: Token::Punct(punct),
        line,
      });
    }
  }
  found
}

/// The level of a long bracket (`[[` is 0, `[==[` is 2) opening at `at`.
fn long_bracket(bytes: &[u8], at: usize) -> Option<usize> {
  if bytes.get(at) != Some(&b'[') {
    return None;
  }
  let mut level = 0;
  while bytes.get(at + 1 + level) == Some(&b'=') {
    level += 1;
  }
  (bytes.get(at + 1 + level) == Some(&b'[')).then_some(level)
}

/// Where the long bracket of `level` opened at `at` closes: past `]=*]`, or
/// the end of the text.
fn close_long_bracket(text: &str, at: usize, level: usize) -> usize {
  let close = format!("]{}]", "=".repeat(level));
  let body = at + level + 2;
  text[body..]
    .find(&close)
    .map_or(text.len(), |found| body + found + close.len())
}

fn word(token: &Token) -> Option<&str> {
  match token {
    Token::Word(word) => Some(word),
    _ => None,
  }
}

/// Whether the `if` after `previous` is an expression (`x = if a then b
/// else c`), which has no `end`.
fn if_expression(previous: Option<&Token>) -> bool {
  match previous {
    None => false,
    Some(Token::Punct(p)) => !matches!(p.as_str(), ")" | "]" | "}" | ";"),
    Some(Token::Word(w)) => matches!(w.as_str(), "return" | "and" | "or" | "not" | "in"),
    Some(Token::Value) => false,
  }
}

/// How much each token opens (+1) or closes (-1) blocks that end in `end`.
fn weights(tokens: &[Located]) -> Vec<i32> {
  tokens
    .iter()
    .enumerate()
    .map(|(index, located)| match word(&located.token) {
      Some("function" | "do") => 1,
      Some("if") if !if_expression(index.checked_sub(1).map(|p| &tokens[p].token)) => 1,
      Some("end") => -1,
      _ => 0,
    })
    .collect()
}

/// The leading spaces and tabs of `line`.
fn indentation(line: &str) -> &str {
  &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// What to insert after Enter in `text`, whose lines are `lines`, with the
/// cursor on line `cursor_line` at byte `cursor_column`: an indented line
/// and `end`, and where in that to leave the cursor. `indent` is one level
/// of indentation. `None` for nothing.
pub fn after_enter(
  lines: &[String],
  cursor_line: usize,
  cursor_column: usize,
  indent: &str,
) -> Option<(String, usize)> {
  let opener_line = cursor_line.checked_sub(1)?;
  let current = lines.get(cursor_line)?;
  let (before, after) = current.split_at(cursor_column.min(current.len()));
  // On a new line: only indentation before the cursor, and only closing
  // brackets after it (`pcall(function()|)`), which end up after `end`.
  if !before.trim().is_empty()
    || !after
      .trim()
      .chars()
      .all(|c| matches!(c, ')' | ']' | '}' | ','))
  {
    return None;
  }
  let text = lines.join("\n");
  let tokens = tokens(&text);
  let weights = weights(&tokens);
  if weights.iter().sum::<i32>() <= 0 {
    return None;
  }
  // The line Enter was pressed on opens a block, and ends where a body
  // follows (not inside a condition or parameters still being written).
  let on_line: Vec<usize> = (0..tokens.len())
    .filter(|&index| tokens[index].line == opener_line)
    .collect();
  let opens: i32 = on_line.iter().map(|&index| weights[index]).sum();
  let last = &tokens[*on_line.last()?].token;
  let has_function = on_line
    .iter()
    .any(|&index| word(&tokens[index].token) == Some("function"));
  let ends_header = match last {
    Token::Word(w) if w == "then" || w == "do" => true,
    // A function's parameters, or its return type.
    Token::Punct(p) => has_function && matches!(p.as_str(), ")" | ">" | "?" | "]" | "}"),
    Token::Word(w) => has_function && !matches!(w.as_str(), "function" | "end"),
    Token::Value => false,
  };
  if opens <= 0 || !ends_header {
    return None;
  }
  // Already a body below: code being edited, not written.
  let opener_indent = indentation(&lines[opener_line]);
  if let Some(next) = lines[cursor_line + 1..]
    .iter()
    .find(|line| !line.trim().is_empty())
    && indentation(next).len() > opener_indent.len()
  {
    return None;
  }
  let inserted = format!("{indent}\n{opener_indent}end");
  Some((inserted, indent.len()))
}

#[cfg(test)]
mod tests {
  use super::*;

  /// What Enter inserts with the cursor at `|`.
  fn enter(marked: &str) -> Option<String> {
    let (line, column) = marked
      .lines()
      .enumerate()
      .find_map(|(line, text)| text.find('|').map(|column| (line, column)))
      .expect("a cursor");
    let lines: Vec<String> = marked.replace('|', "").lines().map(String::from).collect();
    let mut lines = lines;
    if marked.ends_with('\n') || lines.len() <= line {
      lines.push(String::new());
    }
    after_enter(&lines, line, column, "    ").map(|(text, cursor)| {
      assert_eq!(cursor, 4);
      text
    })
  }

  #[test]
  fn ends_new_blocks() {
    let end = Some("    \nend".to_owned());
    assert_eq!(enter("function f()\n|"), end);
    assert_eq!(enter("local function f(a: number): string\n|"), end);
    assert_eq!(enter("function M:method(...)\n|"), end);
    assert_eq!(enter("if x then\n|"), end);
    assert_eq!(enter("for i = 1, 10 do\n|"), end);
    assert_eq!(enter("while true do\n|"), end);
    assert_eq!(enter("do\n|"), end);
    assert_eq!(enter("local f = function()\n|"), end);
    assert_eq!(enter("function f(): Array<number>\n|"), end);
  }

  #[test]
  fn ends_at_the_openers_indentation() {
    assert_eq!(
      enter("function f()\n  if x then\n  |\nend").as_deref(),
      Some("    \n  end")
    );
  }

  #[test]
  fn ends_before_closing_brackets() {
    assert_eq!(enter("pcall(function()\n|)").as_deref(), Some("    \nend"));
    assert_eq!(enter("f(function()\n|x)"), None);
  }

  #[test]
  fn leaves_closed_blocks() {
    assert_eq!(enter("function f()\n|\nend"), None);
    assert_eq!(enter("if x then return end\n|"), None);
    assert_eq!(enter("if a then\n  b()\nelseif c then\n|\nend"), None);
    assert_eq!(enter("if a then\n  b()\nelse\n|\nend"), None);
    assert_eq!(enter("local x = 1\n|"), None);
  }

  #[test]
  fn leaves_lines_that_dont_end_a_header() {
    // A condition being written over several lines.
    assert_eq!(enter("if x and\n|"), None);
    assert_eq!(enter("if x\n|"), None);
    assert_eq!(enter("local f = function(a,\n|"), None);
    // Not at the start of a new line.
    assert_eq!(enter("if x then\n  y|"), None);
  }

  #[test]
  fn leaves_blocks_with_a_body() {
    // Enter at the end of an existing header, with an `end` missing
    // elsewhere.
    assert_eq!(enter("function a()\nif x then\n|\n    y()\n"), None);
  }

  #[test]
  fn skips_strings_comments_and_if_expressions() {
    let end = Some("    \nend".to_owned());
    assert_eq!(enter("local s = \"function end\"\nif x then\n|"), end);
    assert_eq!(enter("-- end end\nif x then\n|"), end);
    assert_eq!(enter("--[[ end\nend ]]\nif x then\n|"), end);
    assert_eq!(enter("local s = [==[ end ]==]\nif x then\n|"), end);
    assert_eq!(enter("local v = if a then 1 else 2\nif x then\n|"), end);
    assert_eq!(enter("local v = if a then 1 else 2\n|"), None);
    assert_eq!(enter("return if a then b else c\n|"), None);
  }
}
