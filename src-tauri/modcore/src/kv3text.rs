//! Reading and writing KV3 *text* files (`.vmdl`, `.vmat`, `.vnmskel`...).
//!
//! The tree keeps values as close to the original text as it can: numbers,
//! booleans and other bare words stay as their original spelling, and
//! strings keep their escapes, so an unchanged file writes back the same
//! data. Only what we edit changes.

use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum Kv {
    /// Numbers, `true`, `false`, `null` and other bare words, as written.
    Raw(String),
    /// A quoted string, escapes kept as written. `multiline` for `"""..."""`.
    Str { text: String, multiline: bool },
    /// `resource:"path"`, `resource_name:"..."`, `panorama:"..."`...
    Prefixed { prefix: String, text: String },
    Array(Vec<Kv>),
    Object(Vec<(String, Kv)>),
    /// `#[ 00 11 ... ]`, the bytes kept as written.
    Binary(String),
}

impl Kv {
    pub fn str(s: &str) -> Kv {
        Kv::Str { text: escape(s), multiline: false }
    }
    pub fn resource(s: &str) -> Kv {
        Kv::Prefixed { prefix: "resource".into(), text: escape(s) }
    }
    pub fn get(&self, key: &str) -> Option<&Kv> {
        match self {
            Kv::Object(o) => o.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Kv> {
        match self {
            Kv::Object(o) => o.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    /// Set a key on an object, replacing it or adding it at the end.
    pub fn set(&mut self, key: &str, value: Kv) {
        if let Kv::Object(o) = self {
            if let Some(slot) = o.iter_mut().find(|(k, _)| k == key) {
                slot.1 = value;
            } else {
                o.push((key.to_string(), value));
            }
        }
    }
    /// The unescaped text of a string or prefixed string.
    pub fn as_str(&self) -> Option<String> {
        match self {
            Kv::Str { text, .. } | Kv::Prefixed { text, .. } => Some(unescape(text)),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&Vec<Kv>> {
        match self {
            Kv::Array(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Kv>> {
        match self {
            Kv::Array(a) => Some(a),
            _ => None,
        }
    }
    /// ModelDoc nodes: the `_class` of an object.
    pub fn class(&self) -> Option<String> {
        self.get("_class").and_then(|c| c.as_str())
    }
    pub fn children(&self) -> Option<&Vec<Kv>> {
        self.get("children").and_then(|c| c.as_array())
    }
    pub fn children_mut(&mut self) -> Option<&mut Vec<Kv>> {
        self.get_mut("children").and_then(|c| c.as_array_mut())
    }
    /// Every object in the tree (depth first, this one included).
    pub fn walk<'a>(&'a self, out: &mut Vec<&'a Kv>) {
        match self {
            Kv::Object(o) => {
                out.push(self);
                for (_, v) in o {
                    v.walk(out);
                }
            }
            Kv::Array(a) => {
                for v in a {
                    v.walk(out);
                }
            }
            _ => {}
        }
    }
    /// Apply `f` to every object in the tree, depth first.
    pub fn walk_mut(&mut self, f: &mut dyn FnMut(&mut Kv)) {
        if matches!(self, Kv::Object(_)) {
            f(self);
        }
        match self {
            Kv::Object(o) => {
                for (_, v) in o.iter_mut() {
                    v.walk_mut(f);
                }
            }
            Kv::Array(a) => {
                for v in a.iter_mut() {
                    v.walk_mut(f);
                }
            }
            _ => {}
        }
    }
}

pub fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// A parsed KV3 text file: its `<!-- kv3 ... -->` header line and root.
#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub header: String,
    pub root: Kv,
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn err(&self, what: &str) -> Error {
        let line = self.s[..self.i.min(self.s.len())].iter().filter(|&&b| b == b'\n').count() + 1;
        Error::msg(format!("KV3 text: {what} on line {line}"))
    }
    fn skip(&mut self) {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
                self.i += 1;
            }
            if self.s[self.i..].starts_with(b"//") {
                while self.i < self.s.len() && self.s[self.i] != b'\n' {
                    self.i += 1;
                }
            } else if self.s[self.i..].starts_with(b"/*") {
                match find(&self.s[self.i + 2..], b"*/") {
                    Some(n) => self.i += n + 4,
                    None => self.i = self.s.len(),
                }
            } else {
                return;
            }
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.s.get(self.i).copied()
    }
    fn word(&mut self) -> String {
        let st = self.i;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_alphanumeric() || b"_.-+|".contains(&c) {
                self.i += 1;
            } else {
                break;
            }
        }
        String::from_utf8_lossy(&self.s[st..self.i]).into_owned()
    }
    /// A quoted string starting at `"`. Returns (raw text, multiline).
    fn quoted(&mut self) -> Result<(String, bool)> {
        if self.s[self.i..].starts_with(b"\"\"\"") {
            let st = self.i + 3;
            let n = find(&self.s[st..], b"\"\"\"").ok_or_else(|| self.err("unclosed \"\"\" string"))?;
            self.i = st + n + 3;
            return Ok((String::from_utf8_lossy(&self.s[st..st + n]).into_owned(), true));
        }
        self.i += 1;
        let st = self.i;
        while self.i < self.s.len() {
            match self.s[self.i] {
                b'\\' => self.i += 2,
                b'"' => {
                    let t = String::from_utf8_lossy(&self.s[st..self.i]).into_owned();
                    self.i += 1;
                    return Ok((t, false));
                }
                _ => self.i += 1,
            }
        }
        Err(self.err("unclosed string"))
    }
    fn value(&mut self) -> Result<Kv> {
        match self.peek() {
            Some(b'{') => {
                self.i += 1;
                let mut o = Vec::new();
                loop {
                    match self.peek() {
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Kv::Object(o));
                        }
                        Some(b'"') => {
                            let (k, _) = self.quoted()?;
                            o.push((unescape(&k), self.after_key()?));
                        }
                        Some(_) => {
                            let k = self.word();
                            if k.is_empty() {
                                return Err(self.err("expected a key"));
                            }
                            o.push((k, self.after_key()?));
                        }
                        None => return Err(self.err("unclosed {")),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                loop {
                    match self.peek() {
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Kv::Array(a));
                        }
                        Some(b',') => self.i += 1,
                        Some(_) => a.push(self.value()?),
                        None => return Err(self.err("unclosed [")),
                    }
                }
            }
            Some(b'"') => {
                let (text, multiline) = self.quoted()?;
                Ok(Kv::Str { text, multiline })
            }
            Some(b'#') if self.s.get(self.i + 1) == Some(&b'[') => {
                let st = self.i + 2;
                let n = find(&self.s[st..], b"]").ok_or_else(|| self.err("unclosed #["))?;
                self.i = st + n + 1;
                Ok(Kv::Binary(String::from_utf8_lossy(&self.s[st..st + n]).trim().to_string()))
            }
            Some(_) => {
                let w = self.word();
                if w.is_empty() {
                    return Err(self.err("unexpected character"));
                }
                if self.s.get(self.i) == Some(&b':') && self.s.get(self.i + 1) == Some(&b'"') {
                    self.i += 1;
                    let (text, _) = self.quoted()?;
                    return Ok(Kv::Prefixed { prefix: w, text });
                }
                Ok(Kv::Raw(w))
            }
            None => Err(self.err("unexpected end of file")),
        }
    }
    fn after_key(&mut self) -> Result<Kv> {
        if self.peek() != Some(b'=') {
            return Err(self.err("expected ="));
        }
        self.i += 1;
        self.value()
    }
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

pub fn parse(text: &str) -> Result<Doc> {
    let t = text.trim_start_matches('\u{feff}');
    let (header, body) = match t.find("-->") {
        Some(n) if t.trim_start().starts_with("<!--") => (t[..n + 3].trim().to_string(), &t[n + 3..]),
        _ => (String::new(), t),
    };
    let mut p = P { s: body.as_bytes(), i: 0 };
    let root = p.value()?;
    if p.peek().is_some() {
        return Err(p.err("text after the end of the file"));
    }
    Ok(Doc { header, root })
}

fn write(v: &Kv, depth: usize, out: &mut String) {
    let tabs = |n: usize| "\t".repeat(n);
    match v {
        Kv::Raw(r) => out.push_str(r),
        Kv::Str { text, multiline: false } => {
            out.push('"');
            out.push_str(text);
            out.push('"');
        }
        Kv::Str { text, multiline: true } => {
            out.push_str("\"\"\"");
            out.push_str(text);
            out.push_str("\"\"\"");
        }
        Kv::Prefixed { prefix, text } => {
            out.push_str(prefix);
            out.push_str(":\"");
            out.push_str(text);
            out.push('"');
        }
        Kv::Binary(b) => {
            out.push_str("#[ ");
            out.push_str(b);
            out.push_str(" ]");
        }
        Kv::Array(a) if a.is_empty() => out.push_str("[  ]"),
        Kv::Array(a) => {
            let simple = a.iter().all(|x| matches!(x, Kv::Raw(_))) && a.len() <= 16;
            if simple {
                out.push_str("[ ");
                let parts: Vec<String> = a.iter().map(|x| if let Kv::Raw(r) = x { r.clone() } else { String::new() }).collect();
                out.push_str(&parts.join(", "));
                out.push_str(" ]");
            } else {
                out.push('\n');
                out.push_str(&tabs(depth));
                out.push_str("[\n");
                for x in a {
                    out.push_str(&tabs(depth + 1));
                    match x {
                        Kv::Object(_) => write_object(x, depth + 1, out),
                        _ => write(x, depth + 1, out),
                    }
                    out.push_str(",\n");
                }
                out.push_str(&tabs(depth));
                out.push(']');
            }
        }
        Kv::Object(_) => {
            if depth > 0 {
                out.push('\n');
                out.push_str(&tabs(depth));
            }
            write_object(v, depth, out);
        }
    }
}

/// `{ ... }` starting where the cursor is.
fn write_object(v: &Kv, depth: usize, out: &mut String) {
    let tabs = |n: usize| "\t".repeat(n);
    if let Kv::Object(o) = v {
        {
            out.push_str("{\n");
            for (k, x) in o {
                out.push_str(&tabs(depth + 1));
                if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.') && !k.is_empty() {
                    out.push_str(k);
                } else {
                    out.push('"');
                    out.push_str(&escape(k));
                    out.push('"');
                }
                out.push_str(" = ");
                write(x, depth + 1, out);
                out.push('\n');
            }
            out.push_str(&tabs(depth));
            out.push('}');
        }
    }
}

pub fn to_string(doc: &Doc) -> String {
    let mut out = String::new();
    if !doc.header.is_empty() {
        out.push_str(&doc.header);
        out.push('\n');
    }
    write(&doc.root, 0, &mut out);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<!-- kv3 encoding:text:version{e21c7f3c-8a33-41c5-9977-a76d3a32aa0d} format:modeldoc28:version{fb63b6ca-f435-4aa0-a2c7-c66ddc651dca} -->
{
	rootNode =
	{
		_class = "RootNode"
		children =
		[
			{
				_class = "RenderMeshList"
				children =
				[
					{
						_class = "RenderMeshFile"
						name = "outer_costume"
						filename = "models/heroes_staging/x/x_outer_costume.dmx"
						import_scale = 1.0
					},
				]
			},
			{
				_class = "GenericGameData"
				game_class = "CCitadelHeroModelGameData_t"
				game_keys =
				{
					m_UIAnimGraph = resource:""
					m_vCameraParrotOffset = [ -10.0, -10.0, 10.0 ]
					note = "say \"hi\""
					// a comment
					m_bTurnToFaceVelocity = false
				}
			},
		]
		model_archetype = ""
		text = """line one
line two"""
		blob = #[ 00 01 ff ]
	}
}
"#;

    #[test]
    fn parses_and_round_trips() {
        let d = parse(SAMPLE).unwrap();
        assert!(d.header.contains("modeldoc28"));
        let root = d.root.get("rootNode").unwrap();
        assert_eq!(root.class().unwrap(), "RootNode");
        let kids = root.children().unwrap();
        assert_eq!(kids[0].class().unwrap(), "RenderMeshList");
        let mesh = &kids[0].children().unwrap()[0];
        assert_eq!(mesh.get("filename").unwrap().as_str().unwrap(), "models/heroes_staging/x/x_outer_costume.dmx");
        assert_eq!(mesh.get("import_scale"), Some(&Kv::Raw("1.0".into())));
        let keys = kids[1].get("game_keys").unwrap();
        assert_eq!(keys.get("note").unwrap().as_str().unwrap(), "say \"hi\"");
        assert_eq!(keys.get("m_UIAnimGraph"), Some(&Kv::Prefixed { prefix: "resource".into(), text: "".into() }));
        assert_eq!(root.get("text").unwrap().as_str().unwrap(), "line one\nline two");
        // Write, read back: same tree.
        let again = parse(&to_string(&d)).unwrap();
        assert_eq!(again, d);
    }

    #[test]
    fn edits_and_errors() {
        let mut d = parse(SAMPLE).unwrap();
        let root = d.root.get_mut("rootNode").unwrap();
        root.set("model_archetype", Kv::str("hero"));
        root.children_mut().unwrap().push(Kv::Object(vec![
            ("_class".into(), Kv::str("AnimIncludeModel")),
            ("model".into(), Kv::str("models/a/b_backup.vmdl")),
        ]));
        let text = to_string(&d);
        assert!(text.contains("model_archetype = \"hero\""));
        assert!(text.contains("_class = \"AnimIncludeModel\""));
        assert!(parse(&text).is_ok());
        assert!(parse("{ a = ").is_err());
        assert!(parse("{ a = \"x }").is_err());
    }

    /// The real decompiled Seven, if it's been extracted on this machine.
    #[test]
    fn real_decompiled_model() {
        let p = std::path::Path::new("/tmp/s2v/out/models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner.vmdl");
        if !p.exists() {
            return;
        }
        let text = std::fs::read_to_string(p).unwrap();
        let d = parse(&text).unwrap();
        let again = parse(&to_string(&d)).unwrap();
        assert_eq!(again, d);
        let mut all = vec![];
        d.root.walk(&mut all);
        let bones = all.iter().filter(|n| n.class().as_deref() == Some("Bone")).count();
        assert_eq!(bones, 245);
    }
}
