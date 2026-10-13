//! IPP/1.1 messages (RFC 8010) as cupsd speaks them: a request built from
//! attribute groups and encoded to bytes, and a response parsed back into
//! its status and groups. Only what a print manager reads and writes:
//! integers, booleans, enums, the string types, ranges, resolutions,
//! dates and collections. The HTTP around them is `cups::Conn`'s.

pub const OPERATION: u8 = 0x01;
pub const JOB: u8 = 0x02;
const END: u8 = 0x03;
pub const PRINTER: u8 = 0x04;
pub const SUBSCRIPTION: u8 = 0x06;

/// The out-of-band value that removes an attribute.
pub const DELETE_ATTR: u8 = 0x16;
const INTEGER: u8 = 0x21;
const BOOLEAN: u8 = 0x22;
const ENUM: u8 = 0x23;
const OCTETS: u8 = 0x30;
const DATE: u8 = 0x31;
const RESOLUTION: u8 = 0x32;
const RANGE: u8 = 0x33;
const BEG_COLLECTION: u8 = 0x34;
const TEXT_LANG: u8 = 0x35;
const NAME_LANG: u8 = 0x36;
const END_COLLECTION: u8 = 0x37;
pub const TEXT: u8 = 0x41;
pub const NAME: u8 = 0x42;
pub const KEYWORD: u8 = 0x44;
pub const URI: u8 = 0x45;
const CHARSET: u8 = 0x47;
const LANGUAGE: u8 = 0x48;
pub const MIME: u8 = 0x49;
const MEMBER: u8 = 0x4A;

/// One value of an attribute. The string types keep their tag, so a
/// keyword and a name encode back as what they were.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i32),
    Bool(bool),
    Enum(i32),
    Str(u8, String),
    Range(i32, i32),
    Resolution(i32, i32, i8),
    /// RFC 2579 DateAndTime, the 11 bytes as they came.
    Date(Vec<u8>),
    Collection(Vec<Attr>),
    Octets(Vec<u8>),
    /// no-value, unknown or unsupported: the attribute is there, empty.
    Empty(u8),
}

impl Value {
    pub fn keyword(s: &str) -> Value {
        Value::Str(KEYWORD, s.into())
    }
    pub fn name(s: &str) -> Value {
        Value::Str(NAME, s.into())
    }
    pub fn text(s: &str) -> Value {
        Value::Str(TEXT, s.into())
    }
    pub fn uri(s: &str) -> Value {
        Value::Str(URI, s.into())
    }
    pub fn mime(s: &str) -> Value {
        Value::Str(MIME, s.into())
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(_, s) => Some(s),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i32> {
        match self {
            Value::Int(i) | Value::Enum(i) => Some(*i),
            _ => None,
        }
    }
}

/// A named attribute and its values (a 1setOf has several).
#[derive(Clone, Debug, PartialEq)]
pub struct Attr {
    pub name: String,
    pub values: Vec<Value>,
}

/// A delimited group: the operation attributes, a printer's, a job's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    pub tag: u8,
    pub attrs: Vec<Attr>,
}

impl Group {
    pub fn get(&self, name: &str) -> Option<&[Value]> {
        self.attrs
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.values.as_slice())
    }

    pub fn str(&self, name: &str) -> Option<&str> {
        self.get(name)?.first()?.as_str()
    }

    pub fn string(&self, name: &str) -> String {
        self.str(name).unwrap_or_default().to_string()
    }

    pub fn int(&self, name: &str) -> Option<i32> {
        self.get(name)?.first()?.as_int()
    }

    pub fn bool(&self, name: &str) -> Option<bool> {
        match self.get(name)?.first()? {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn strs(&self, name: &str) -> Vec<String> {
        self.get(name)
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    }
}

/// A request: the operation, its groups in order, and the document that
/// follows the attributes (a test page, a PPD file).
#[derive(Clone, Debug)]
pub struct Request {
    pub op: u16,
    pub groups: Vec<Group>,
    pub data: Vec<u8>,
}

impl Request {
    pub fn new(op: u16) -> Request {
        Request {
            op,
            groups: vec![Group {
                tag: OPERATION,
                attrs: vec![
                    Attr {
                        name: "attributes-charset".into(),
                        values: vec![Value::Str(CHARSET, "utf-8".into())],
                    },
                    Attr {
                        name: "attributes-natural-language".into(),
                        values: vec![Value::Str(LANGUAGE, "en".into())],
                    },
                ],
            }],
            data: Vec::new(),
        }
    }

    /// Add `name` to the group tagged `tag`, opening that group after the
    /// others when it is not there yet.
    pub fn add(mut self, tag: u8, name: &str, values: Vec<Value>) -> Request {
        let attr = Attr {
            name: name.into(),
            values,
        };
        match self.groups.iter_mut().find(|g| g.tag == tag) {
            Some(g) => g.attrs.push(attr),
            None => self.groups.push(Group {
                tag,
                attrs: vec![attr],
            }),
        }
        self
    }

    pub fn op_attr(self, name: &str, value: Value) -> Request {
        self.add(OPERATION, name, vec![value])
    }

    pub fn printer_attr(self, name: &str, value: Value) -> Request {
        self.add(PRINTER, name, vec![value])
    }

    pub fn requested(self, names: &[&str]) -> Request {
        self.add(
            OPERATION,
            "requested-attributes",
            names.iter().map(|n| Value::keyword(n)).collect(),
        )
    }

    pub fn with_data(mut self, data: Vec<u8>) -> Request {
        self.data = data;
        self
    }

    /// The message: version 2.0, request id 1, the groups, the
    /// end-of-attributes tag, then the document.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![2, 0];
        out.extend_from_slice(&self.op.to_be_bytes());
        out.extend_from_slice(&1i32.to_be_bytes());
        for g in &self.groups {
            out.push(g.tag);
            for a in &g.attrs {
                encode_attr(&mut out, a);
            }
        }
        out.push(END);
        out.extend_from_slice(&self.data);
        out
    }
}

fn put_str(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len().min(u16::MAX as usize) as u16).to_be_bytes());
    out.extend_from_slice(&s[..s.len().min(u16::MAX as usize)]);
}

fn encode_attr(out: &mut Vec<u8>, a: &Attr) {
    for (i, v) in a.values.iter().enumerate() {
        let name = if i == 0 { a.name.as_bytes() } else { b"" };
        encode_value(out, name, v);
    }
}

fn encode_value(out: &mut Vec<u8>, name: &[u8], v: &Value) {
    let (tag, body): (u8, Vec<u8>) = match v {
        Value::Int(i) => (INTEGER, i.to_be_bytes().to_vec()),
        Value::Enum(i) => (ENUM, i.to_be_bytes().to_vec()),
        Value::Bool(b) => (BOOLEAN, vec![u8::from(*b)]),
        Value::Str(t, s) => (*t, s.as_bytes().to_vec()),
        Value::Range(a, b) => {
            let mut v = a.to_be_bytes().to_vec();
            v.extend_from_slice(&b.to_be_bytes());
            (RANGE, v)
        }
        Value::Resolution(x, y, u) => {
            let mut v = x.to_be_bytes().to_vec();
            v.extend_from_slice(&y.to_be_bytes());
            v.push(*u as u8);
            (RESOLUTION, v)
        }
        Value::Date(d) => (DATE, d.clone()),
        Value::Octets(d) => (OCTETS, d.clone()),
        Value::Empty(t) => (*t, Vec::new()),
        Value::Collection(members) => {
            out.push(BEG_COLLECTION);
            put_str(out, name);
            put_str(out, b"");
            for m in members {
                for (i, v) in m.values.iter().enumerate() {
                    if i == 0 {
                        out.push(MEMBER);
                        put_str(out, b"");
                        put_str(out, m.name.as_bytes());
                    }
                    encode_value(out, b"", v);
                }
            }
            out.push(END_COLLECTION);
            put_str(out, b"");
            put_str(out, b"");
            return;
        }
    };
    out.push(tag);
    put_str(out, name);
    put_str(out, &body);
}

/// A parsed response: the status code and the groups in order. The
/// document after the attributes (if any) is left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Response {
    pub status: u16,
    pub groups: Vec<Group>,
}

impl Response {
    pub fn ok(&self) -> bool {
        self.status < 0x0100
    }

    pub fn groups(&self, tag: u8) -> impl Iterator<Item = &Group> {
        self.groups.iter().filter(move |g| g.tag == tag)
    }

    pub fn first(&self, tag: u8) -> Option<&Group> {
        self.groups(tag).next()
    }

    /// The operation group's status-message, which cupsd fills with why a
    /// request failed.
    pub fn message(&self) -> Option<&str> {
        self.first(OPERATION)?.str("status-message")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError(pub &'static str);

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ParseError> {
        let end = self.at.checked_add(n).ok_or(ParseError("overflow"))?;
        let s = self.b.get(self.at..end).ok_or(ParseError("truncated"))?;
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, ParseError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ParseError> {
        let s = self.take(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }
    fn chunk(&mut self) -> Result<&'a [u8], ParseError> {
        let n = self.u16()? as usize;
        self.take(n)
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.at).copied()
    }
}

fn i32_of(s: &[u8]) -> Result<i32, ParseError> {
    s.get(..4)
        .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(ParseError("short integer"))
}

fn utf8(s: &[u8]) -> String {
    String::from_utf8_lossy(s).into_owned()
}

/// One value of tag `tag` from its bytes. A collection reads its members
/// from `r`, up to its end tag.
fn decode_value(tag: u8, body: &[u8], r: &mut Reader) -> Result<Value, ParseError> {
    Ok(match tag {
        INTEGER => Value::Int(i32_of(body)?),
        ENUM => Value::Enum(i32_of(body)?),
        BOOLEAN => Value::Bool(body.first().is_some_and(|b| *b != 0)),
        RANGE if body.len() >= 8 => Value::Range(i32_of(body)?, i32_of(&body[4..])?),
        RESOLUTION if body.len() >= 9 => {
            Value::Resolution(i32_of(body)?, i32_of(&body[4..])?, body[8] as i8)
        }
        DATE => Value::Date(body.to_vec()),
        TEXT_LANG | NAME_LANG => {
            // language length, language, text length, text
            let mut inner = Reader { b: body, at: 0 };
            inner.chunk()?;
            let t = if tag == TEXT_LANG { TEXT } else { NAME };
            Value::Str(t, utf8(inner.chunk()?))
        }
        BEG_COLLECTION => Value::Collection(decode_members(r)?),
        0x10..=0x1f => Value::Empty(tag),
        0x40..=0x5f => Value::Str(tag, utf8(body)),
        _ => Value::Octets(body.to_vec()),
    })
}

fn decode_members(r: &mut Reader) -> Result<Vec<Attr>, ParseError> {
    let mut members: Vec<Attr> = Vec::new();
    loop {
        let tag = r.u8()?;
        let _name = r.chunk()?;
        let body = r.chunk()?;
        match tag {
            END_COLLECTION => return Ok(members),
            MEMBER => members.push(Attr {
                name: utf8(body),
                values: Vec::new(),
            }),
            _ => {
                let v = decode_value(tag, body, r)?;
                match members.last_mut() {
                    Some(m) => m.values.push(v),
                    None => return Err(ParseError("collection value before its member name")),
                }
            }
        }
    }
}

pub fn parse(b: &[u8]) -> Result<Response, ParseError> {
    let mut r = Reader { b, at: 0 };
    r.take(2)?; // version
    let status = r.u16()?;
    r.take(4)?; // request id
    let mut groups: Vec<Group> = Vec::new();
    loop {
        let tag = r.u8()?;
        match tag {
            END => break,
            0x00..=0x0f => groups.push(Group {
                tag,
                attrs: Vec::new(),
            }),
            _ => {
                let name = utf8(r.chunk()?);
                let body = r.chunk()?;
                let v = decode_value(tag, body, &mut r)?;
                let g = groups
                    .last_mut()
                    .ok_or(ParseError("attribute before any group"))?;
                if name.is_empty() {
                    // An additional value of the attribute before it.
                    match g.attrs.last_mut() {
                        Some(a) => a.values.push(v),
                        None => return Err(ParseError("additional value with no attribute")),
                    }
                } else {
                    g.attrs.push(Attr {
                        name,
                        values: vec![v],
                    });
                }
            }
        }
        if r.peek().is_none() {
            break;
        }
    }
    Ok(Response { status, groups })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(req: &Request) -> Response {
        parse(&req.encode()).expect("parses")
    }

    #[test]
    fn a_request_encodes_its_header_and_groups() {
        let req =
            Request::new(0x400A).op_attr("printer-uri", Value::uri("ipp://localhost/printers/a"));
        let b = req.encode();
        assert_eq!(&b[..8], &[2, 0, 0x40, 0x0A, 0, 0, 0, 1]);
        assert_eq!(b[8], OPERATION);
        // attributes-charset: tag, name length 18, name, value length 5.
        assert_eq!(b[9], CHARSET);
        assert_eq!(&b[10..12], &[0, 18]);
        assert_eq!(&b[12..30], b"attributes-charset");
        assert_eq!(&b[30..32], &[0, 5]);
        assert_eq!(*b.last().unwrap(), END);
    }

    #[test]
    fn values_and_sets_survive_a_round_trip() {
        let req = Request::new(0x000B)
            .requested(&["printer-name", "printer-state"])
            .printer_attr("printer-is-shared", Value::Bool(true))
            .printer_attr("printer-state", Value::Enum(3))
            .printer_attr("copies-supported", Value::Range(1, 99))
            .printer_attr("job-k-octets", Value::Int(-7))
            .printer_attr("printer-info", Value::text("Front desk"));
        let r = round_trip(&req);
        assert_eq!(r.status, 0x000B);
        let op = r.first(OPERATION).unwrap();
        assert_eq!(
            op.strs("requested-attributes"),
            ["printer-name", "printer-state"]
        );
        let p = r.first(PRINTER).unwrap();
        assert_eq!(p.bool("printer-is-shared"), Some(true));
        assert_eq!(p.int("printer-state"), Some(3));
        assert_eq!(p.get("copies-supported"), Some(&[Value::Range(1, 99)][..]));
        assert_eq!(p.int("job-k-octets"), Some(-7));
        assert_eq!(p.str("printer-info"), Some("Front desk"));
        assert_eq!(p.str("missing"), None);
    }

    #[test]
    fn each_printer_is_a_group_of_its_own() {
        let mut req = Request::new(0);
        for name in ["one", "two"] {
            req.groups.push(Group {
                tag: PRINTER,
                attrs: vec![Attr {
                    name: "printer-name".into(),
                    values: vec![Value::name(name)],
                }],
            });
        }
        let r = round_trip(&req);
        let names: Vec<String> = r
            .groups(PRINTER)
            .map(|g| g.string("printer-name"))
            .collect();
        assert_eq!(names, ["one", "two"]);
    }

    #[test]
    fn collections_nest_and_the_attribute_after_them_still_reads() {
        let col = Value::Collection(vec![
            Attr {
                name: "media-size".into(),
                values: vec![Value::Collection(vec![
                    Attr {
                        name: "x-dimension".into(),
                        values: vec![Value::Int(21000)],
                    },
                    Attr {
                        name: "y-dimension".into(),
                        values: vec![Value::Int(29700)],
                    },
                ])],
            },
            Attr {
                name: "media-source".into(),
                values: vec![Value::keyword("main")],
            },
        ]);
        let req = Request::new(0)
            .printer_attr("media-col-default", col.clone())
            .printer_attr("media-default", Value::keyword("iso_a4_210x297mm"));
        let p = round_trip(&req).first(PRINTER).cloned().unwrap();
        assert_eq!(p.get("media-col-default"), Some(&[col][..]));
        assert_eq!(p.str("media-default"), Some("iso_a4_210x297mm"));
    }

    #[test]
    fn text_with_a_language_reads_as_its_text() {
        // printer-info as textWithLanguage "en" / "Lab".
        let mut b = vec![2, 0, 0, 0, 0, 0, 0, 1, PRINTER, TEXT_LANG];
        b.extend_from_slice(&[0, 12]);
        b.extend_from_slice(b"printer-info");
        b.extend_from_slice(&[0, 9, 0, 2, b'e', b'n', 0, 3, b'L', b'a', b'b']);
        b.push(END);
        let r = parse(&b).unwrap();
        assert!(r.ok());
        assert_eq!(r.first(PRINTER).unwrap().str("printer-info"), Some("Lab"));
    }

    #[test]
    fn a_short_message_is_an_error_not_a_panic() {
        assert!(parse(&[2, 0, 0]).is_err());
        let mut b = Request::new(0).encode();
        b.truncate(20);
        assert!(parse(&b).is_err());
    }
}
