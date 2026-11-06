// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Minimal JSON reader, enough for hyperfine's `--export-json` output.
//! Malformed input gives an error, never a panic.

/// A parsed JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// Any number, as `f64`.
    Num(f64),
    /// A string.
    Str(String),
    /// An array.
    Arr(Vec<Value>),
    /// An object, keeping the order of its members.
    Obj(Vec<(String, Value)>),
}

impl Value {
    /// Member `key` of an object.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The number, if this is one.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    /// The string, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The elements, if this is an array.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }
}

/// A parse error with its byte offset.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("JSON inválido en el byte {offset}: {reason}")]
pub struct JsonError {
    /// Byte offset in the input.
    pub offset: usize,
    /// What was wrong.
    pub reason: &'static str,
}

/// Deepest nesting accepted, to bound recursion.
const MAX_DEPTH: usize = 64;

/// Parses a complete JSON document.
pub fn parse(text: &str) -> Result<Value, JsonError> {
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
    };
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(p.err("datos después del valor"));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, reason: &'static str) -> JsonError {
        JsonError {
            offset: self.i,
            reason,
        }
    }

    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.s.get(self.i) {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &[u8]) -> bool {
        if self.s[self.i..].starts_with(lit) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.err("anidamiento demasiado profundo"));
        }
        self.ws();
        match self.s.get(self.i) {
            None => Err(self.err("fin inesperado")),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Value::Str),
            Some(b't') if self.eat(b"true") => Ok(Value::Bool(true)),
            Some(b'f') if self.eat(b"false") => Ok(Value::Bool(false)),
            Some(b'n') if self.eat(b"null") => Ok(Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("carácter inesperado")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.i += 1;
        let mut members = Vec::new();
        self.ws();
        if self.eat(b"}") {
            return Ok(Value::Obj(members));
        }
        loop {
            self.ws();
            if self.s.get(self.i) != Some(&b'"') {
                return Err(self.err("se esperaba una clave"));
            }
            let key = self.string()?;
            self.ws();
            if !self.eat(b":") {
                return Err(self.err("se esperaba ':'"));
            }
            let v = self.value(depth + 1)?;
            members.push((key, v));
            self.ws();
            if self.eat(b",") {
                continue;
            }
            if self.eat(b"}") {
                return Ok(Value::Obj(members));
            }
            return Err(self.err("se esperaba ',' o '}'"));
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.i += 1;
        let mut items = Vec::new();
        self.ws();
        if self.eat(b"]") {
            return Ok(Value::Arr(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.ws();
            if self.eat(b",") {
                continue;
            }
            if self.eat(b"]") {
                return Ok(Value::Arr(items));
            }
            return Err(self.err("se esperaba ',' o ']'"));
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let digits = self
            .s
            .get(self.i..self.i + 4)
            .ok_or(self.err("escape \\u incompleto"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.err("escape \\u inválido"))?;
        let v = u32::from_str_radix(text, 16).map_err(|_| self.err("escape \\u inválido"))?;
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let start = self.i;
            while let Some(&b) = self.s.get(self.i) {
                if b == b'"' || b == b'\\' || b < 0x20 {
                    break;
                }
                self.i += 1;
            }
            // The input is a &str and we stopped at ASCII bytes, so this
            // slice is valid UTF-8.
            out.push_str(std::str::from_utf8(&self.s[start..self.i]).unwrap_or_default());
            match self.s.get(self.i) {
                None => return Err(self.err("cadena sin cerrar")),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    let Some(&e) = self.s.get(self.i) else {
                        return Err(self.err("escape incompleto"));
                    };
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut c = self.hex4()?;
                            if (0xD800..0xDC00).contains(&c) && self.eat(b"\\u") {
                                let low = self.hex4()?;
                                if (0xDC00..0xE000).contains(&low) {
                                    c = 0x10000 + ((c - 0xD800) << 10) + (low - 0xDC00);
                                }
                            }
                            out.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(self.err("escape desconocido")),
                    }
                }
                Some(_) => return Err(self.err("carácter de control en una cadena")),
            }
        }
    }

    fn number(&mut self) -> Result<Value, JsonError> {
        let start = self.i;
        while let Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') = self.s.get(self.i) {
            self.i += 1;
        }
        std::str::from_utf8(&self.s[start..self.i])
            .ok()
            .and_then(|t| t.parse::<f64>().ok())
            .map(Value::Num)
            .ok_or(JsonError {
                offset: start,
                reason: "número inválido",
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hyperfine_shape() {
        let v = parse(
            r#"{"results":[{"command":"gx \"a\"","median":0.5,"times":[0.4,0.5,6e-1],"exit_codes":[0,0,0],"parameters":null,"ok":true}]}"#,
        )
        .unwrap();
        let r = &v.get("results").unwrap().as_array().unwrap()[0];
        assert_eq!(r.get("command").unwrap().as_str(), Some("gx \"a\""));
        assert_eq!(r.get("median").unwrap().as_f64(), Some(0.5));
        let times: Vec<f64> = r
            .get("times")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_f64)
            .collect();
        assert_eq!(times, [0.4, 0.5, 0.6]);
        assert_eq!(r.get("parameters"), Some(&Value::Null));
    }

    #[test]
    fn unicode_escapes() {
        assert_eq!(parse(r#""añ😀\n""#).unwrap(), Value::Str("añ😀\n".into()));
    }

    #[test]
    fn rejects_malformed_input_without_panicking() {
        for bad in [
            "",
            "{",
            "[1,",
            r#"{"a" 1}"#,
            r#""\u12"#,
            "1 2",
            "tru",
            r#""\q""#,
            "-",
            "[1]]",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(1000) + &"]".repeat(1000);
        assert!(parse(&deep).is_err());
    }
}
