//! Calculator: the model behind its keys, the button pad and display ([`view`]).
//! Standard evaluates each operator as it is pressed (`2 + 3 × 4 =` is 20); Scientific keeps
//! the expression and evaluates it with precedence through [`crate::calc`] (14).

use crate::calc;
use crate::textedit::keysym;

pub mod view;
pub use view::*;

#[cfg(test)]
mod tests;

/// Typed digits beyond this are dropped: past 15 an f64 no longer holds them all.
pub const MAX_DIGITS: usize = 15;
/// The history keeps this many lines, dropping the oldest.
pub const HISTORY: usize = 100;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Standard,
    Scientific,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Angle {
    #[default]
    Degrees,
    Radians,
    Grads,
}

impl Angle {
    fn to_radians(self, v: f64) -> f64 {
        match self {
            Angle::Degrees => v.to_radians(),
            Angle::Radians => v,
            Angle::Grads => v * std::f64::consts::PI / 200.0,
        }
    }

    /// `v` in quarter turns when it is a whole number of them: the trig of a right angle is
    /// exact rather than off by float noise (tan 90° is undefined, not 1.6e16).
    fn quarter_turns(self, v: f64) -> Option<i64> {
        let quarter = match self {
            Angle::Degrees => 90.0,
            Angle::Grads => 100.0,
            Angle::Radians => return None,
        };
        let q = v / quarter;
        (q.fract() == 0.0 && q.abs() < 1e15).then_some(q as i64)
    }
}

/// A function of the shown value, applied at once in both modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    Sqrt,
    Reciprocal,
    Square,
    Factorial,
    Percent,
    Negate,
    Sin,
    Cos,
    Tan,
    Ln,
    Log,
}

/// One press of a key or a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Digit(u8),
    Point,
    /// `+ - * / ^`.
    Op(char),
    Equals,
    /// C: everything but the memory and the history.
    Clear,
    /// CE: the value being typed.
    ClearEntry,
    Backspace,
    Func(Func),
    Pi,
    E,
    Open,
    Close,
    MemClear,
    MemRecall,
    MemStore,
    MemAdd,
    MemSub,
}

impl Input {
    /// Whether the input works only in Scientific.
    pub fn scientific(self) -> bool {
        matches!(
            self,
            Input::Op('^')
                | Input::Pi
                | Input::E
                | Input::Open
                | Input::Close
                | Input::Func(
                    Func::Square
                        | Func::Factorial
                        | Func::Sin
                        | Func::Cos
                        | Func::Tan
                        | Func::Ln
                        | Func::Log
                )
        )
    }
}

/// The input a key stands for: `sym` and its text `ch`, with Ctrl held or not. Ctrl+L, R, M, P
/// and Q are MC, MR, MS, M+ and M-; Ctrl+C and Ctrl+V are the host's to copy and paste.
pub fn input_for(sym: u32, ch: Option<char>, ctrl: bool) -> Option<Input> {
    if ctrl {
        return match ch.map(|c| c.to_ascii_lowercase()).or_else(|| letter(sym)) {
            Some('l') => Some(Input::MemClear),
            Some('r') => Some(Input::MemRecall),
            Some('m') => Some(Input::MemStore),
            Some('p') => Some(Input::MemAdd),
            Some('q') => Some(Input::MemSub),
            _ => None,
        };
    }
    match sym {
        keysym::RETURN | keysym::KP_ENTER => return Some(Input::Equals),
        keysym::ESCAPE => return Some(Input::Clear),
        keysym::DELETE | keysym::KP_DELETE => return Some(Input::ClearEntry),
        keysym::BACKSPACE => return Some(Input::Backspace),
        F9 => return Some(Input::Func(Func::Negate)),
        _ => {}
    }
    Some(match ch? {
        c @ '0'..='9' => Input::Digit(c as u8 - b'0'),
        '.' | ',' => Input::Point,
        c @ ('+' | '-' | '*' | '/' | '^') => Input::Op(c),
        '×' => Input::Op('*'),
        '÷' => Input::Op('/'),
        'y' => Input::Op('^'),
        '=' => Input::Equals,
        '%' => Input::Func(Func::Percent),
        '@' => Input::Func(Func::Sqrt),
        'r' => Input::Func(Func::Reciprocal),
        'q' => Input::Func(Func::Square),
        '!' => Input::Func(Func::Factorial),
        's' => Input::Func(Func::Sin),
        'o' => Input::Func(Func::Cos),
        't' => Input::Func(Func::Tan),
        'n' => Input::Func(Func::Ln),
        'l' => Input::Func(Func::Log),
        'p' => Input::Pi,
        'e' => Input::E,
        '(' => Input::Open,
        ')' => Input::Close,
        _ => return None,
    })
}

const F9: u32 = 0xffc6;

fn letter(sym: u32) -> Option<char> {
    char::from_u32(sym)
        .filter(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_lowercase())
}

/// Why the display shows a message instead of a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    DivideByZero,
    Invalid,
    Overflow,
}

/// One `=` worked: what was asked, as the expression line showed it, and the answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub expr: String,
    pub result: f64,
}

impl Line {
    /// The row the history list shows.
    pub fn text(&self) -> String {
        format!("{} = {}", self.expr, calc::format(self.result))
    }
}

/// What the display holds: digits being typed, or a value worked out or recalled, which the
/// next digit replaces.
#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Typing(String),
    Value(f64),
}

/// Scientific's expression so far.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Term {
    Num(f64),
    Op(char),
    Open,
    Close,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Calc {
    pub mode: Mode,
    pub angle: Angle,
    entry: Option<Entry>,
    /// The entry was typed, recalled or changed by a function since the last operator: an
    /// operator pressed straight after another replaces it.
    entered: bool,
    /// Standard: the running value and the operator waiting for its right operand.
    acc: Option<f64>,
    pending: Option<char>,
    /// Standard: the last `=`'s operator and right operand, which another `=` repeats.
    repeat: Option<(char, f64)>,
    expr: Vec<Term>,
    /// The last `=`'s expression, shown above its answer until the next input.
    asked: Option<String>,
    pub memory: Option<f64>,
    pub history: Vec<Line>,
    pub error: Option<Error>,
}

/// The symbol the expression line and the buttons show for `op`.
pub fn op_symbol(op: char) -> char {
    match op {
        '-' => '−',
        '*' => '×',
        '/' => '÷',
        o => o,
    }
}

impl Calc {
    pub fn new(mode: Mode) -> Calc {
        Calc {
            mode,
            ..Calc::default()
        }
    }

    /// The value the display stands for.
    pub fn value(&self) -> f64 {
        match &self.entry {
            None => 0.0,
            Some(Entry::Value(v)) => *v,
            Some(Entry::Typing(s)) => match s.as_str() {
                "" | "-" => 0.0,
                s => s.parse().unwrap_or(0.0),
            },
        }
    }

    /// The number the display shows; an error's message is the view's.
    pub fn display(&self) -> String {
        match &self.entry {
            Some(Entry::Typing(s)) if s.is_empty() => "0".into(),
            Some(Entry::Typing(s)) if s == "-" => "-0".into(),
            Some(Entry::Typing(s)) => s.clone(),
            _ => calc::format(self.value()),
        }
    }

    /// The text Copy puts on the clipboard: the number shown, or nothing over an error.
    pub fn copy_text(&self) -> Option<String> {
        self.error.is_none().then(|| self.display())
    }

    /// The line above the number: the operation in progress, or the last `=`'s question.
    pub fn expression(&self) -> String {
        if let Some(asked) = &self.asked {
            return format!("{asked} =");
        }
        match self.mode {
            Mode::Standard => match (self.acc, self.pending) {
                (Some(a), Some(op)) => format!("{} {}", calc::format(a), op_symbol(op)),
                _ => String::new(),
            },
            Mode::Scientific => shown(&self.expr),
        }
    }

    /// Switches the mode; an operation in progress is dropped, the shown value kept.
    pub fn set_mode(&mut self, mode: Mode) {
        if mode == self.mode {
            return;
        }
        let v = self.value();
        let (memory, history, angle) = (self.memory, std::mem::take(&mut self.history), self.angle);
        *self = Calc {
            memory,
            history,
            angle,
            ..Calc::new(mode)
        };
        if self.error.is_none() && v != 0.0 {
            self.entry = Some(Entry::Value(v));
        }
    }

    /// Shows history line `i`'s answer, as if typed.
    pub fn reuse(&mut self, i: usize) {
        if let Some(r) = self.history.get(i).map(|l| l.result) {
            self.error = None;
            self.set_value(r);
        }
    }

    /// Shows the number `text` is, or works out the expression it is in Scientific; false
    /// when it is neither.
    pub fn paste(&mut self, text: &str) -> bool {
        let text: String = text.trim().chars().filter(|c| *c != ' ').collect();
        let v = match self.mode {
            Mode::Standard => text.parse::<f64>().ok().filter(|v| v.is_finite()),
            Mode::Scientific => calc::value(&text),
        };
        let Some(v) = v else {
            return false;
        };
        self.error = None;
        self.set_value(v);
        true
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    /// Feeds one input; false when it changed nothing.
    pub fn input(&mut self, input: Input) -> bool {
        if input.scientific() && self.mode == Mode::Standard {
            return false;
        }
        let before = self.clone();
        if self.error.is_some() {
            match input {
                Input::Clear | Input::ClearEntry | Input::Digit(_) | Input::Point => {
                    self.reset();
                }
                _ => return false,
            }
        }
        self.asked = None;
        match input {
            Input::Digit(d) => self.digit(d),
            Input::Point => self.point(),
            Input::Backspace => {
                if let Some(Entry::Typing(s)) = &mut self.entry {
                    s.pop();
                    if s == "-" {
                        s.clear();
                    }
                }
            }
            Input::Clear => self.reset(),
            Input::ClearEntry => {
                self.entry = Some(Entry::Typing(String::new()));
                self.entered = true;
            }
            Input::Op(op) => match self.mode {
                Mode::Standard => self.standard_op(op),
                Mode::Scientific => self.scientific_op(op),
            },
            Input::Equals => match self.mode {
                Mode::Standard => self.standard_equals(),
                Mode::Scientific => self.scientific_equals(),
            },
            Input::Func(Func::Negate) => match &mut self.entry {
                Some(Entry::Typing(s)) => match s.strip_prefix('-') {
                    Some(rest) => *s = rest.to_string(),
                    None => s.insert(0, '-'),
                },
                _ => {
                    let v = self.value();
                    self.set_value(-v);
                }
            },
            Input::Func(f) => {
                let v = self.value();
                match self.func(f, v) {
                    Ok(r) => self.set_value(r),
                    Err(e) => self.fail(e),
                }
            }
            Input::Pi => self.set_value(std::f64::consts::PI),
            Input::E => self.set_value(std::f64::consts::E),
            Input::Open => {
                // Typed "2(" and ")(" multiply; an answer or a recalled value before ( is replaced.
                if matches!(self.entry, Some(Entry::Typing(_)))
                    || matches!(self.expr.last(), Some(Term::Close))
                {
                    self.push_operand();
                    self.expr.push(Term::Op('*'));
                }
                self.expr.push(Term::Open);
                self.entry = None;
                self.entered = false;
            }
            Input::Close => self.close(),
            Input::MemClear => self.memory = None,
            Input::MemRecall => {
                if let Some(m) = self.memory {
                    self.set_value(m);
                }
            }
            Input::MemStore => self.store(|_, v| v),
            Input::MemAdd => self.store(|m, v| m + v),
            Input::MemSub => self.store(|m, v| m - v),
        }
        *self != before
    }

    fn reset(&mut self) {
        *self = Calc {
            mode: self.mode,
            angle: self.angle,
            memory: self.memory,
            history: std::mem::take(&mut self.history),
            ..Calc::default()
        };
    }

    fn set_value(&mut self, v: f64) {
        self.entry = Some(Entry::Value(v));
        self.entered = true;
    }

    fn fail(&mut self, e: Error) {
        self.reset();
        self.error = Some(e);
    }

    /// The text being typed, a fresh one when the display holds a value.
    fn typing(&mut self) -> &mut String {
        if !matches!(self.entry, Some(Entry::Typing(_))) {
            self.entry = Some(Entry::Typing(String::new()));
        }
        self.entered = true;
        match &mut self.entry {
            Some(Entry::Typing(s)) => s,
            _ => unreachable!("the entry was just made typing"),
        }
    }

    fn digit(&mut self, d: u8) {
        let s = self.typing();
        let digits = s.chars().filter(char::is_ascii_digit).count();
        let lone_zero = s.trim_start_matches('-') == "0";
        if digits >= MAX_DIGITS && !lone_zero {
            return;
        }
        if lone_zero {
            s.pop();
        }
        s.push(char::from(b'0' + d));
    }

    fn point(&mut self) {
        let s = self.typing();
        if s.contains('.') {
            return;
        }
        if s.is_empty() || s == "-" {
            s.push('0');
        }
        s.push('.');
    }

    fn store(&mut self, f: impl Fn(f64, f64) -> f64) {
        let v = self.value();
        self.memory = Some(f(self.memory.unwrap_or(0.0), v));
        self.entry = Some(Entry::Value(v));
    }

    fn func(&self, f: Func, v: f64) -> Result<f64, Error> {
        let r = match f {
            Func::Sqrt if v < 0.0 => return Err(Error::Invalid),
            Func::Sqrt => v.sqrt(),
            Func::Reciprocal if v == 0.0 => return Err(Error::DivideByZero),
            Func::Reciprocal => 1.0 / v,
            Func::Square => v * v,
            Func::Factorial => factorial(v).ok_or(Error::Invalid)?,
            Func::Percent => match (self.mode, self.pending, self.acc) {
                (Mode::Standard, Some('+' | '-'), Some(a)) => a * v / 100.0,
                _ => v / 100.0,
            },
            Func::Negate => -v,
            Func::Sin | Func::Cos | Func::Tan => self.trig(f, v)?,
            Func::Ln | Func::Log if v <= 0.0 => return Err(Error::Invalid),
            Func::Ln => v.ln(),
            Func::Log => v.log10(),
        };
        if r.is_finite() {
            Ok(r)
        } else {
            Err(Error::Overflow)
        }
    }

    fn trig(&self, f: Func, v: f64) -> Result<f64, Error> {
        if let Some(q) = self.angle.quarter_turns(v) {
            // sin, cos and tan at 0°, 90°, 180°, 270°.
            let (sin, cos) =
                [(0.0, 1.0), (1.0, 0.0), (0.0, -1.0), (-1.0, 0.0)][q.rem_euclid(4) as usize];
            return match f {
                Func::Sin => Ok(sin),
                Func::Cos => Ok(cos),
                _ if cos == 0.0 => Err(Error::Invalid),
                _ => Ok(sin / cos),
            };
        }
        let r = self.angle.to_radians(v);
        // sin π is 1.2e-16 in f64: a zero that far below the angle's own precision is noise.
        let noise = |y: f64| {
            if r.abs() >= 1.0 && y.abs() < 1e-15 * r.abs() {
                0.0
            } else {
                y
            }
        };
        Ok(match f {
            Func::Sin => noise(r.sin()),
            Func::Cos => noise(r.cos()),
            _ => noise(r.tan()),
        })
    }

    fn standard_op(&mut self, op: char) {
        let v = self.value();
        match (self.acc, self.pending) {
            (Some(a), Some(p)) if self.entered => match binary(p, a, v) {
                Ok(r) => self.acc = Some(r),
                Err(e) => return self.fail(e),
            },
            (Some(_), Some(_)) => {}
            _ => self.acc = Some(v),
        }
        self.pending = Some(op);
        self.repeat = None;
        self.entry = self.acc.map(Entry::Value);
        self.entered = false;
    }

    fn standard_equals(&mut self) {
        let v = self.value();
        let (a, op, b) = match (self.acc, self.pending, self.repeat) {
            (Some(a), Some(op), _) => (a, op, if self.entered { v } else { a }),
            (_, None, Some((op, b))) => (v, op, b),
            _ => return,
        };
        let asked = format!("{} {} {}", calc::format(a), op_symbol(op), calc::format(b));
        match binary(op, a, b) {
            Ok(r) => {
                self.answer(asked, r);
                self.repeat = Some((op, b));
            }
            Err(e) => self.fail(e),
        }
    }

    /// Ends the expression with the shown value; a closed group is the operand unless a number
    /// was typed after it, which it then multiplies.
    fn push_operand(&mut self) {
        if matches!(self.expr.last(), Some(Term::Close)) {
            if !self.entered {
                return;
            }
            self.expr.push(Term::Op('*'));
        }
        let v = self.value();
        self.expr.push(Term::Num(v));
    }

    fn scientific_op(&mut self, op: char) {
        match self.expr.last() {
            Some(Term::Op(_)) if !self.entered => {
                self.expr.pop();
            }
            _ => self.push_operand(),
        }
        self.expr.push(Term::Op(op));
        self.entered = false;
        if let Some(Entry::Typing(_)) = self.entry {
            let v = self.value();
            self.entry = Some(Entry::Value(v));
        }
    }

    fn depth(&self) -> usize {
        let count = |term| self.expr.iter().filter(|t| **t == term).count();
        count(Term::Open) - count(Term::Close)
    }

    /// Works out the innermost open group and shows its value as the operand it becomes.
    fn close(&mut self) {
        let Some(open) = self.innermost_open() else {
            return;
        };
        self.push_operand();
        self.expr.push(Term::Close);
        match evaluate(&self.expr[open..]) {
            Ok(v) => {
                self.entry = Some(Entry::Value(v));
                self.entered = false;
            }
            Err(e) => self.fail(e),
        }
    }

    fn innermost_open(&self) -> Option<usize> {
        let mut closed = 0;
        for (i, t) in self.expr.iter().enumerate().rev() {
            match t {
                Term::Close => closed += 1,
                Term::Open if closed == 0 => return Some(i),
                Term::Open => closed -= 1,
                _ => {}
            }
        }
        None
    }

    fn scientific_equals(&mut self) {
        if self.expr.is_empty() {
            return;
        }
        self.push_operand();
        for _ in 0..self.depth() {
            self.expr.push(Term::Close);
        }
        let asked = shown(&self.expr);
        match evaluate(&self.expr) {
            Ok(r) => {
                self.expr.clear();
                self.answer(asked, r);
            }
            Err(e) => self.fail(e),
        }
    }

    fn answer(&mut self, asked: String, r: f64) {
        self.history.push(Line {
            expr: asked.clone(),
            result: r,
        });
        if self.history.len() > HISTORY {
            self.history.remove(0);
        }
        self.asked = Some(asked);
        self.acc = None;
        self.pending = None;
        self.set_value(r);
    }
}

fn binary(op: char, a: f64, b: f64) -> Result<f64, Error> {
    if op == '/' && b == 0.0 {
        return Err(Error::DivideByZero);
    }
    let r = calc::apply(op, a, b);
    if r.is_nan() {
        Err(Error::Invalid)
    } else if r.is_finite() {
        Ok(r)
    } else {
        Err(Error::Overflow)
    }
}

/// `n!` for a whole `n` from 0 to 170, past which it is no longer an f64.
fn factorial(n: f64) -> Option<f64> {
    (n.fract() == 0.0 && (0.0..=170.0).contains(&n))
        .then(|| (1..=n as u32).map(f64::from).product())
}

/// The terms as the shared evaluator reads them; a value prints in full, never in exponent
/// form, so it reads back as the same f64.
fn source(expr: &[Term]) -> String {
    let mut s = String::new();
    for t in expr {
        match t {
            Term::Num(v) if *v < 0.0 => s.push_str(&format!("(-{})", -v)),
            Term::Num(v) => s.push_str(&format!("{v}")),
            Term::Op(o) => s.push(*o),
            Term::Open => s.push('('),
            Term::Close => s.push(')'),
        }
    }
    s
}

fn evaluate(expr: &[Term]) -> Result<f64, Error> {
    if let Some(v) = calc::value(&source(expr)) {
        return Ok(v);
    }
    let by_zero = expr
        .windows(2)
        .any(|w| matches!(w, [Term::Op('/'), Term::Num(z)] if *z == 0.0));
    Err(if by_zero {
        Error::DivideByZero
    } else {
        Error::Invalid
    })
}

/// The expression line: numbers as the display shows them, operators as their buttons do.
fn shown(expr: &[Term]) -> String {
    let parts: Vec<String> = expr
        .iter()
        .map(|t| match t {
            Term::Num(v) => calc::format(*v),
            Term::Op(o) => op_symbol(*o).to_string(),
            Term::Open => "(".into(),
            Term::Close => ")".into(),
        })
        .collect();
    parts.join(" ")
}
