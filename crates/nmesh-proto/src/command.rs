//! NIP-CD slash commands: definitions (kind 31992) and invocation parsing.
//!
//! nmesh uses NIP-CD as-is for invoking agents so that commands work from *any* Nostr client
//! (invocations are plain text). The agent bridge publishes one definition per MCP tool.
//!
//! nmesh extension (documented in `docs/spec/0004-agents.md`): if the argument text begins with
//! `{`, it is parsed as a JSON object of named arguments instead of positionally. This lets
//! tools with many optional parameters be invoked without breaking NIP-CD positional rules.

use nostr::{EventBuilder, PublicKey, Tag, TagKind};
use serde_json::{Map, Number, Value};

use crate::kinds::{self, kind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgType {
    Pubkey,
    Event,
    Address,
    Relay,
    Number,
    Bool,
    Enum,
    Word,
    Text,
}

impl ArgType {
    pub fn as_str(self) -> &'static str {
        match self {
            ArgType::Pubkey => "pubkey",
            ArgType::Event => "event",
            ArgType::Address => "address",
            ArgType::Relay => "relay",
            ArgType::Number => "number",
            ArgType::Bool => "bool",
            ArgType::Enum => "enum",
            ArgType::Word => "word",
            ArgType::Text => "text",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArgSpec {
    pub name: String,
    pub ty: ArgType,
    pub required: bool,
    pub label: String,
    pub choices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SlashCommandDefinition {
    /// Stable `d` tag.
    pub id: String,
    pub command: String,
    pub title: String,
    pub description: String,
    pub args: Vec<ArgSpec>,
    /// NIP-CD scopes, e.g. `kind:14`, `relay:wss://...`.
    pub scopes: Vec<String>,
}

impl SlashCommandDefinition {
    pub fn to_builder(&self) -> EventBuilder {
        let mut tags = vec![
            Tag::identifier(&self.id),
            Tag::custom(TagKind::custom("command"), [self.command.clone()]),
            Tag::custom(TagKind::custom("title"), [self.title.clone()]),
            Tag::custom(TagKind::custom("description"), [self.description.clone()]),
        ];
        for a in &self.args {
            let mut v = vec![
                a.name.clone(),
                a.ty.as_str().to_string(),
                if a.required { "required" } else { "optional" }.to_string(),
                a.label.clone(),
            ];
            v.extend(a.choices.iter().cloned());
            tags.push(Tag::custom(TagKind::custom("arg"), v));
        }
        for s in &self.scopes {
            tags.push(Tag::custom(TagKind::custom("s"), [s.clone()]));
        }
        EventBuilder::new(kind(kinds::SLASH_COMMAND_DEFINITION), "").tags(tags)
    }

    /// Parse a kind 31992 event's tags. Returns `None` if required tags are missing.
    pub fn from_tags(tags: &nostr::Tags) -> Option<Self> {
        let one = |name: &str| {
            tags.find(TagKind::custom(name))
                .and_then(|t| t.content())
                .map(str::to_owned)
        };
        let args = tags
            .filter(TagKind::custom("arg"))
            .filter_map(|t| {
                let s = t.as_slice();
                let ty = match s.get(2)?.as_str() {
                    "pubkey" => ArgType::Pubkey,
                    "event" => ArgType::Event,
                    "address" => ArgType::Address,
                    "relay" => ArgType::Relay,
                    "number" => ArgType::Number,
                    "bool" => ArgType::Bool,
                    "enum" => ArgType::Enum,
                    "text" => ArgType::Text,
                    _ => ArgType::Word,
                };
                Some(ArgSpec {
                    name: s.get(1)?.clone(),
                    ty,
                    required: s.get(3).map(String::as_str) == Some("required"),
                    label: s.get(4).cloned().unwrap_or_default(),
                    choices: s.iter().skip(5).cloned().collect(),
                })
            })
            .collect();
        Some(Self {
            id: tags.identifier()?.to_string(),
            command: one("command")?,
            title: one("title").unwrap_or_default(),
            description: one("description").unwrap_or_default(),
            args,
            scopes: tags
                .filter(TagKind::custom("s"))
                .filter_map(|t| t.content().map(str::to_owned))
                .collect(),
        })
    }
}

/// A parsed `/command[@nostr:npub] args...` invocation.
#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    pub command: String,
    pub target: Option<PublicKey>,
    /// Everything after the command token, with leading whitespace removed.
    pub rest: String,
}

impl Invocation {
    /// Returns true if this invocation is addressed to `me` (or to everyone).
    pub fn is_for(&self, me: &PublicKey) -> bool {
        self.target.is_none_or(|t| &t == me)
    }
}

const BECH32_CHARSET: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

/// Parse a NIP-CD invocation. Returns `None` for ordinary text (which must be treated as text).
pub fn parse_invocation(content: &str) -> Option<Invocation> {
    let body = content.strip_prefix('/')?;
    let token_end = body.find(char::is_whitespace).unwrap_or(body.len());
    let (token, rest) = body.split_at(token_end);
    let (command, target) = match token.split_once('@') {
        Some((cmd, target)) => {
            let t = target.strip_prefix("nostr:").unwrap_or(target);
            let t = t.strip_prefix("npub1")?;
            let len = t.find(|c| !BECH32_CHARSET.contains(c)).unwrap_or(t.len());
            let pk = PublicKey::parse(&format!("npub1{}", &t[..len])).ok()?;
            (cmd, Some(pk))
        }
        None => (token, None),
    };
    if command.is_empty() {
        return None;
    }
    Some(Invocation {
        command: command.to_string(),
        target,
        rest: rest.trim_start().to_string(),
    })
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum BindError {
    #[error("missing required argument `{0}`")]
    Missing(String),
    #[error("argument `{name}`: {reason}")]
    Invalid { name: String, reason: String },
    #[error("invalid JSON arguments: {0}")]
    Json(String),
}

/// Bind invocation text to named arguments using the command's `ArgSpec`s.
pub fn bind_args(specs: &[ArgSpec], rest: &str) -> Result<Map<String, Value>, BindError> {
    let rest = rest.trim();
    if rest.starts_with('{') {
        return match serde_json::from_str::<Value>(rest) {
            Ok(Value::Object(m)) => Ok(m),
            Ok(_) => Err(BindError::Json("expected a JSON object".into())),
            Err(e) => Err(BindError::Json(e.to_string())),
        };
    }

    let mut out = Map::new();
    let mut remaining = rest;
    for spec in specs {
        remaining = remaining.trim_start();
        if remaining.is_empty() {
            if spec.required {
                return Err(BindError::Missing(spec.name.clone()));
            }
            break;
        }
        let raw = if spec.ty == ArgType::Text {
            let all = remaining;
            remaining = "";
            all
        } else {
            let end = remaining
                .find(char::is_whitespace)
                .unwrap_or(remaining.len());
            let (word, tail) = remaining.split_at(end);
            remaining = tail;
            word
        };
        out.insert(spec.name.clone(), convert(spec, raw)?);
    }
    Ok(out)
}

fn convert(spec: &ArgSpec, raw: &str) -> Result<Value, BindError> {
    let invalid = |reason: &str| BindError::Invalid {
        name: spec.name.clone(),
        reason: reason.to_string(),
    };
    Ok(match spec.ty {
        ArgType::Number => {
            let n: f64 = raw.parse().map_err(|_| invalid("expected a number"))?;
            if n.fract() == 0.0 && n.abs() < (i64::MAX as f64) {
                Value::Number((n as i64).into())
            } else {
                Value::Number(Number::from_f64(n).ok_or_else(|| invalid("not finite"))?)
            }
        }
        ArgType::Bool => match raw.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" | "t" | "y" => Value::Bool(true),
            "false" | "no" | "0" | "f" | "n" => Value::Bool(false),
            _ => return Err(invalid("expected a boolean")),
        },
        ArgType::Enum => {
            if !spec.choices.iter().any(|c| c == raw) {
                return Err(invalid(&format!("expected one of {:?}", spec.choices)));
            }
            Value::String(raw.to_string())
        }
        ArgType::Pubkey | ArgType::Event | ArgType::Address => {
            Value::String(raw.strip_prefix("nostr:").unwrap_or(raw).to_string())
        }
        ArgType::Relay | ArgType::Word | ArgType::Text => Value::String(raw.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Keys, ToBech32};
    use serde_json::json;

    fn spec(name: &str, ty: ArgType, required: bool) -> ArgSpec {
        ArgSpec {
            name: name.into(),
            ty,
            required,
            label: name.into(),
            choices: vec![],
        }
    }

    #[test]
    fn plain_text_is_not_a_command() {
        assert_eq!(parse_invocation("hello /not-a-command"), None);
        assert_eq!(parse_invocation("/"), None);
    }

    #[test]
    fn parses_targeted_invocation() {
        let pk = Keys::generate().public_key();
        let npub = pk.to_bech32().unwrap();
        let inv = parse_invocation(&format!("/echo@nostr:{npub} hello world")).unwrap();
        assert_eq!(inv.command, "echo");
        assert_eq!(inv.target, Some(pk));
        assert_eq!(inv.rest, "hello world");
        assert!(inv.is_for(&pk));
        assert!(!inv.is_for(&Keys::generate().public_key()));
    }

    #[test]
    fn binds_positional_with_trailing_text() {
        let specs = [
            spec("n", ArgType::Number, true),
            spec("msg", ArgType::Text, false),
        ];
        let m = bind_args(&specs, "3  hello   world").unwrap();
        assert_eq!(Value::Object(m), json!({"n": 3, "msg": "hello   world"}));
    }

    #[test]
    fn binds_json_object() {
        let m = bind_args(&[], r#"{"a": 1, "b": "x"}"#).unwrap();
        assert_eq!(Value::Object(m), json!({"a": 1, "b": "x"}));
    }

    #[test]
    fn definition_roundtrip() {
        let def = SlashCommandDefinition {
            id: "a:echo".into(),
            command: "echo".into(),
            title: "Echo".into(),
            description: "echo it".into(),
            args: vec![
                spec("n", ArgType::Number, true),
                ArgSpec {
                    name: "u".into(),
                    ty: ArgType::Enum,
                    required: false,
                    label: "units".into(),
                    choices: vec!["c".into(), "f".into()],
                },
            ],
            scopes: vec!["kind:14".into()],
        };
        let ev = def.to_builder().sign_with_keys(&Keys::generate()).unwrap();
        assert_eq!(SlashCommandDefinition::from_tags(&ev.tags), Some(def));
    }

    #[test]
    fn missing_required() {
        let specs = [spec("city", ArgType::Word, true)];
        assert_eq!(
            bind_args(&specs, ""),
            Err(BindError::Missing("city".into()))
        );
    }
}
