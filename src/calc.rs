//! An arithmetic expression needs an operator between two operands or a function call: `42`, `2024-10-06` and `file-1` are not one.
//! `^` binds tightest and to the right, under a unary minus (`-2^2` is -4).

pub fn answer(text: &str) -> Option<String> {
    evaluate(text).map(format)
}

pub fn evaluate(text: &str) -> Option<f64> {
    let tokens = tokenize(text)?;
    let mut p = Parser {
        tokens,
        at: 0,
        ops: 0,
    };
    let v = p.expr()?;
    (p.at == p.tokens.len() && p.ops > 0 && v.is_finite()).then_some(v)
}

/// `v` to 12 significant digits, without trailing zeros; huge and tiny
/// values in exponent form.
pub fn format(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let digits = 12 - (v.abs().log10().floor() as i32 + 1);
    let r = if (0..=300).contains(&digits) {
        let m = 10f64.powi(digits);
        (v * m).round() / m
    } else {
        v
    };
    if r == 0.0 {
        return "0".into();
    }
    if r.abs() >= 1e16 || r.abs() < 1e-6 {
        let s = format!("{:.11e}", r);
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let m = m.trim_end_matches('0').trim_end_matches('.');
        format!("{m}e{e}")
    } else {
        format!("{r}")
    }
}

#[derive(Clone, Copy)]
enum Tok {
    Num(f64),
    Op(char),
    Open,
    Close,
    Func(fn(f64) -> f64),
}

fn tokenize(text: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => i += 1,
            '0'..='9' | '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                // "06", "007": a date or a version, not a number.
                let b = s.as_bytes();
                if b.len() > 1 && b[0] == b'0' && b[1].is_ascii_digit() {
                    return None;
                }
                out.push(Tok::Num(s.parse().ok()?));
            }
            '*' if chars.get(i + 1) == Some(&'*') => {
                out.push(Tok::Op('^'));
                i += 2;
            }
            '+' | '-' | '*' | '/' | '%' | '^' => {
                out.push(Tok::Op(c));
                i += 1;
            }
            '×' => {
                out.push(Tok::Op('*'));
                i += 1;
            }
            '÷' => {
                out.push(Tok::Op('/'));
                i += 1;
            }
            '(' => {
                out.push(Tok::Open);
                i += 1;
            }
            ')' => {
                out.push(Tok::Close);
                i += 1;
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect::<String>().to_lowercase();
                out.push(match word.as_str() {
                    "pi" => Tok::Num(std::f64::consts::PI),
                    "e" => Tok::Num(std::f64::consts::E),
                    "sqrt" => Tok::Func(f64::sqrt),
                    "abs" => Tok::Func(f64::abs),
                    "ln" => Tok::Func(f64::ln),
                    "log" => Tok::Func(f64::log10),
                    "sin" => Tok::Func(f64::sin),
                    "cos" => Tok::Func(f64::cos),
                    "tan" => Tok::Func(f64::tan),
                    _ => return None,
                });
            }
            _ => return None,
        }
    }
    Some(out)
}

struct Parser {
    tokens: Vec<Tok>,
    at: usize,
    ops: usize,
}

impl Parser {
    fn peek(&self) -> Option<Tok> {
        self.tokens.get(self.at).copied()
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.peek();
        self.at += 1;
        t
    }

    /// expr := term (('+' | '-') term)*
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(Tok::Op(op @ ('+' | '-'))) = self.peek() {
            self.at += 1;
            self.ops += 1;
            let r = self.term()?;
            v = if op == '+' { v + r } else { v - r };
        }
        Some(v)
    }

    /// term := unary (('*' | '/' | '%') unary)*
    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        while let Some(Tok::Op(op @ ('*' | '/' | '%'))) = self.peek() {
            self.at += 1;
            self.ops += 1;
            let r = self.unary()?;
            v = match op {
                '*' => v * r,
                '/' => v / r,
                _ => v % r,
            };
        }
        Some(v)
    }

    /// unary := ('-' | '+') unary | power
    fn unary(&mut self) -> Option<f64> {
        match self.peek() {
            Some(Tok::Op('-')) => {
                self.at += 1;
                Some(-self.unary()?)
            }
            Some(Tok::Op('+')) => {
                self.at += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    /// power := atom ('^' unary)?
    fn power(&mut self) -> Option<f64> {
        let base = self.atom()?;
        if matches!(self.peek(), Some(Tok::Op('^'))) {
            self.at += 1;
            self.ops += 1;
            return Some(base.powf(self.unary()?));
        }
        Some(base)
    }

    /// atom := number | '(' expr ')' | func '(' expr ')'
    fn atom(&mut self) -> Option<f64> {
        match self.next()? {
            Tok::Num(v) => Some(v),
            Tok::Open => {
                let v = self.expr()?;
                matches!(self.next()?, Tok::Close).then_some(v)
            }
            Tok::Func(f) => {
                if !matches!(self.next()?, Tok::Open) {
                    return None;
                }
                let v = self.expr()?;
                if !matches!(self.next()?, Tok::Close) {
                    return None;
                }
                self.ops += 1;
                Some(f(v))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_associativity_and_unary_minus() {
        assert_eq!(evaluate("2+3*4"), Some(14.0));
        assert_eq!(evaluate("(2+3)*4"), Some(20.0));
        assert_eq!(evaluate("10 - 4 - 3"), Some(3.0), "left to right");
        assert_eq!(evaluate("2^3^2"), Some(512.0), "^ to the right");
        assert_eq!(evaluate("-2^2"), Some(-4.0), "^ under the minus");
        assert_eq!(evaluate("2^-1"), Some(0.5));
        assert_eq!(evaluate("-(3-5)*2"), Some(4.0));
        assert_eq!(evaluate("7 % 3"), Some(1.0));
        assert_eq!(evaluate("1.5*.5"), Some(0.75));
        assert_eq!(evaluate("6 × 7"), Some(42.0));
        assert_eq!(evaluate("9 ÷ 3"), Some(3.0));
        assert_eq!(evaluate("2**10"), Some(1024.0));
    }

    #[test]
    fn functions_and_constants() {
        assert_eq!(evaluate("sqrt(16)"), Some(4.0));
        assert_eq!(evaluate("sqrt(9)+1"), Some(4.0));
        assert_eq!(evaluate("abs(-3)"), Some(3.0));
        assert_eq!(evaluate("log(1000)"), Some(3.0));
        assert_eq!(answer("2*pi"), Some("6.28318530718".into()));
        assert_eq!(evaluate("sqrt 16"), None, "the argument is in parentheses");
    }

    #[test]
    fn a_plain_number_or_a_name_is_no_expression() {
        for text in [
            "",
            "42",
            "-5",
            "(7)",
            "3.14",
            "2024-10-06",
            "1.2.3",
            "file-1",
            "notes.txt",
            "e",
            "pi",
            "2+",
            "(2+3",
            "2+3)",
            "2 3",
            "1,5+2",
            "x*2",
        ] {
            assert_eq!(evaluate(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_value_that_is_not_finite_has_no_answer() {
        assert_eq!(evaluate("1/0"), None);
        assert_eq!(evaluate("sqrt(-1)"), None);
        assert_eq!(evaluate("0/0"), None);
    }

    #[test]
    fn answers_lose_float_noise_and_trailing_zeros() {
        assert_eq!(answer("2+3*4").as_deref(), Some("14"));
        assert_eq!(answer("0.1+0.2").as_deref(), Some("0.3"));
        assert_eq!(answer("1/3").as_deref(), Some("0.333333333333"));
        assert_eq!(answer("-7/2").as_deref(), Some("-3.5"));
        assert_eq!(answer("1-1").as_deref(), Some("0"));
        assert_eq!(answer("2^70").as_deref(), Some("1.18059162072e21"));
        assert_eq!(answer("1/2^30").as_deref(), Some("9.31322574615e-10"));
        assert_eq!(answer("10^15").as_deref(), Some("1000000000000000"));
    }
}
