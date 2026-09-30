//! Immutable IR action-signature declarations and bounded call validation
//! (T029a, D16).
//!
//! An [`ActionSignatureStore`] holds caller-supplied, trusted
//! [`ActionSignature`] declarations keyed by exact action id. It is
//! declaration-only: it contains no runtime implementation, registers no
//! handler, loads no library and grants no authority to install one. Trusted
//! executable bindings (T029b) must separately match its
//! [`ActionBundleIdentity`] before running anything. IR action ids are
//! symbolic vocabulary strings, a namespace distinct from combat ability
//! ULIDs and from logical paths.
//!
//! Bundle identity is content-derived: the revision is BLAKE3 over the
//! canonical JSON of `{version, bundle, actions}` with actions in lexical id
//! order and parameters in declaration order. Compatibility is exact identity
//! equality — there is no semver or additive fallback.
//!
//! Every failure is one [`SignatureError`] with a deterministic code and a
//! logical JSON pointer, checked in the documented first-failure order.
//! Validation borrows caller data and changes nothing.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Write};

use crpg_core::Ulid;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ActionCall, ActionSignature, DataValue, Digest, ValueType};

/// The only supported declaration-bundle wire version.
pub const ACTION_BUNDLE_VERSION: u32 = 1;
/// Maximum declared actions in one bundle.
pub const MAX_ACTION_SIGNATURES: usize = 1024;
/// Maximum parameters in one action, and arguments in one call.
pub const MAX_ACTION_PARAMETERS: usize = 32;
/// Maximum UTF-8 bytes of an action id, parameter name or argument name.
pub const MAX_ACTION_ID_BYTES: usize = 128;
/// Maximum canonical bytes of one encoded [`ActionCall`], final LF included.
pub const MAX_ACTION_CALL_BYTES: usize = 65_536;
/// Maximum bytes of one serialized declaration bundle.
pub const MAX_ACTION_BUNDLE_BYTES: usize = 33_554_432;
/// Maximum nesting depth of one argument value (its root has depth 1).
pub const MAX_ACTION_VALUE_DEPTH: usize = 32;
/// Maximum `DataValue` nodes across all arguments of one call.
pub const MAX_ACTION_VALUE_NODES: usize = 4096;
/// Maximum UTF-8 bytes of one text value or map key inside a call.
pub const MAX_ACTION_VALUE_STRING_BYTES: usize = 4096;

/// The exact identity a call and its trusted bindings must name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionBundleIdentity {
    /// Caller-chosen bundle identity.
    pub bundle: Ulid,
    /// Content-derived declaration revision.
    pub revision: Digest,
}

/// An immutable, validated set of action declarations.
///
/// Constructed only through [`ActionSignatureStore::new`] or
/// [`read_action_signatures`]; there is no mutator and no unrestricted
/// `Deserialize`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionSignatureStore {
    identity: ActionBundleIdentity,
    signatures: BTreeMap<String, ActionSignature>,
}

/// Which bound a [`SignatureErrorCode::LimitExceeded`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureLimit {
    /// Serialized bundle bytes ([`MAX_ACTION_BUNDLE_BYTES`]).
    BundleBytes,
    /// Declared actions ([`MAX_ACTION_SIGNATURES`]).
    Actions,
    /// Parameters of one action ([`MAX_ACTION_PARAMETERS`]).
    Parameters,
    /// Arguments of one call ([`MAX_ACTION_PARAMETERS`]).
    Arguments,
    /// Canonical call bytes ([`MAX_ACTION_CALL_BYTES`]).
    CallBytes,
    /// Value nesting depth ([`MAX_ACTION_VALUE_DEPTH`]).
    ValueDepth,
    /// Value nodes across a call ([`MAX_ACTION_VALUE_NODES`]).
    ValueNodes,
    /// Bytes of one text value or map key ([`MAX_ACTION_VALUE_STRING_BYTES`]).
    ValueStringBytes,
}

/// The stable failure code of a [`SignatureError`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureErrorCode {
    /// Invalid UTF-8, JSON syntax, duplicate keys, non-integer numbers,
    /// trailing input, or a typed shape mismatch.
    Malformed,
    /// A bundle version other than [`ACTION_BUNDLE_VERSION`].
    UnsupportedVersion,
    /// An action, parameter or argument name outside 1..=128 UTF-8 bytes.
    InvalidIdentifier,
    /// An action id declared more than once.
    DuplicateAction,
    /// A parameter name declared more than once in one action.
    DuplicateParameter,
    /// A serialized bundle's revision disagrees with its content.
    RevisionMismatch,
    /// A call names a bundle identity other than this store's.
    BundleMismatch,
    /// A call names an undeclared action.
    UnknownAction,
    /// A required parameter has no argument.
    MissingArgument,
    /// An argument names no declared parameter.
    ExtraArgument,
    /// An argument's category differs from its parameter's.
    TypeMismatch {
        /// The declared category.
        expected: ValueType,
        /// The supplied category.
        actual: ValueType,
    },
    /// A declaration, call or byte bound was exceeded.
    LimitExceeded {
        /// The exceeded bound.
        kind: SignatureLimit,
        /// Its value.
        limit: usize,
    },
}

impl SignatureErrorCode {
    /// The stable snake-case spelling used by `Display`.
    fn as_str(&self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::UnsupportedVersion => "unsupported_version",
            Self::InvalidIdentifier => "invalid_identifier",
            Self::DuplicateAction => "duplicate_action",
            Self::DuplicateParameter => "duplicate_parameter",
            Self::RevisionMismatch => "revision_mismatch",
            Self::BundleMismatch => "bundle_mismatch",
            Self::UnknownAction => "unknown_action",
            Self::MissingArgument => "missing_argument",
            Self::ExtraArgument => "extra_argument",
            Self::TypeMismatch { .. } => "type_mismatch",
            Self::LimitExceeded { .. } => "limit_exceeded",
        }
    }
}

/// The first failure found, with a logical RFC 6901 JSON pointer.
///
/// Pointers are stable symbolic locations (`/actions/<id>`,
/// `/args/<name>/value/0`), not source offsets; the root is `""`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureError {
    /// What failed.
    pub code: SignatureErrorCode,
    /// Where it failed.
    pub pointer: String,
}

impl SignatureError {
    fn new(code: SignatureErrorCode, pointer: impl Into<String>) -> Self {
        Self {
            code,
            pointer: pointer.into(),
        }
    }

    fn limit(kind: SignatureLimit, limit: usize, pointer: impl Into<String>) -> Self {
        Self::new(SignatureErrorCode::LimitExceeded { kind, limit }, pointer)
    }

    fn malformed(pointer: impl Into<String>) -> Self {
        Self::new(SignatureErrorCode::Malformed, pointer)
    }
}

impl fmt::Display for SignatureError {
    /// Renders `<snake_case_code> at <pointer>`, with the root shown as `/`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pointer = if self.pointer.is_empty() {
            "/"
        } else {
            &self.pointer
        };
        write!(f, "{} at {pointer}", self.code.as_str())
    }
}

impl std::error::Error for SignatureError {}

/// Escapes one RFC 6901 reference token (`~` then `/`).
fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn valid_identifier(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_ACTION_ID_BYTES
}

fn category(value: &DataValue) -> ValueType {
    match value {
        DataValue::Bool(_) => ValueType::Bool,
        DataValue::Integer(_) => ValueType::Integer,
        DataValue::Unsigned(_) => ValueType::Unsigned,
        DataValue::Fixed(_) => ValueType::Fixed,
        DataValue::Text(_) => ValueType::Text,
        DataValue::ObjectRef(_) => ValueType::ObjectRef,
        DataValue::List(_) => ValueType::List,
        DataValue::Map(_) => ValueType::Map,
    }
}

/// Where a [`CappedWriter`] sends accepted bytes.
enum Sink {
    /// Count only; retain nothing.
    Count,
    /// Retain the bytes.
    Keep(Vec<u8>),
    /// Stream the bytes into a BLAKE3 hasher.
    Hash(Box<blake3::Hasher>),
}

/// A writer that fails once more than `cap` bytes are written.
struct CappedWriter {
    cap: usize,
    written: usize,
    sink: Sink,
}

impl Write for CappedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let next = self
            .written
            .checked_add(buf.len())
            .filter(|next| *next <= self.cap)
            .ok_or_else(|| io::Error::other("capacity exceeded"))?;
        self.written = next;
        match &mut self.sink {
            Sink::Count => {}
            Sink::Keep(bytes) => bytes.extend_from_slice(buf),
            Sink::Hash(hasher) => {
                hasher.update(buf);
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Encodes `value` in the crate's canonical pretty form (two-space indent,
/// one final LF) through a capped writer, never exceeding `cap` bytes.
///
/// `value` must already serialize its object keys in lexical order; the
/// private views below do, and tests pin equality with `canonical_json`.
fn capped_canonical<T: Serialize + ?Sized>(value: &T, cap: usize, sink: Sink) -> Option<Sink> {
    let mut writer = CappedWriter {
        cap,
        written: 0,
        sink,
    };
    serde_json::to_writer_pretty(&mut writer, value).ok()?;
    writer.write_all(b"\n").ok()?;
    Some(writer.sink)
}

/// Canonical bytes of `value` within `cap`, or `None` past it.
fn capped_bytes<T: Serialize + ?Sized>(value: &T, cap: usize) -> Option<Vec<u8>> {
    match capped_canonical(value, cap, Sink::Keep(Vec::new()))? {
        Sink::Keep(bytes) => Some(bytes),
        Sink::Count | Sink::Hash(_) => None,
    }
}

/// Canonical view of one parameter: keys in lexical order.
#[derive(Serialize)]
struct ParameterView<'a> {
    name: &'a str,
    required: bool,
    value_type: &'a ValueType,
}

/// Canonical view of one signature: keys in lexical order, parameters in
/// declaration order.
#[derive(Serialize)]
struct SignatureView<'a> {
    action_id: &'a str,
    parameters: Vec<ParameterView<'a>>,
}

impl<'a> SignatureView<'a> {
    fn of(signature: &'a ActionSignature) -> Self {
        Self {
            action_id: &signature.action_id,
            parameters: signature
                .parameters
                .iter()
                .map(|parameter| ParameterView {
                    name: &parameter.name,
                    required: parameter.required,
                    value_type: &parameter.value_type,
                })
                .collect(),
        }
    }
}

/// Canonical view of the bundle envelope; `revision` is absent in the
/// revision preimage and present on the wire.
#[derive(Serialize)]
struct BundleView<'a> {
    actions: Vec<SignatureView<'a>>,
    bundle: Ulid,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<Digest>,
    version: u32,
}

impl<'a> BundleView<'a> {
    fn of(
        bundle: Ulid,
        signatures: &'a BTreeMap<String, ActionSignature>,
        revision: Option<Digest>,
    ) -> Self {
        Self {
            actions: signatures.values().map(SignatureView::of).collect(),
            bundle,
            revision,
            version: ACTION_BUNDLE_VERSION,
        }
    }
}

/// Checks constructor rules in the pinned order without cloning entries.
fn check_declarations(signatures: &[ActionSignature]) -> Result<(), SignatureError> {
    if signatures.len() > MAX_ACTION_SIGNATURES {
        return Err(SignatureError::limit(
            SignatureLimit::Actions,
            MAX_ACTION_SIGNATURES,
            "/actions",
        ));
    }
    let mut order: Vec<&ActionSignature> = signatures.iter().collect();
    order.sort_by(|left, right| left.action_id.cmp(&right.action_id));
    for signature in &order {
        if !valid_identifier(&signature.action_id) {
            return Err(SignatureError::new(
                SignatureErrorCode::InvalidIdentifier,
                format!("/actions/{}", escape(&signature.action_id)),
            ));
        }
    }
    for pair in order.windows(2) {
        if pair[0].action_id == pair[1].action_id {
            return Err(SignatureError::new(
                SignatureErrorCode::DuplicateAction,
                format!("/actions/{}", escape(&pair[0].action_id)),
            ));
        }
    }
    for signature in &order {
        let action = format!("/actions/{}", escape(&signature.action_id));
        if signature.parameters.len() > MAX_ACTION_PARAMETERS {
            return Err(SignatureError::limit(
                SignatureLimit::Parameters,
                MAX_ACTION_PARAMETERS,
                format!("{action}/parameters"),
            ));
        }
        let mut names: Vec<&str> = signature
            .parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect();
        names.sort_unstable();
        for name in &names {
            if !valid_identifier(name) {
                return Err(SignatureError::new(
                    SignatureErrorCode::InvalidIdentifier,
                    format!("{action}/parameters/{}", escape(name)),
                ));
            }
        }
        for pair in names.windows(2) {
            if pair[0] == pair[1] {
                return Err(SignatureError::new(
                    SignatureErrorCode::DuplicateParameter,
                    format!("{action}/parameters/{}", escape(pair[0])),
                ));
            }
        }
    }
    Ok(())
}

/// Derives the declaration revision from canonical preimage bytes.
fn revision_of(
    bundle: Ulid,
    signatures: &BTreeMap<String, ActionSignature>,
) -> Result<Digest, SignatureError> {
    let view = BundleView::of(bundle, signatures, None);
    let sink = Sink::Hash(Box::new(blake3::Hasher::new()));
    match capped_canonical(&view, MAX_ACTION_BUNDLE_BYTES, sink) {
        Some(Sink::Hash(hasher)) => Ok(Digest::from_bytes(*hasher.finalize().as_bytes())),
        _ => Err(SignatureError::limit(
            SignatureLimit::BundleBytes,
            MAX_ACTION_BUNDLE_BYTES,
            "",
        )),
    }
}

/// One pending container during the iterative value walk.
enum Children<'a> {
    Args(std::collections::btree_map::Iter<'a, String, DataValue>),
    List(std::iter::Enumerate<std::slice::Iter<'a, DataValue>>),
    Map(std::collections::btree_map::Iter<'a, String, DataValue>),
}

/// One path segment of a visited node, resolved to a pointer only on error.
enum Segment<'a> {
    Arg(&'a str),
    Index(usize),
    Key(&'a str),
}

/// Visited nodes: parent index and segment, bounded by the node budget.
struct Trail<'a> {
    nodes: Vec<(Option<usize>, Segment<'a>)>,
}

impl Trail<'_> {
    /// The pointer of `parent` extended by `segment`.
    fn pointer(&self, parent: Option<usize>, segment: &Segment<'_>) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push(Self::render(segment));
        let mut cursor = parent;
        while let Some(index) = cursor {
            let (up, segment) = &self.nodes[index];
            parts.push(Self::render(segment));
            cursor = *up;
        }
        parts.reverse();
        parts.concat()
    }

    fn render(segment: &Segment<'_>) -> String {
        match segment {
            Segment::Arg(name) => format!("/args/{}", escape(name)),
            Segment::Index(index) => format!("/value/{index}"),
            Segment::Key(key) => format!("/value/{}", escape(key)),
        }
    }
}

/// Stage 3: depth, node and string budgets, iteratively, before any
/// recursive serde traversal of the call.
fn check_value_budgets(call: &ActionCall) -> Result<(), SignatureError> {
    let mut trail = Trail { nodes: Vec::new() };
    let mut stack: Vec<(Children<'_>, usize, Option<usize>)> =
        vec![(Children::Args(call.args.iter()), 1, None)];
    let mut total: usize = 0;
    while let Some((children, depth, parent)) = stack.last_mut() {
        let (depth, parent) = (*depth, *parent);
        let next = match children {
            Children::Args(iter) => iter
                .next()
                .map(|(name, value)| (Segment::Arg(name.as_str()), value)),
            Children::List(iter) => iter
                .next()
                .map(|(index, value)| (Segment::Index(index), value)),
            Children::Map(iter) => iter
                .next()
                .map(|(key, value)| (Segment::Key(key.as_str()), value)),
        };
        let Some((segment, value)) = next else {
            stack.pop();
            continue;
        };
        if let Segment::Key(key) = &segment {
            if key.len() > MAX_ACTION_VALUE_STRING_BYTES {
                return Err(SignatureError::limit(
                    SignatureLimit::ValueStringBytes,
                    MAX_ACTION_VALUE_STRING_BYTES,
                    trail.pointer(parent, &segment),
                ));
            }
        }
        if depth > MAX_ACTION_VALUE_DEPTH {
            return Err(SignatureError::limit(
                SignatureLimit::ValueDepth,
                MAX_ACTION_VALUE_DEPTH,
                trail.pointer(parent, &segment),
            ));
        }
        total += 1;
        if total > MAX_ACTION_VALUE_NODES {
            return Err(SignatureError::limit(
                SignatureLimit::ValueNodes,
                MAX_ACTION_VALUE_NODES,
                trail.pointer(parent, &segment),
            ));
        }
        match value {
            DataValue::Text(text) if text.len() > MAX_ACTION_VALUE_STRING_BYTES => {
                let mut pointer = trail.pointer(parent, &segment);
                pointer.push_str("/value");
                return Err(SignatureError::limit(
                    SignatureLimit::ValueStringBytes,
                    MAX_ACTION_VALUE_STRING_BYTES,
                    pointer,
                ));
            }
            DataValue::List(items) => {
                trail.nodes.push((parent, segment));
                let index = trail.nodes.len() - 1;
                stack.push((
                    Children::List(items.iter().enumerate()),
                    depth + 1,
                    Some(index),
                ));
            }
            DataValue::Map(entries) => {
                trail.nodes.push((parent, segment));
                let index = trail.nodes.len() - 1;
                stack.push((Children::Map(entries.iter()), depth + 1, Some(index)));
            }
            _ => {}
        }
    }
    Ok(())
}

impl ActionSignatureStore {
    /// Validates and assembles trusted declarations once, deriving the
    /// bundle revision.
    ///
    /// First failure, in order: more than [`MAX_ACTION_SIGNATURES`] actions
    /// (checked before any sorting or cloning); invalid action ids in lexical
    /// id order; the lexically first duplicate action id; then per action in
    /// lexical order: parameter count, invalid parameter names in lexical
    /// order, the lexically first duplicate parameter name. Identifiers are
    /// 1..=128 UTF-8 bytes, preserved exactly (no normalization). Parameter
    /// declaration order is preserved. Empty bundles and zero-parameter
    /// actions are valid.
    pub fn new(bundle: Ulid, signatures: Vec<ActionSignature>) -> Result<Self, SignatureError> {
        check_declarations(&signatures)?;
        let map: BTreeMap<String, ActionSignature> = signatures
            .into_iter()
            .map(|signature| (signature.action_id.clone(), signature))
            .collect();
        let revision = revision_of(bundle, &map)?;
        Ok(Self {
            identity: ActionBundleIdentity { bundle, revision },
            signatures: map,
        })
    }

    /// The exact identity calls and trusted bindings must name.
    pub fn identity(&self) -> &ActionBundleIdentity {
        &self.identity
    }

    /// The number of declared actions.
    pub fn len(&self) -> usize {
        self.signatures.len()
    }

    /// Whether no action is declared.
    pub fn is_empty(&self) -> bool {
        self.signatures.is_empty()
    }

    /// The declaration for exactly `action_id`, or `None` for any absent
    /// spelling (invalid ids included). Inert: not a validation operation.
    pub fn get(&self, action_id: &str) -> Option<&ActionSignature> {
        self.signatures.get(action_id)
    }

    /// Every declaration in exact lexical Rust string order of its id.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ActionSignature)> + '_ {
        self.signatures
            .iter()
            .map(|(id, signature)| (id.as_str(), signature))
    }

    /// Validates one call against this store, read-only.
    ///
    /// First failure, in order: `identity` differs (`BundleMismatch` at the
    /// root); action id then argument names (lexical) outside 1..=128 bytes;
    /// more than 32 arguments; depth, node and string budgets by iterative
    /// depth-first traversal (arguments lexical, lists by index, maps by
    /// key); canonical call bytes above [`MAX_ACTION_CALL_BYTES`] (counted,
    /// not retained); unknown action; extra arguments (lexical); then
    /// declared parameters in declaration order: missing required, then
    /// category mismatch. Categories are exact: no coercion, and containers
    /// constrain only their root category.
    pub fn validate_call(
        &self,
        identity: &ActionBundleIdentity,
        call: &ActionCall,
    ) -> Result<&ActionSignature, SignatureError> {
        if *identity != self.identity {
            return Err(SignatureError::new(SignatureErrorCode::BundleMismatch, ""));
        }
        if !valid_identifier(&call.action_id) {
            return Err(SignatureError::new(
                SignatureErrorCode::InvalidIdentifier,
                "/action_id",
            ));
        }
        for name in call.args.keys() {
            if !valid_identifier(name) {
                return Err(SignatureError::new(
                    SignatureErrorCode::InvalidIdentifier,
                    format!("/args/{}", escape(name)),
                ));
            }
        }
        if call.args.len() > MAX_ACTION_PARAMETERS {
            return Err(SignatureError::limit(
                SignatureLimit::Arguments,
                MAX_ACTION_PARAMETERS,
                "/args",
            ));
        }
        check_value_budgets(call)?;
        if capped_canonical(call, MAX_ACTION_CALL_BYTES, Sink::Count).is_none() {
            return Err(SignatureError::limit(
                SignatureLimit::CallBytes,
                MAX_ACTION_CALL_BYTES,
                "",
            ));
        }
        let Some(signature) = self.signatures.get(&call.action_id) else {
            return Err(SignatureError::new(
                SignatureErrorCode::UnknownAction,
                "/action_id",
            ));
        };
        for name in call.args.keys() {
            if !signature
                .parameters
                .iter()
                .any(|parameter| parameter.name == *name)
            {
                return Err(SignatureError::new(
                    SignatureErrorCode::ExtraArgument,
                    format!("/args/{}", escape(name)),
                ));
            }
        }
        for parameter in &signature.parameters {
            match call.args.get(&parameter.name) {
                None if parameter.required => {
                    return Err(SignatureError::new(
                        SignatureErrorCode::MissingArgument,
                        format!("/args/{}", escape(&parameter.name)),
                    ));
                }
                None => {}
                Some(value) => {
                    let actual = category(value);
                    if actual != parameter.value_type {
                        return Err(SignatureError::new(
                            SignatureErrorCode::TypeMismatch {
                                expected: parameter.value_type.clone(),
                                actual,
                            },
                            format!("/args/{}", escape(&parameter.name)),
                        ));
                    }
                }
            }
        }
        Ok(signature)
    }

    /// Reads and validates one encoded call.
    ///
    /// Order: raw input above [`MAX_ACTION_CALL_BYTES`] (`CallBytes` at the
    /// root); `identity` mismatch; strict JSON (UTF-8, unique keys, integers
    /// only, no trailing input) and exact `ActionCall` shape, `Malformed` at
    /// the root; then every [`validate_call`](Self::validate_call) stage.
    pub fn read_call(
        &self,
        identity: &ActionBundleIdentity,
        bytes: &[u8],
    ) -> Result<ActionCall, SignatureError> {
        if bytes.len() > MAX_ACTION_CALL_BYTES {
            return Err(SignatureError::limit(
                SignatureLimit::CallBytes,
                MAX_ACTION_CALL_BYTES,
                "",
            ));
        }
        if *identity != self.identity {
            return Err(SignatureError::new(SignatureErrorCode::BundleMismatch, ""));
        }
        let value = crate::canonical::parse(bytes).map_err(|_| SignatureError::malformed(""))?;
        let call: ActionCall =
            serde_json::from_value(value).map_err(|_| SignatureError::malformed(""))?;
        self.validate_call(identity, &call)?;
        Ok(call)
    }

    /// Validates one call, then encodes it as canonical bytes. A rejection
    /// leaves the store and the call unchanged.
    pub fn write_call(
        &self,
        identity: &ActionBundleIdentity,
        call: &ActionCall,
    ) -> Result<Vec<u8>, SignatureError> {
        self.validate_call(identity, call)?;
        capped_bytes(call, MAX_ACTION_CALL_BYTES).ok_or_else(|| {
            SignatureError::limit(SignatureLimit::CallBytes, MAX_ACTION_CALL_BYTES, "")
        })
    }
}

/// Reads one serialized declaration bundle.
///
/// Order: raw input above [`MAX_ACTION_BUNDLE_BYTES`] (`BundleBytes` at the
/// root, before any parsing); strict JSON (UTF-8, unique keys, integers only,
/// no trailing input; `Malformed` at the root); exactly the `version`,
/// `bundle`, `revision`, `actions` fields (`Malformed` at the root); an
/// unsigned integer version (`Malformed` at `/version`) equal to
/// [`ACTION_BUNDLE_VERSION`] (`UnsupportedVersion` at `/version`); typed
/// bundle ULID, revision digest and actions (`Malformed` at `/bundle`,
/// `/revision`, `/actions`); the constructor checks of
/// [`ActionSignatureStore::new`]; finally the recomputed revision
/// (`RevisionMismatch` at `/revision`).
pub fn read_action_signatures(bytes: &[u8]) -> Result<ActionSignatureStore, SignatureError> {
    if bytes.len() > MAX_ACTION_BUNDLE_BYTES {
        return Err(SignatureError::limit(
            SignatureLimit::BundleBytes,
            MAX_ACTION_BUNDLE_BYTES,
            "",
        ));
    }
    let value = crate::canonical::parse(bytes).map_err(|_| SignatureError::malformed(""))?;
    let Value::Object(mut envelope) = value else {
        return Err(SignatureError::malformed(""));
    };
    let mut keys: Vec<&str> = envelope.keys().map(String::as_str).collect();
    keys.sort_unstable();
    if keys != ["actions", "bundle", "revision", "version"] {
        return Err(SignatureError::malformed(""));
    }
    let version = envelope
        .remove("version")
        .and_then(|version| version.as_u64())
        .ok_or_else(|| SignatureError::malformed("/version"))?;
    if version != u64::from(ACTION_BUNDLE_VERSION) {
        return Err(SignatureError::new(
            SignatureErrorCode::UnsupportedVersion,
            "/version",
        ));
    }
    let bundle: Ulid = envelope
        .remove("bundle")
        .and_then(|bundle| serde_json::from_value(bundle).ok())
        .ok_or_else(|| SignatureError::malformed("/bundle"))?;
    let revision: Digest = envelope
        .remove("revision")
        .and_then(|revision| serde_json::from_value(revision).ok())
        .ok_or_else(|| SignatureError::malformed("/revision"))?;
    let actions: Vec<ActionSignature> = envelope
        .remove("actions")
        .and_then(|actions| serde_json::from_value(actions).ok())
        .ok_or_else(|| SignatureError::malformed("/actions"))?;
    let store = ActionSignatureStore::new(bundle, actions)?;
    if store.identity.revision != revision {
        return Err(SignatureError::new(
            SignatureErrorCode::RevisionMismatch,
            "/revision",
        ));
    }
    Ok(store)
}

/// Writes one validated store as canonical bundle bytes (`version`,
/// `bundle`, `revision`, `actions`; actions in lexical id order, parameters
/// in declaration order), bounded by [`MAX_ACTION_BUNDLE_BYTES`].
pub fn write_action_signatures(store: &ActionSignatureStore) -> Result<Vec<u8>, SignatureError> {
    let view = BundleView::of(
        store.identity.bundle,
        &store.signatures,
        Some(store.identity.revision),
    );
    capped_bytes(&view, MAX_ACTION_BUNDLE_BYTES).ok_or_else(|| {
        SignatureError::limit(SignatureLimit::BundleBytes, MAX_ACTION_BUNDLE_BYTES, "")
    })
}
