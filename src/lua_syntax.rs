//! Windower Lua dialect normalization for stock Lua engines. Source snapshots stay
//! untouched; inserted parentheses preserve line numbering, comments and byte strings.
pub struct Normalized {
    pub bytes: Vec<u8>,
    pub literal_methods: usize,
    pub immediate_functions: usize,
}
fn bracket(source: &[u8], start: usize) -> Option<(usize, usize)> {
    if source.get(start) != Some(&b'[') {
        return None;
    }
    let mut end = start + 1;
    while source.get(end) == Some(&b'=') {
        end += 1;
    }
    (source.get(end) == Some(&b'[')).then_some((end + 1, end - start - 1))
}
fn long_end(source: &[u8], start: usize, equals: usize) -> usize {
    let mut i = start;
    while i < source.len() {
        if source[i] == b']'
            && source
                .get(i + 1..i + 1 + equals)
                .is_some_and(|s| s.iter().all(|b| *b == b'='))
            && source.get(i + 1 + equals) == Some(&b']')
        {
            return i + 2 + equals;
        }
        i += 1;
    }
    source.len()
}
#[derive(PartialEq)]
enum Kind {
    Word,
    Number,
    String,
    Symbol,
}
struct Token {
    start: usize,
    end: usize,
    kind: Kind,
}
fn tokens(source: &[u8]) -> Vec<Token> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < source.len() {
        if source[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if source.get(i..i + 2) == Some(b"--") {
            i = if let Some((start, eq)) = bracket(source, i + 2) {
                long_end(source, start, eq)
            } else {
                source[i..]
                    .iter()
                    .position(|b| *b == b'\n')
                    .map_or(source.len(), |n| i + n)
            };
            continue;
        }
        let start = i;
        let kind = if matches!(source[i], b'\'' | b'"') {
            let quote = source[i];
            i += 1;
            while i < source.len() {
                if source[i] == b'\\' {
                    i = (i + 2).min(source.len());
                } else if source[i] == quote {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            Kind::String
        } else if let Some((content, eq)) = bracket(source, i) {
            i = long_end(source, content, eq);
            Kind::String
        } else if source[i].is_ascii_alphabetic() || source[i] == b'_' {
            i += 1;
            while source
                .get(i)
                .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
            {
                i += 1;
            }
            Kind::Word
        } else if source[i].is_ascii_digit() {
            if source
                .get(i..i + 2)
                .is_some_and(|s| s == b"0x" || s == b"0X")
            {
                i += 2;
                while source.get(i).is_some_and(u8::is_ascii_hexdigit) {
                    i += 1;
                }
            } else {
                while source.get(i).is_some_and(u8::is_ascii_digit) {
                    i += 1;
                }
                if source.get(i) == Some(&b'.') && source.get(i + 1) != Some(&b'.') {
                    i += 1;
                    while source.get(i).is_some_and(u8::is_ascii_digit) {
                        i += 1;
                    }
                }
                if source.get(i).is_some_and(|b| matches!(b, b'e' | b'E')) {
                    i += 1;
                    if source.get(i).is_some_and(|b| matches!(b, b'+' | b'-')) {
                        i += 1;
                    }
                    while source.get(i).is_some_and(u8::is_ascii_digit) {
                        i += 1;
                    }
                }
            }
            Kind::Number
        } else {
            i += 1;
            Kind::Symbol
        };
        out.push(Token {
            start,
            end: i,
            kind,
        });
    }
    out
}
struct Block {
    anonymous: Option<usize>,
    waiting_do: bool,
}
pub fn normalize(source: &[u8]) -> Normalized {
    let mut result = Normalized {
        bytes: Vec::new(),
        literal_methods: 0,
        immediate_functions: 0,
    };
    if source.first() == Some(&0x1b) {
        result.bytes = source.to_vec();
        return result;
    }
    let list = tokens(source);
    let mut blocks: Vec<Block> = Vec::new();
    let mut before = vec![0u8; source.len() + 1];
    let mut after = vec![0u8; source.len() + 1];
    for (index, t) in list.iter().enumerate() {
        let text = &source[t.start..t.end];
        let next = list.get(index + 1).map(|n| &source[n.start..n.end]);
        if (t.kind == Kind::String
            || t.kind == Kind::Number
            || (t.kind == Kind::Word && matches!(text, b"true" | b"false" | b"nil")))
            && next == Some(b":")
        {
            before[t.start] += 1;
            after[t.end] += 1;
            result.literal_methods += 1;
        }
        if t.kind != Kind::Word {
            continue;
        }
        match text {
            b"function" => blocks.push(Block {
                anonymous: (next == Some(b"(")).then_some(t.start),
                waiting_do: false,
            }),
            b"if" | b"repeat" => blocks.push(Block {
                anonymous: None,
                waiting_do: false,
            }),
            b"for" | b"while" => blocks.push(Block {
                anonymous: None,
                waiting_do: true,
            }),
            b"do" => {
                if let Some(block) = blocks.last_mut().filter(|b| b.waiting_do) {
                    block.waiting_do = false;
                } else {
                    blocks.push(Block {
                        anonymous: None,
                        waiting_do: false,
                    });
                }
            }
            b"end" | b"until" => {
                if let Some(block) = blocks.pop()
                    && let Some(start) = block.anonymous
                    && next.is_some_and(|n| matches!(n, b"(" | b"{" | b":" | b"[" | b"."))
                {
                    before[start] += 1;
                    after[t.end] += 1;
                    result.immediate_functions += 1;
                }
            }
            _ => {}
        }
    }
    result
        .bytes
        .reserve(source.len() + 2 * (result.literal_methods + result.immediate_functions));
    for i in 0..=source.len() {
        for _ in 0..after[i] {
            result.bytes.push(b')');
        }
        for _ in 0..before[i] {
            result.bytes.push(b'(');
        }
        if let Some(byte) = source.get(i) {
            result.bytes.push(*byte);
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_calls_preserve_comments_escapes_long_strings_and_line_numbers() {
        let original=b"-- 'comment':ignored()\nlocal x='it\\'s':format()\nlocal y=[=[hello]=]:upper()\n--[==[ 'hidden':ignored() ]==]\nlocal z=\"a:b\"\n";
        let n = normalize(original);
        assert_eq!(n.literal_methods, 2);
        assert_eq!(n.bytes,b"-- 'comment':ignored()\nlocal x=('it\\'s'):format()\nlocal y=([=[hello]=]):upper()\n--[==[ 'hidden':ignored() ]==]\nlocal z=\"a:b\"\n");
        assert_eq!(normalize(&n.bytes).bytes, n.bytes);
    }
    #[test]
    fn preserves_non_utf8_source_and_standard_lua() {
        for source in [
            b"local x=string.format('%s',v)".as_slice(),
            b"-- copyright \xa9\nreturn 'ok'",
            b"--[=[unclosed",
            b"return 'unterminated",
            b"\x1bLua 'opaque':bytes",
        ] {
            assert_eq!(normalize(source).bytes, source);
        }
    }
    #[test]
    fn immediate_closures_and_primitive_methods_respect_nested_blocks() {
        let s=b"local f=function() if true then for i=1,2 do local f=function() return 1 end() end else repeat do break end until true end return function()end end()\nlocal a=true:fn();local b=1:exp()+5:sqrt()";
        let n = normalize(s);
        assert_eq!(n.immediate_functions, 2);
        assert_eq!(n.literal_methods, 3);
        assert_eq!(
            String::from_utf8(n.bytes.clone()).unwrap(),
            "local f=(function() if true then for i=1,2 do local f=(function() return 1 end)() end else repeat do break end until true end return function()end end)()\nlocal a=(true):fn();local b=(1):exp()+(5):sqrt()"
        );
        assert_eq!(normalize(&n.bytes).bytes, n.bytes);
    }
}
