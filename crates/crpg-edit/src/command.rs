//! The closed set of editor mutations and how a pointer edit is addressed.

/// Where a pointer edit applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditTarget {
    /// A whole document; the pointer is absolute within it.
    Document(crpg_data::SourcePath),
    /// An identified object; the pointer is relative to the object's own
    /// location (its index pointer is prefixed), and may be empty.
    Object(crpg_core::Ulid),
}

/// The closed set of editor mutations. Every mutation goes through one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditCommand {
    /// Add a complete new document at an unused path.
    CreateDocument {
        /// Unused logical path the document is stored at.
        path: crpg_data::SourcePath,
        /// The complete typed document, always at its current schema version.
        document: crpg_data::Document,
    },
    /// Remove the document at `path`.
    DeleteDocument {
        /// Logical path of the document to remove.
        path: crpg_data::SourcePath,
    },
    /// Move a document's bytes unchanged from `from` to the unused path `to`.
    RenameDocument {
        /// Logical path the document currently has.
        from: crpg_data::SourcePath,
        /// Unused logical path the document moves to.
        to: crpg_data::SourcePath,
    },
    /// Replace the value at a pointer, or add an absent object member.
    /// `value` is UTF-8 JSON text, parsed by crpg-data's strict parser.
    SetValue {
        /// The document or object the pointer is resolved against.
        target: EditTarget,
        /// RFC 6901 pointer to the value to replace or the member to add.
        pointer: String,
        /// UTF-8 JSON text of the new value.
        value: Vec<u8>,
    },
    /// Insert into an array at a final index token (`0..=len`) or `-` (append).
    InsertValue {
        /// The document or object the pointer is resolved against.
        target: EditTarget,
        /// RFC 6901 pointer whose final token names the insertion point.
        pointer: String,
        /// UTF-8 JSON text of the inserted value.
        value: Vec<u8>,
    },
    /// Remove an existing object member or array element.
    RemoveValue {
        /// The document or object the pointer is resolved against.
        target: EditTarget,
        /// RFC 6901 pointer to the member or element to remove.
        pointer: String,
    },
    /// Delete an identified object: a root object deletes its document, a
    /// nested object is removed from its containing array.
    DeleteObject {
        /// Identity of the object to delete.
        id: crpg_core::Ulid,
    },
    /// Append a placement to the placements aggregate beside the area's
    /// `area.json`, creating that aggregate if it does not exist.
    PlaceInstance {
        /// Identity of the area that owns the placement.
        area: crpg_core::Ulid,
        /// The new placement, with a caller-supplied id.
        placement: crpg_data::Placement,
    },
    /// Replace an existing placement's transform.
    MovePlacement {
        /// Identity of the placement to move.
        placement: crpg_core::Ulid,
        /// The placement's new transform.
        transform: crpg_data::Transform,
    },
}
