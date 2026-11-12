//! MongoDB documents as text: mongosh syntax (what the document view shows and edits, and what
//! Claude and `tb` print) and relaxed Extended JSON (what exports write; `mongoimport` reads it).
//!
//! The mongosh text reads back with `mongo_shell::parse_value` to the same values and types
//! (tested below), so a document can be edited as text without losing an ObjectId, a date, a
//! NumberLong or a decimal.

use base64::Engine as _;
use mongodb::bson;
use bson::spec::BinarySubtype;
use bson::{Bson, Document};

/// A document or value in mongosh syntax; `pretty` = indented over several lines.
pub fn shell(v: &Bson, pretty: bool) -> String {
    let mut out = String::new();
    write(v, pretty, 0, &mut out);
    out
}

pub fn shell_doc(d: &Document, pretty: bool) -> String {
    shell(&Bson::Document(d.clone()), pretty)
}

/// Relaxed Extended JSON (`{"$oid": …}`, `{"$date": …}`); compact or indented.
pub fn relaxed_json(d: &Document, pretty: bool) -> String {
    let v = Bson::Document(d.clone()).into_relaxed_extjson();
    if pretty { serde_json::to_string_pretty(&v) } else { serde_json::to_string(&v) }.unwrap_or_default()
}

/// Canonical Extended JSON, one line: every type spelled out (`{"$numberLong": "5"}`), so a
/// dump restores exactly (`mongoimport` reads it).
pub fn canonical_json(d: &Document) -> String {
    serde_json::to_string(&Bson::Document(d.clone()).into_canonical_extjson()).unwrap_or_default()
}

fn key(k: &str) -> String {
    let plain = k.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if plain { k.to_string() } else { serde_json::to_string(k).unwrap_or_default() }
}

fn scalar(v: &Bson) -> bool {
    !matches!(v, Bson::Document(_) | Bson::Array(_))
}

fn write(v: &Bson, pretty: bool, depth: usize, out: &mut String) {
    let pad = |d: usize| "  ".repeat(d);
    match v {
        Bson::Document(d) if d.is_empty() => out.push_str("{}"),
        Bson::Document(d) => {
            if !pretty {
                out.push('{');
                for (i, (k, v)) in d.iter().enumerate() {
                    out.push_str(if i == 0 { " " } else { ", " });
                    out.push_str(&key(k));
                    out.push_str(": ");
                    write(v, false, depth + 1, out);
                }
                out.push_str(" }");
                return;
            }
            out.push_str("{\n");
            let n = d.len();
            for (i, (k, v)) in d.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                out.push_str(&key(k));
                out.push_str(": ");
                write(v, true, depth + 1, out);
                if i + 1 < n {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&pad(depth));
            out.push('}');
        }
        Bson::Array(a) if a.is_empty() => out.push_str("[]"),
        Bson::Array(a) => {
            // Short lists of plain values stay on one line.
            let inline = !pretty || (a.iter().all(scalar) && {
                let mut probe = String::new();
                for v in a {
                    write(v, false, 0, &mut probe);
                }
                probe.len() < 70
            });
            if inline {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    write(v, false, depth + 1, out);
                }
                out.push(']');
                return;
            }
            out.push_str("[\n");
            for (i, v) in a.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                write(v, true, depth + 1, out);
                if i + 1 < a.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&pad(depth));
            out.push(']');
        }
        Bson::String(s) => out.push_str(&serde_json::to_string(s).unwrap_or_default()),
        Bson::Int32(n) => out.push_str(&n.to_string()),
        Bson::Int64(n) => out.push_str(&format!("NumberLong(\"{n}\")")),
        Bson::Double(f) => out.push_str(&double(*f)),
        Bson::Decimal128(d) => out.push_str(&format!("NumberDecimal(\"{d}\")")),
        Bson::Boolean(b) => out.push_str(if *b { "true" } else { "false" }),
        Bson::Null => out.push_str("null"),
        Bson::Undefined => out.push_str("undefined"),
        Bson::ObjectId(o) => out.push_str(&format!("ObjectId(\"{o}\")")),
        Bson::DateTime(d) => match d.try_to_rfc3339_string() {
            Ok(s) => out.push_str(&format!("ISODate(\"{s}\")")),
            // Outside the years 0–9999: milliseconds since 1970.
            Err(_) => out.push_str(&format!("ISODate({})", d.timestamp_millis())),
        },
        Bson::Binary(b) if b.subtype == BinarySubtype::Uuid && b.bytes.len() == 16 => {
            let u = uuid::Uuid::from_slice(&b.bytes).map(|u| u.to_string()).unwrap_or_default();
            out.push_str(&format!("UUID(\"{u}\")"));
        }
        Bson::Binary(b) => {
            let sub: u8 = b.subtype.into();
            out.push_str(&format!("BinData({sub}, \"{}\")", base64::engine::general_purpose::STANDARD.encode(&b.bytes)));
        }
        Bson::Timestamp(t) => out.push_str(&format!("Timestamp({}, {})", t.time, t.increment)),
        // A pattern with "/" (or none) reads back reliably only in the RegExp() form.
        Bson::RegularExpression(r) if r.pattern.contains('/') || r.pattern.is_empty() => out.push_str(&format!(
            "RegExp({}, {})",
            serde_json::to_string(&r.pattern).unwrap_or_default(),
            serde_json::to_string(&r.options).unwrap_or_default()
        )),
        Bson::RegularExpression(r) => out.push_str(&format!("/{}/{}", r.pattern, r.options)),
        Bson::MinKey => out.push_str("MinKey()"),
        Bson::MaxKey => out.push_str("MaxKey()"),
        // Rare types (code, symbol, DBPointer): their Extended JSON form.
        other => out.push_str(&serde_json::to_string(&other.clone().into_relaxed_extjson()).unwrap_or_default()),
    }
}

/// A double that reads back as a double: 5 → "5.0", 1e21 → "1e21", NaN, Infinity.
fn double(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f.is_infinite() {
        if f > 0.0 { "Infinity".into() } else { "-Infinity".into() }
    } else if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{f:.1}")
    } else {
        let s = format!("{f:?}");
        if s.contains(['.', 'e', 'E']) { s } else { format!("{s}.0") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::mongo_shell::parse_value;
    use bson::doc;

    #[test]
    fn shell_text_reads_back_the_same() {
        let d = doc! {
            "_id": bson::oid::ObjectId::parse_str("65a1f0c2e4b0a1b2c3d4e5f6").unwrap(),
            "name": "Zoë \"Z\" O'Neil\nline two",
            "age": 41_i32,
            "big": 9_007_199_254_740_993_i64,
            "small_long": 5_i64,
            "price": 5.0_f64,
            "ratio": 0.1_f64,
            "tiny": 1e-7_f64,
            "huge": 1e300_f64,
            "dec": bson::Decimal128::from_bytes([1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x3c, 0x30]),
            "ok": true,
            "none": null,
            "at": bson::DateTime::from_millis(1_793_620_800_123),
            "old": bson::DateTime::from_millis(-62_198_755_200_000_000),
            "uid": bson::Binary { subtype: BinarySubtype::Uuid, bytes: uuid::Uuid::nil().as_bytes().to_vec() },
            "bin": bson::Binary { subtype: BinarySubtype::Generic, bytes: vec![0, 255, 7] },
            "ts": bson::Timestamp { time: 1_700_000_000, increment: 3 },
            "re": bson::Regex { pattern: "^a/b".into(), options: "i".into() },
            "tags": ["a", "b"],
            "nested": { "deep": { "list": [1, { "x": 2.5 }], "empty": {}, "none": [] } },
            "odd key": 1,
            "$weird": 2,
        };
        for pretty in [true, false] {
            let text = shell_doc(&d, pretty);
            let back = parse_value(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
            assert_eq!(back, Bson::Document(d.clone()), "{text}");
        }
        let text = shell_doc(&d, true);
        assert!(text.contains("  big: NumberLong(\"9007199254740993\"),"), "{text}");
        assert!(text.contains("  price: 5.0,"), "{text}");
        assert!(text.contains("  tags: [\"a\", \"b\"],"), "{text}");
        assert!(text.contains("  \"odd key\": 1,"), "{text}");
    }

    #[test]
    fn relaxed_json_keeps_types_as_extended_json() {
        let d = doc! { "_id": bson::oid::ObjectId::parse_str("65a1f0c2e4b0a1b2c3d4e5f6").unwrap(), "n": 5_i64, "at": bson::DateTime::from_millis(0) };
        let text = relaxed_json(&d, false);
        assert_eq!(text, r#"{"_id":{"$oid":"65a1f0c2e4b0a1b2c3d4e5f6"},"n":5,"at":{"$date":"1970-01-01T00:00:00Z"}}"#);
    }
}
