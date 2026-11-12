//! POSIX extended regular expressions for `\pdfmatch`, matched by backtracking.

#[derive(Clone, Debug)]
enum Atom {
    Char(char),
    Any,
    Class(Vec<(char, char)>, bool),
    Group(Box<Alt>, usize),
    Start,
    End,
}

#[derive(Clone, Debug)]
struct Piece {
    atom: Atom,
    min: usize,
    max: Option<usize>,
}

#[derive(Clone, Debug)]
struct Alt(Vec<Vec<Piece>>);

/// Why a pattern is invalid, worded as the BSD `regerror` that pdfTeX reports on macOS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PatternError {
    Paren,
    Bracket,
    Brace,
    Repeat,
    Escape,
    Class,
}

impl PatternError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            PatternError::Paren => "parentheses not balanced",
            PatternError::Bracket => "brackets ([ ]) not balanced",
            PatternError::Brace => "braces not balanced",
            PatternError::Repeat => "repetition-operator operand invalid",
            PatternError::Escape => "trailing backslash (\\)",
            PatternError::Class => "invalid character class",
        }
    }
}

struct Parser<'a> {
    chars: &'a [char],
    i: usize,
    groups: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }

    fn alt(&mut self) -> Result<Alt, PatternError> {
        let mut branches = vec![self.seq()?];
        while self.peek() == Some('|') {
            self.i += 1;
            branches.push(self.seq()?);
        }
        Ok(Alt(branches))
    }

    fn seq(&mut self) -> Result<Vec<Piece>, PatternError> {
        let mut pieces = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            let (min, max) = self.repeat()?;
            pieces.push(Piece { atom, min, max });
        }
        Ok(pieces)
    }

    fn number(&mut self) -> Option<usize> {
        let s = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        self.chars[s..self.i].iter().collect::<String>().parse().ok()
    }

    fn repeat(&mut self) -> Result<(usize, Option<usize>), PatternError> {
        let (mut min, mut max) = (1, Some(1));
        loop {
            let (a, b) = match self.peek() {
                Some('*') => (0, None),
                Some('+') => (1, None),
                Some('?') => (0, Some(1)),
                Some('{') => {
                    self.i += 1;
                    let a = self.number().ok_or(PatternError::Brace)?;
                    let b = if self.peek() == Some(',') {
                        self.i += 1;
                        self.number()
                    } else {
                        Some(a)
                    };
                    if self.peek() != Some('}') {
                        return Err(PatternError::Brace);
                    }
                    (a, b)
                }
                _ => return Ok((min, max)),
            };
            self.i += 1;
            min *= a;
            max = match (max, b) {
                (Some(x), Some(y)) => Some(x * y),
                _ => None,
            };
        }
    }

    fn atom(&mut self) -> Result<Atom, PatternError> {
        let c = self.peek().ok_or(PatternError::Paren)?;
        self.i += 1;
        Ok(match c {
            '.' => Atom::Any,
            '^' => Atom::Start,
            '$' => Atom::End,
            '(' => {
                self.groups += 1;
                let n = self.groups;
                let inner = self.alt()?;
                if self.peek() != Some(')') {
                    return Err(PatternError::Paren);
                }
                self.i += 1;
                Atom::Group(Box::new(inner), n)
            }
            '[' => self.class()?,
            '\\' => {
                let e = self.peek().ok_or(PatternError::Escape)?;
                self.i += 1;
                Atom::Char(e)
            }
            '*' | '+' | '?' | '{' => return Err(PatternError::Repeat),
            c => Atom::Char(c),
        })
    }

    fn class(&mut self) -> Result<Atom, PatternError> {
        let negated = self.peek() == Some('^');
        if negated {
            self.i += 1;
        }
        let mut ranges = Vec::new();
        let mut first = true;
        loop {
            let c = self.peek().ok_or(PatternError::Bracket)?;
            self.i += 1;
            if c == ']' && !first {
                break;
            }
            first = false;
            if c == '[' && self.peek() == Some(':') {
                let end = self.chars[self.i..]
                    .iter()
                    .position(|&c| c == ']')
                    .ok_or(PatternError::Bracket)?
                    + self.i;
                let name: String = self.chars[self.i + 1..end - 1].iter().collect();
                self.i = end + 1;
                let class: &[(char, char)] = match name.as_str() {
                    "alpha" => &[('a', 'z'), ('A', 'Z')],
                    "digit" => &[('0', '9')],
                    "alnum" => &[('a', 'z'), ('A', 'Z'), ('0', '9')],
                    "upper" => &[('A', 'Z')],
                    "lower" => &[('a', 'z')],
                    "space" => &[(' ', ' '), ('\t', '\r')],
                    "xdigit" => &[('0', '9'), ('a', 'f'), ('A', 'F')],
                    "punct" => &[('!', '/'), (':', '@'), ('[', '`'), ('{', '~')],
                    _ => return Err(PatternError::Class),
                };
                ranges.extend_from_slice(class);
                continue;
            }
            if self.peek() == Some('-') && self.chars.get(self.i + 1).is_some_and(|&n| n != ']') {
                let hi = self.chars[self.i + 1];
                self.i += 2;
                ranges.push((c, hi));
            } else {
                ranges.push((c, c));
            }
        }
        Ok(Atom::Class(ranges, negated))
    }
}

struct Matcher<'a> {
    text: &'a [char],
    icase: bool,
    caps: Vec<Option<(usize, usize)>>,
}

fn same(a: char, b: char, icase: bool) -> bool {
    a == b || icase && a.to_lowercase().eq(b.to_lowercase())
}

impl Matcher<'_> {
    fn atom_once(&mut self, atom: &Atom, i: usize, k: &mut dyn FnMut(&mut Self, usize) -> bool) -> bool {
        match atom {
            Atom::Char(c) => i < self.text.len() && same(self.text[i], *c, self.icase) && k(self, i + 1),
            Atom::Any => i < self.text.len() && k(self, i + 1),
            Atom::Class(ranges, negated) => {
                if i >= self.text.len() {
                    return false;
                }
                let t = self.text[i];
                let hit = ranges.iter().any(|&(lo, hi)| {
                    (lo..=hi).contains(&t)
                        || self.icase && t.to_lowercase().chain(t.to_uppercase()).any(|u| (lo..=hi).contains(&u))
                });
                hit != *negated && k(self, i + 1)
            }
            Atom::Start => i == 0 && k(self, i),
            Atom::End => i == self.text.len() && k(self, i),
            Atom::Group(alt, n) => {
                let n = *n;
                let alt = alt.clone();
                self.alt(&alt, i, &mut |m: &mut Self, j| {
                    let saved = m.caps[n];
                    m.caps[n] = Some((i, j));
                    if k(m, j) {
                        return true;
                    }
                    m.caps[n] = saved;
                    false
                })
            }
        }
    }

    fn piece(&mut self, p: &Piece, count: usize, i: usize, k: &mut dyn FnMut(&mut Self, usize) -> bool) -> bool {
        if p.max.is_none_or(|m| count < m) {
            let p2 = p.clone();
            let matched = self.atom_once(&p.atom, i, &mut |m: &mut Self, j| {
                (j != i || count < p2.min) && m.piece(&p2, count + 1, j, k)
            });
            if matched {
                return true;
            }
        }
        count >= p.min && k(self, i)
    }

    fn seq(&mut self, pieces: &[Piece], i: usize, k: &mut dyn FnMut(&mut Self, usize) -> bool) -> bool {
        match pieces.split_first() {
            None => k(self, i),
            Some((first, rest)) => {
                let rest = rest.to_vec();
                self.piece(first, 0, i, &mut |m: &mut Self, j| m.seq(&rest, j, k))
            }
        }
    }

    fn alt(&mut self, alt: &Alt, i: usize, k: &mut dyn FnMut(&mut Self, usize) -> bool) -> bool {
        alt.0.iter().any(|branch| self.seq(branch, i, k))
    }
}

/// The captures of a match: byte offset and text of the whole match and of each group.
pub(crate) type Captures = Vec<Option<(usize, String)>>;

/// `matchstrings`: `Ok(1)` with the captures up to `subcount` when `pattern` matches `subject`,
/// `Ok(0)` when not, and the reason when the pattern is invalid.
pub(crate) fn match_strings(
    pattern: &str,
    subject: &str,
    icase: bool,
    subcount: i32,
) -> Result<(i32, Captures), PatternError> {
    let pat: Vec<char> = pattern.chars().collect();
    let mut parser = Parser {
        chars: &pat,
        i: 0,
        groups: 0,
    };
    let alt = parser.alt()?;
    if parser.i != pat.len() {
        return Err(PatternError::Paren);
    }
    let text: Vec<char> = subject.chars().collect();
    let groups = parser.groups;
    let count = if subcount < 0 { groups + 1 } else { subcount as usize };
    for start in 0..=text.len() {
        let mut m = Matcher {
            text: &text,
            icase,
            caps: vec![None; groups + 1],
        };
        let mut end = None;
        if m.alt(&alt, start, &mut |_, j| {
            end = Some(j);
            true
        }) {
            m.caps[0] = Some((start, end.unwrap_or(start)));
            let caps = (0..count)
                .map(|g| {
                    m.caps.get(g).copied().flatten().map(|(s, e)| {
                        let offset: usize = text[..s].iter().map(|c| c.len_utf8()).sum();
                        (offset, text[s..e].iter().collect())
                    })
                })
                .collect();
            return Ok((1, caps));
        }
    }
    Ok((0, Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_with_groups() {
        let (r, caps) = match_strings("b(c+)d", "abcccde", false, -1).unwrap();
        assert_eq!(r, 1);
        assert_eq!(caps[0], Some((1, "bcccd".into())));
        assert_eq!(caps[1], Some((2, "ccc".into())));
        assert_eq!(match_strings("^x", "abc", false, -1).unwrap().0, 0);
        assert_eq!(match_strings("A[[:digit:]]{2}", "xa12", true, -1).unwrap().0, 1);
        assert_eq!(match_strings("a(", "a", false, -1), Err(PatternError::Paren));
        assert_eq!(match_strings("c[", "a", false, -1), Err(PatternError::Bracket));
        assert_eq!(match_strings("(a|b)*c", "ababc", false, -1).unwrap().0, 1);
    }
}
