//! Pure RFC 6901 pointer edits of one typed document at its current tag.
//!
//! `edit_document` serializes a document with its existing serde shape,
//! applies one `Set`, `Insert` or `Remove` at a JSON Pointer, parses any new
//! value with the crate's strict integer-only parser, and re-decodes the
//! result at the tag it already carries. It never migrates and never retags;
//! every `Ok` result is canonical and writable.

use crate::{DataError, Document};
use serde_json::Value;
use std::fmt;

/// Longest pointer `pointer_tokens` and `edit_document` accept, in bytes:
/// 8 KiB, which covers crpg-edit's 4 KiB caller pointer plus any index
/// pointer prefix (at most 55 bytes).
pub const MAX_EDIT_POINTER_BYTES: usize = 8_192;

/// One RFC 6901 edit of a document's JSON value. `value` is UTF-8 JSON text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointerEdit {
    /// Replace an existing value, or add an absent member of an existing object.
    Set {
        /// RFC 6901 pointer to the target.
        pointer: String,
        /// UTF-8 JSON text of the new value.
        value: Vec<u8>,
    },
    /// Insert into the parent array at the final token: an index `0..=len`, or `-` to append.
    Insert {
        /// RFC 6901 pointer whose final token names the insertion point.
        pointer: String,
        /// UTF-8 JSON text of the inserted value.
        value: Vec<u8>,
    },
    /// Remove an existing object member or array element.
    Remove {
        /// RFC 6901 pointer to the target.
        pointer: String,
    },
}

/// Why a pointer edit was refused. The input document is never changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointerEditError {
    /// The pointer is longer than `MAX_EDIT_POINTER_BYTES`.
    PointerTooLong,
    /// Not RFC 6901 syntax, a bad `~` escape, or a malformed array index token.
    InvalidPointer,
    /// The empty pointer: whole-document replacement is not a pointer edit.
    RootPointer,
    /// The first reference token is `schema`.
    SchemaTag,
    /// The target, or for `Insert` the parent array or index, does not exist.
    NotFound,
    /// The value text fails the strict parser.
    Value {
        /// `Display` of the parser's `DataError`.
        message: String,
    },
    /// The edited value fails current-tag typed decoding, local invariants,
    /// or the canonical writer.
    Document {
        /// `Display` of the underlying `DataError`.
        message: String,
    },
}

impl fmt::Display for PointerEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PointerTooLong => {
                write!(f, "json pointer exceeds {MAX_EDIT_POINTER_BYTES} bytes")
            }
            Self::InvalidPointer => f.write_str("invalid json pointer"),
            Self::RootPointer => f.write_str("json pointer must not be empty"),
            Self::SchemaTag => f.write_str("the schema tag cannot be edited"),
            Self::NotFound => f.write_str("json pointer target not found"),
            Self::Value { message } => write!(f, "invalid value: {message}"),
            Self::Document { message } => write!(f, "edited document is invalid: {message}"),
        }
    }
}

impl std::error::Error for PointerEditError {}

/// Decodes an RFC 6901 pointer into its reference tokens, unescaping `~1`
/// and `~0`. The empty pointer yields no tokens. Fails only with
/// `PointerTooLong` or `InvalidPointer`.
pub fn pointer_tokens(pointer: &str) -> Result<Vec<String>, PointerEditError> {
    if pointer.len() > MAX_EDIT_POINTER_BYTES {
        return Err(PointerEditError::PointerTooLong);
    }
    if pointer.is_empty() {
        return Ok(Vec::new());
    }
    let rest = pointer
        .strip_prefix('/')
        .ok_or(PointerEditError::InvalidPointer)?;
    rest.split('/').map(unescape).collect()
}

/// One left-to-right pass, so `~01` decodes to `~1`, never `/`.
fn unescape(raw: &str) -> Result<String, PointerEditError> {
    let mut token = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '~' {
            match chars.next() {
                Some('0') => token.push('~'),
                Some('1') => token.push('/'),
                _ => return Err(PointerEditError::InvalidPointer),
            }
        } else {
            token.push(c);
        }
    }
    Ok(token)
}

/// An array reference token after the strict index grammar.
enum ArrayToken {
    /// A well-formed index that fits `usize`.
    Index(usize),
    /// `-`: the nonexistent element after the last.
    Append,
    /// A well-formed index that cannot fit `usize`, so no array has it.
    Overflow,
}

/// `0`, or an ASCII digit `1`-`9` followed by ASCII digits; `-`; anything else
/// is `InvalidPointer`.
fn array_token(token: &str) -> Result<ArrayToken, PointerEditError> {
    if token == "-" {
        return Ok(ArrayToken::Append);
    }
    let bytes = token.as_bytes();
    let well_formed = match bytes {
        [] => false,
        [b'0'] => true,
        [first, rest @ ..] => (b'1'..=b'9').contains(first) && rest.iter().all(u8::is_ascii_digit),
    };
    if !well_formed {
        return Err(PointerEditError::InvalidPointer);
    }
    // The grammar is already checked, so a parse failure can only be overflow.
    Ok(token
        .parse::<usize>()
        .map_or(ArrayToken::Overflow, ArrayToken::Index))
}

/// An existing element index (`< len`), or the refusal per the index rules.
fn existing_index(token: &str, len: usize) -> Result<usize, PointerEditError> {
    match array_token(token)? {
        ArrayToken::Index(i) if i < len => Ok(i),
        _ => Err(PointerEditError::NotFound),
    }
}

fn document_error(error: DataError) -> PointerEditError {
    PointerEditError::Document {
        message: error.to_string(),
    }
}

/// Applies one pointer edit to a copy of `document`. Pure; re-decodes at the
/// document's current schema tag only, so it never migrates or retags.
///
/// Precedence, first failure wins: pointer length, pointer syntax, root
/// pointer, schema tag, value text, traversal (missing targets and malformed
/// array indices in left-to-right order), typed decode with local invariants,
/// then the canonical writer.
pub fn edit_document(
    document: &Document,
    edit: &PointerEdit,
) -> Result<Document, PointerEditError> {
    let pointer = match edit {
        PointerEdit::Set { pointer, .. }
        | PointerEdit::Insert { pointer, .. }
        | PointerEdit::Remove { pointer } => pointer,
    };
    let tokens = pointer_tokens(pointer)?;
    let Some((last, parents)) = tokens.split_last() else {
        return Err(PointerEditError::RootPointer);
    };
    if tokens[0] == "schema" {
        return Err(PointerEditError::SchemaTag);
    }
    let parse = |text: &[u8]| {
        crate::canonical::parse(text).map_err(|error| PointerEditError::Value {
            message: error.to_string(),
        })
    };
    let op = match edit {
        PointerEdit::Set { value, .. } => Op::Set(parse(value)?),
        PointerEdit::Insert { value, .. } => Op::Insert(parse(value)?),
        PointerEdit::Remove { .. } => Op::Remove,
    };
    let mut root =
        serde_json::to_value(document).map_err(|e| document_error(crate::error::malformed(e)))?;

    let mut parent = &mut root;
    for token in parents {
        parent = match parent {
            Value::Object(members) => members.get_mut(token.as_str()),
            Value::Array(elements) => {
                let i = existing_index(token, elements.len())?;
                elements.get_mut(i)
            }
            _ => None,
        }
        .ok_or(PointerEditError::NotFound)?;
    }

    match (op, parent) {
        (Op::Set(value), Value::Object(members)) => {
            members.insert(last.clone(), value);
        }
        (Op::Set(value), Value::Array(elements)) => {
            let i = existing_index(last, elements.len())?;
            let slot = elements.get_mut(i).ok_or(PointerEditError::NotFound)?;
            *slot = value;
        }
        (Op::Insert(value), Value::Array(elements)) => {
            let at = match array_token(last)? {
                ArrayToken::Append => elements.len(),
                ArrayToken::Index(i) if i <= elements.len() => i,
                ArrayToken::Index(_) | ArrayToken::Overflow => {
                    return Err(PointerEditError::NotFound)
                }
            };
            elements.insert(at, value);
        }
        (Op::Remove, Value::Object(members)) => {
            members
                .remove(last.as_str())
                .ok_or(PointerEditError::NotFound)?;
        }
        (Op::Remove, Value::Array(elements)) => {
            let i = existing_index(last, elements.len())?;
            elements.remove(i);
        }
        // Insert into an object, or any edit under a string, number, bool or null.
        _ => return Err(PointerEditError::NotFound),
    }

    let decoded = crate::document::decode_current(root).map_err(document_error)?;
    crate::write_document(&decoded).map_err(document_error)?;
    Ok(decoded)
}

/// The edit after its value text has passed the strict parser.
enum Op {
    Set(Value),
    Insert(Value),
    Remove,
}
