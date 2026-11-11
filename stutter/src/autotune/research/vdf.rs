//! Bounded parser for the quoted Valve Data Format used by Steam manifests.

use std::collections::{BTreeMap, BTreeSet};

const MAX_INPUT_BYTES: usize = 1024 * 1024;
const MAX_TOKENS: usize = 65_536;
const MAX_TOKEN_BYTES: usize = 4096;
const MAX_DEPTH: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VdfValue {
    String(String),
    Object(BTreeMap<String, VdfValue>),
}

impl VdfValue {
    pub fn string(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            Self::Object(_) => None,
        }
    }

    pub fn object(&self) -> Option<&BTreeMap<String, VdfValue>> {
        match self {
            Self::Object(value) => Some(value),
            Self::String(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VdfDocument {
    pub name: String,
    pub value: VdfValue,
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Text(String),
    Open,
    Close,
}

fn tokenize(bytes: &[u8]) -> anyhow::Result<Vec<Token>> {
    anyhow::ensure!(bytes.len() <= MAX_INPUT_BYTES, "VDF exceeds size limit");
    let mut result = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        match bytes[cursor] {
            byte if byte.is_ascii_whitespace() => cursor += 1,
            b'/' if bytes.get(cursor + 1) == Some(&b'/') => {
                cursor += 2;
                while cursor < bytes.len() && bytes[cursor] != b'\n' {
                    cursor += 1;
                }
            }
            b'{' => {
                result.push(Token::Open);
                cursor += 1;
            }
            b'}' => {
                result.push(Token::Close);
                cursor += 1;
            }
            b'"' => {
                cursor += 1;
                let mut value = Vec::new();
                let mut closed = false;
                while cursor < bytes.len() {
                    let byte = bytes[cursor];
                    cursor += 1;
                    if byte == b'"' {
                        closed = true;
                        break;
                    }
                    if byte == b'\\' {
                        let escaped = *bytes
                            .get(cursor)
                            .ok_or_else(|| anyhow::anyhow!("truncated VDF escape"))?;
                        anyhow::ensure!(matches!(escaped, b'\\' | b'"'), "unsupported VDF escape");
                        value.push(escaped);
                        cursor += 1;
                    } else {
                        anyhow::ensure!(byte != 0 && !byte.is_ascii_control(), "invalid VDF text");
                        value.push(byte);
                    }
                    anyhow::ensure!(value.len() <= MAX_TOKEN_BYTES, "VDF token too long");
                }
                anyhow::ensure!(closed, "unterminated VDF string");
                result.push(Token::Text(String::from_utf8(value)?));
            }
            _ => anyhow::bail!("unsupported unquoted VDF token"),
        }
        anyhow::ensure!(result.len() <= MAX_TOKENS, "too many VDF tokens");
    }
    Ok(result)
}

fn parse_object(tokens: &[Token], cursor: &mut usize, depth: usize) -> anyhow::Result<VdfValue> {
    anyhow::ensure!(depth <= MAX_DEPTH, "VDF nesting too deep");
    let mut object = BTreeMap::new();
    loop {
        let key = match tokens.get(*cursor) {
            Some(Token::Close) => {
                *cursor += 1;
                break;
            }
            Some(Token::Text(key)) => {
                *cursor += 1;
                key.to_ascii_lowercase()
            }
            _ => anyhow::bail!("malformed VDF object"),
        };
        anyhow::ensure!(!object.contains_key(&key), "duplicate VDF key");
        let value = match tokens.get(*cursor) {
            Some(Token::Text(value)) => {
                *cursor += 1;
                VdfValue::String(value.clone())
            }
            Some(Token::Open) => {
                *cursor += 1;
                parse_object(tokens, cursor, depth + 1)?
            }
            _ => anyhow::bail!("malformed VDF value"),
        };
        object.insert(key, value);
    }
    Ok(VdfValue::Object(object))
}

impl VdfDocument {
    pub fn parse_all(bytes: &[u8]) -> anyhow::Result<Vec<Self>> {
        let tokens = tokenize(bytes)?;
        let mut roots = Vec::new();
        let mut names = BTreeSet::new();
        let mut cursor = 0;
        while cursor < tokens.len() {
            let name = match tokens.get(cursor) {
                Some(Token::Text(name)) => name.to_ascii_lowercase(),
                _ => anyhow::bail!("missing VDF root name"),
            };
            cursor += 1;
            anyhow::ensure!(
                tokens.get(cursor) == Some(&Token::Open),
                "missing VDF root object"
            );
            anyhow::ensure!(names.insert(name.clone()), "duplicate VDF root name");
            cursor += 1;
            let value = parse_object(&tokens, &mut cursor, 1)?;
            roots.push(Self { name, value });
            anyhow::ensure!(roots.len() <= 64, "too many VDF root objects");
        }
        anyhow::ensure!(!roots.is_empty(), "missing VDF root object");
        Ok(roots)
    }

    pub fn parse(bytes: &[u8]) -> anyhow::Result<Self> {
        let mut roots = Self::parse_all(bytes)?;
        anyhow::ensure!(roots.len() == 1, "expected one VDF root object");
        Ok(roots.remove(0))
    }

    pub fn root_object(&self) -> anyhow::Result<&BTreeMap<String, VdfValue>> {
        self.value
            .object()
            .ok_or_else(|| anyhow::anyhow!("VDF root is not an object"))
    }
}

#[cfg(test)]
#[path = "vdf_tests.rs"]
mod tests;
