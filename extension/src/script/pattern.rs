//! Lua patterns, matched within a budget.
//!
//! The pattern functions of the Lua library are written in C: they do not
//! pass through the instruction count that stops a script, and a pattern that
//! backtracks a lot could keep a backend busy for hours, with no way to
//! cancel it. Scripts get this matcher instead. It is the algorithm of
//! `lstrlib.c` of Lua 5.4, and gives the same answers; it counts its steps,
//! and stops when the script has none left.

/// The most captures a pattern may have, as in Lua.
const MAX_CAPTURES: usize = 32;
/// How deep the matcher may call itself, as in Lua.
const MAX_DEPTH: usize = 200;
const ESCAPE: u8 = b'%';

/// Why a search did not end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Halt {
    /// The pattern is not a pattern: the message is the one Lua gives.
    Malformed(&'static str),
    /// The budget is spent.
    OutOfSteps,
}

/// One capture of a match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capture {
    /// `()`: a place in the text, counted from zero.
    Position(usize),
    /// `(…)`: a piece of the text, from a place to another.
    Span(usize, usize),
}

/// A match: from `start` to `end`, the end not included, both counted from
/// zero, and what the pattern captured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    pub captures: Vec<Capture>,
}

/// How a search goes through the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// From a place on, as `string.find` does: a pattern that starts with
    /// `^` only matches at that place.
    Find,
    /// The same, for a text to find as it is written: nothing is special.
    Plain,
    /// At one place only; `^` is a character like another.
    At,
    /// From a place on, skipping a match that ends where the previous one
    /// did: how `gmatch` and `gsub` go from a match to the next.
    Next { previous_end: Option<usize> },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Length {
    Unfinished,
    Position,
    Closed(usize),
}

struct Matcher<'a> {
    text: &'a [u8],
    pattern: &'a [u8],
    captures: Vec<(usize, Length)>,
    depth: usize,
    steps: &'a mut i64,
}

type Outcome<T> = Result<T, Halt>;

impl Matcher<'_> {
    fn step(&mut self) -> Outcome<()> {
        *self.steps -= 1;
        if *self.steps < 0 {
            return Err(Halt::OutOfSteps);
        }
        Ok(())
    }

    /// The place after the class that starts at `p`.
    fn class_end(&self, mut p: usize) -> Outcome<usize> {
        let first = self.pattern[p];
        p += 1;
        if first == ESCAPE {
            if p == self.pattern.len() {
                return Err(Halt::Malformed("malformed pattern (ends with '%')"));
            }
            return Ok(p + 1);
        }
        if first == b'[' {
            if self.pattern.get(p) == Some(&b'^') {
                p += 1;
            }
            // Look for the bracket that closes the set.
            loop {
                if p == self.pattern.len() {
                    return Err(Halt::Malformed("malformed pattern (missing ']')"));
                }
                let c = self.pattern[p];
                p += 1;
                if c == ESCAPE && p < self.pattern.len() {
                    p += 1;
                }
                if self.pattern.get(p) == Some(&b']') {
                    return Ok(p + 1);
                }
            }
        }
        Ok(p)
    }

    /// True when a character of the text is in the set between `p`, its
    /// opening bracket, and `end`, its closing one.
    fn in_set(&self, c: u8, mut p: usize, end: usize) -> bool {
        let mut wanted = true;
        if self.pattern.get(p + 1) == Some(&b'^') {
            wanted = false;
            p += 1;
        }
        loop {
            p += 1;
            if p >= end {
                return !wanted;
            }
            if self.pattern[p] == ESCAPE {
                p += 1;
                if p < self.pattern.len() && is_in_class(c, self.pattern[p]) {
                    return wanted;
                }
            } else if self.pattern.get(p + 1) == Some(&b'-') && p + 2 < end {
                p += 2;
                if self.pattern[p - 2] <= c && c <= self.pattern[p] {
                    return wanted;
                }
            } else if self.pattern[p] == c {
                return wanted;
            }
        }
    }

    /// True when the character at `s` is of the class that goes from `p` to
    /// `end`.
    fn single(&mut self, s: usize, p: usize, end: usize) -> Outcome<bool> {
        self.step()?;
        let Some(&c) = self.text.get(s) else { return Ok(false) };
        Ok(match self.pattern[p] {
            b'.' => true,
            ESCAPE => is_in_class(c, self.pattern[p + 1]),
            b'[' => self.in_set(c, p, end - 1),
            literal => literal == c,
        })
    }

    /// `%bxy`: the place after a piece of text that opens with x and closes
    /// with the y that balances it.
    fn balance(&mut self, mut s: usize, p: usize) -> Outcome<Option<usize>> {
        if p + 1 >= self.pattern.len() {
            return Err(Halt::Malformed("malformed pattern (missing arguments to '%b')"));
        }
        let (open, close) = (self.pattern[p], self.pattern[p + 1]);
        if self.text.get(s) != Some(&open) {
            return Ok(None);
        }
        let mut level = 1;
        loop {
            s += 1;
            self.step()?;
            match self.text.get(s) {
                None => return Ok(None),
                Some(&c) if c == close => {
                    level -= 1;
                    if level == 0 {
                        return Ok(Some(s + 1));
                    }
                }
                Some(&c) if c == open => level += 1,
                Some(_) => {}
            }
        }
    }

    /// `*` and `+`: as many as possible, then fewer until the rest matches.
    fn longest(&mut self, s: usize, p: usize, end: usize) -> Outcome<Option<usize>> {
        let mut count = 0;
        while self.single(s + count, p, end)? {
            count += 1;
        }
        loop {
            if let Some(found) = self.from(s + count, end + 1)? {
                return Ok(Some(found));
            }
            if count == 0 {
                return Ok(None);
            }
            count -= 1;
        }
    }

    /// `-`: as few as possible, then more until the rest matches.
    fn shortest(&mut self, mut s: usize, p: usize, end: usize) -> Outcome<Option<usize>> {
        loop {
            if let Some(found) = self.from(s, end + 1)? {
                return Ok(Some(found));
            }
            if !self.single(s, p, end)? {
                return Ok(None);
            }
            s += 1;
        }
    }

    fn open_capture(&mut self, s: usize, p: usize, what: Length) -> Outcome<Option<usize>> {
        if self.captures.len() >= MAX_CAPTURES {
            return Err(Halt::Malformed("too many captures"));
        }
        self.captures.push((s, what));
        let found = self.from(s, p)?;
        if found.is_none() {
            self.captures.pop();
        }
        Ok(found)
    }

    fn close_capture(&mut self, s: usize, p: usize) -> Outcome<Option<usize>> {
        let open = self
            .captures
            .iter()
            .rposition(|(_, length)| *length == Length::Unfinished)
            .ok_or(Halt::Malformed("invalid pattern capture"))?;
        self.captures[open].1 = Length::Closed(s - self.captures[open].0);
        let found = self.from(s, p)?;
        if found.is_none() {
            self.captures[open].1 = Length::Unfinished;
        }
        Ok(found)
    }

    /// `%1` to `%9`: the text of a capture, again.
    fn same_as_capture(&mut self, s: usize, digit: u8) -> Outcome<Option<usize>> {
        let index = usize::from(digit).wrapping_sub(usize::from(b'1'));
        let (start, length) = match self.captures.get(index) {
            Some((start, Length::Closed(length))) => (*start, *length),
            // A place has no text: nothing can be the same as it.
            Some((_, Length::Position)) => return Ok(None),
            _ => return Err(Halt::Malformed("invalid capture index in pattern")),
        };
        self.step()?;
        let again = self.text.get(s..s + length);
        Ok((again == Some(&self.text[start..start + length])).then_some(s + length))
    }

    /// Matches the pattern from `p` on against the text from `s` on. Gives
    /// the place where the match ends.
    fn from(&mut self, s: usize, p: usize) -> Outcome<Option<usize>> {
        if self.depth == 0 {
            return Err(Halt::Malformed("pattern too complex"));
        }
        self.depth -= 1;
        let found = self.walk(s, p);
        self.depth += 1;
        found
    }

    fn walk(&mut self, mut s: usize, mut p: usize) -> Outcome<Option<usize>> {
        loop {
            self.step()?;
            if p == self.pattern.len() {
                return Ok(Some(s));
            }
            match self.pattern[p] {
                b'(' => {
                    return if self.pattern.get(p + 1) == Some(&b')') {
                        self.open_capture(s, p + 2, Length::Position)
                    } else {
                        self.open_capture(s, p + 1, Length::Unfinished)
                    };
                }
                b')' => return self.close_capture(s, p + 1),
                b'$' if p + 1 == self.pattern.len() => {
                    return Ok((s == self.text.len()).then_some(s));
                }
                ESCAPE if self.pattern.get(p + 1) == Some(&b'b') => {
                    match self.balance(s, p + 2)? {
                        Some(after) => {
                            s = after;
                            p += 4;
                        }
                        None => return Ok(None),
                    }
                    continue;
                }
                ESCAPE if self.pattern.get(p + 1) == Some(&b'f') => {
                    p += 2;
                    if self.pattern.get(p) != Some(&b'[') {
                        return Err(Halt::Malformed("missing '[' after '%f' in pattern"));
                    }
                    let end = self.class_end(p)?;
                    let before = if s == 0 { 0 } else { self.text[s - 1] };
                    let here = self.text.get(s).copied().unwrap_or(0);
                    if !self.in_set(before, p, end - 1) && self.in_set(here, p, end - 1) {
                        p = end;
                        continue;
                    }
                    return Ok(None);
                }
                ESCAPE if self.pattern.get(p + 1).is_some_and(u8::is_ascii_digit) => {
                    match self.same_as_capture(s, self.pattern[p + 1])? {
                        Some(after) => {
                            s = after;
                            p += 2;
                        }
                        None => return Ok(None),
                    }
                    continue;
                }
                _ => {}
            }

            // A class, and what may follow it.
            let end = self.class_end(p)?;
            let suffix = self.pattern.get(end).copied();
            if !self.single(s, p, end)? {
                if matches!(suffix, Some(b'*' | b'?' | b'-')) {
                    p = end + 1;
                    continue;
                }
                return Ok(None);
            }
            match suffix {
                Some(b'?') => {
                    if let Some(found) = self.from(s + 1, end + 1)? {
                        return Ok(Some(found));
                    }
                    p = end + 1;
                }
                Some(b'+') => return self.longest(s + 1, p, end),
                Some(b'*') => return self.longest(s, p, end),
                Some(b'-') => return self.shortest(s, p, end),
                _ => {
                    s += 1;
                    p = end;
                }
            }
        }
    }

    fn attempt(&mut self, s: usize) -> Outcome<Option<Found>> {
        self.captures.clear();
        self.depth = MAX_DEPTH;
        let Some(end) = self.from(s, 0)? else { return Ok(None) };
        let captures = self
            .captures
            .iter()
            .map(|(start, length)| match length {
                Length::Position => Ok(Capture::Position(*start)),
                Length::Closed(length) => Ok(Capture::Span(*start, start + length)),
                Length::Unfinished => Err(Halt::Malformed("unfinished capture")),
            })
            .collect::<Outcome<Vec<_>>>()?;
        Ok(Some(Found { start: s, end, captures }))
    }
}

/// True when a character is of a class of Lua: `%a`, `%d`… and their
/// opposites in capitals. Any other character stands for itself.
fn is_in_class(c: u8, class: u8) -> bool {
    let inside = match class.to_ascii_lowercase() {
        b'a' => c.is_ascii_alphabetic(),
        b'c' => c.is_ascii_control(),
        b'd' => c.is_ascii_digit(),
        b'g' => c.is_ascii_graphic(),
        b'l' => c.is_ascii_lowercase(),
        b'p' => c.is_ascii_punctuation(),
        // The white space of C, which has the vertical tabulation too.
        b's' => c.is_ascii_whitespace() || c == 0x0b,
        b'u' => c.is_ascii_uppercase(),
        b'w' => c.is_ascii_alphanumeric(),
        b'x' => c.is_ascii_hexdigit(),
        _ => return class == c,
    };
    inside == class.is_ascii_lowercase()
}

/// Looks for a text in another as it is written, from a place on.
fn plain(text: &[u8], wanted: &[u8], from: usize, steps: &mut i64) -> Outcome<Option<Found>> {
    if wanted.len() > text.len() {
        return Ok(None);
    }
    for start in from..=text.len() - wanted.len() {
        let same = text[start..].iter().zip(wanted).take_while(|(a, b)| a == b).count();
        *steps -= 1 + same as i64;
        if *steps < 0 {
            return Err(Halt::OutOfSteps);
        }
        if same == wanted.len() {
            return Ok(Some(Found { start, end: start + wanted.len(), captures: Vec::new() }));
        }
    }
    Ok(None)
}

/// Looks for a pattern in a text, from the place `from`, counted from zero.
/// Each step of the search takes one from `steps`; the search stops when none
/// are left.
pub fn search(
    text: &[u8],
    pattern: &[u8],
    from: usize,
    mode: Mode,
    steps: &mut i64,
) -> Outcome<Option<Found>> {
    if from > text.len() {
        return Ok(None);
    }
    if mode == Mode::Plain {
        return plain(text, pattern, from, steps);
    }
    let anchored = mode == Mode::Find && pattern.first() == Some(&b'^');
    let pattern = if anchored { &pattern[1..] } else { pattern };
    let mut matcher =
        Matcher { text, pattern, captures: Vec::new(), depth: MAX_DEPTH, steps };
    let mut start = from;
    loop {
        let found = matcher.attempt(start)?;
        match (found, mode) {
            (Some(found), Mode::Next { previous_end }) if Some(found.end) == previous_end => {}
            (Some(found), _) => return Ok(Some(found)),
            (None, _) => {}
        }
        if anchored || mode == Mode::At || start >= text.len() {
            return Ok(None);
        }
        start += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Lua itself answers: the search of its own library, as text.
    fn reference(lua: &mlua::Lua, text: &str, pattern: &str, from: usize) -> String {
        let find: mlua::Function = lua
            .load(
                r#"return function(text, pattern, from)
                    local found = table.pack(pcall(string.find, text, pattern, from))
                    if not found[1] then return "error" end
                    if found[2] == nil then return "nil" end
                    local parts = {}
                    for i = 2, found.n do parts[#parts + 1] = tostring(found[i]) end
                    return table.concat(parts, "|")
                end"#,
            )
            .eval()
            .unwrap();
        find.call((text, pattern, from + 1)).unwrap()
    }

    /// The same answer from this matcher, written the same way.
    fn ours(text: &str, pattern: &str, from: usize) -> String {
        let mut steps = 10_000_000;
        match search(text.as_bytes(), pattern.as_bytes(), from, Mode::Find, &mut steps) {
            Err(_) => "error".to_owned(),
            Ok(None) => "nil".to_owned(),
            Ok(Some(found)) => {
                let mut parts = vec![(found.start + 1).to_string(), found.end.to_string()];
                for capture in found.captures {
                    parts.push(match capture {
                        Capture::Position(at) => (at + 1).to_string(),
                        Capture::Span(start, end) => text[start..end].to_owned(),
                    });
                }
                parts.join("|")
            }
        }
    }

    const TEXTS: [&str; 14] = [
        "",
        "a",
        "hello world",
        "Hello, World! 123",
        "  padded  ",
        "key = value",
        "2026-11-26",
        "f(a(b)c)d) e",
        "aaa",
        "abcabc",
        "THE (quick) fox",
        "x^2 + y$ = 100%",
        "one;two;;four",
        "tab\there\nline",
    ];

    const PATTERNS: [&str; 62] = [
        "",
        "a",
        "l+",
        "l*",
        "l-",
        "o?",
        ".",
        ".-",
        ".*",
        "^h",
        "^",
        "$",
        "d$",
        "^%s*(.-)%s*$",
        "(%w+) = (%w+)",
        "(%d+)-(%d+)-(%d+)",
        "%a+",
        "%A+",
        "%d",
        "%D+",
        "%l+",
        "%u+",
        "%p",
        "%s+",
        "%S+",
        "%w+",
        "%x+",
        "%c",
        "%g+",
        "[aeiou]+",
        "[^aeiou ]+",
        "[a-c]+",
        "[%a_][%w_]*",
        "[%]]",
        "[]]",
        "[^]]+",
        "[a-]",
        "[-a]",
        "()",
        "()a()",
        "(a)(b)(c)",
        "(a*(.)%w(%s*))",
        "(.)%1",
        "(a+)%1",
        "%b()",
        "%bxy",
        "%f[%w]%w+",
        "%f[%a]",
        "%f[%z]",
        "(;)",
        ";(.-);",
        "%%",
        "%^",
        "%$",
        "x^2",
        "a?a?a?aaa",
        "(",
        ")",
        "%",
        "[a",
        "%b",
        "%f",
    ];

    #[test]
    fn the_matcher_answers_as_lua_does() {
        let lua = mlua::Lua::new();
        let mut compared = 0;
        for text in TEXTS {
            for pattern in PATTERNS {
                for from in [0, 1, 3] {
                    if from > text.len() {
                        continue;
                    }
                    assert_eq!(
                        ours(text, pattern, from),
                        reference(&lua, text, pattern, from),
                        "text {text:?}, pattern {pattern:?}, from {from}"
                    );
                    compared += 1;
                }
            }
        }
        assert!(compared > 2000);
    }

    #[test]
    fn a_plain_search_takes_the_text_as_it_is_written() {
        let mut steps = 1000;
        let found = search(b"100% (sure)", b"% (", 0, Mode::Plain, &mut steps).unwrap().unwrap();
        assert_eq!((found.start, found.end), (3, 6));
        assert_eq!(search(b"abc", b"abcd", 0, Mode::Plain, &mut steps), Ok(None));
        assert_eq!(search(b"abc", b"", 3, Mode::Plain, &mut steps).unwrap().unwrap().start, 3);
    }

    #[test]
    fn matches_follow_each_other_without_repeating_an_empty_one() {
        // What gmatch and gsub do with "a*" on "baaac": an empty match before
        // b, then "aaa", then an empty one before c and one at the end.
        let mut steps = 1000;
        let mut at = 0;
        let mut previous_end = None;
        let mut seen = Vec::new();
        while let Some(found) =
            search(b"baaac", b"a*", at, Mode::Next { previous_end }, &mut steps).unwrap()
        {
            seen.push((found.start, found.end));
            previous_end = Some(found.end);
            at = found.end;
        }
        assert_eq!(seen, [(0, 0), (1, 4), (5, 5)]);
        // In that mode and at one place, a circumflex is a character.
        let literal = search(b"x^2", b"^2", 0, Mode::Next { previous_end: None }, &mut steps);
        assert_eq!(literal.unwrap().map(|found| found.start), Some(1));
        assert_eq!(search(b"x^2", b"x", 1, Mode::At, &mut steps), Ok(None));
    }

    #[test]
    fn a_search_that_backtracks_too_much_is_stopped() {
        let text = "a".repeat(2000);
        let mut steps = 1_000_000;
        let halted = search(text.as_bytes(), b".-.-.-.-.-b", 0, Mode::Find, &mut steps);
        assert_eq!(halted, Err(Halt::OutOfSteps));
        assert!(steps < 0);
        // A plain search is counted too.
        let mut steps = 100_000;
        let mut wanted = "a".repeat(1000);
        wanted.push('b');
        let halted = search(text.as_bytes(), wanted.as_bytes(), 0, Mode::Plain, &mut steps);
        assert_eq!(halted, Err(Halt::OutOfSteps));
    }
}
