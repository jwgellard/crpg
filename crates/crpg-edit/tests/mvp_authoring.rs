//! T058 §11 `tests/mvp_authoring.rs`: the spec §17.2 MVP data shape is
//! authorable headlessly, through commands only, to a clean validation.
//! Dialogue and quest behaviour belongs to T053/T054 and is not tested here.

mod support;

use crpg_core::Fx16_16;
use crpg_data::{
    Creature, Dialogue, DialogueBody, DialogueNode, Document, Item, Placement, Quest, QuestState,
    QuestTransition, SourcePath,
};
use crpg_edit::{EditCommand, EditTarget};
use std::collections::{BTreeMap, BTreeSet};
use support::*;

const GOBLIN: u128 = 40;
const MAYOR: u128 = 41;
const SWORD: u128 = 42;
const GOBLIN_PLACEMENT: u128 = 43;
const MAYOR_PLACEMENT: u128 = 44;
const DIALOGUE: u128 = 45;
const GREET: u128 = 46;
const REPLY: u128 = 47;
const END: u128 = 48;
const QUEST: u128 = 50;
const STARTED: u128 = 51;
const COMPLETED: u128 = 52;

fn whole(n: i32) -> Fx16_16 {
    Fx16_16::from_raw(n * 65_536)
}

/// `minimal-d6`-style whole-number stats.
fn stats(might: i32, guile: i32, resolve: i32, health: i32) -> BTreeMap<String, Fx16_16> {
    [
        ("might", might),
        ("guile", guile),
        ("resolve", resolve),
        ("health", health),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), whole(value)))
    .collect()
}

fn creature(n: u128, slug: &str, tag: &str, stats: BTreeMap<String, Fx16_16>) -> Document {
    Document::Creature(Creature {
        id: id(n),
        slug: slug.to_owned(),
        name: format!("mvp.{slug}"),
        note: None,
        stats,
        tags: vec![tag.to_owned()],
        faction: None,
        inventory: Vec::new(),
    })
}

fn create(path: &str, document: Document) -> EditCommand {
    EditCommand::CreateDocument {
        path: sp(path),
        document,
    }
}

/// A locale string, written in the same batch as the field that uses it.
fn text(key: &str, value: &str) -> EditCommand {
    EditCommand::SetValue {
        target: EditTarget::Document(sp("locale/en.json")),
        pointer: format!("/strings/{key}"),
        value: json_str(value),
    }
}

fn place(n: u128, slug: &str, prefab: u128, x: i32) -> EditCommand {
    EditCommand::PlaceInstance {
        area: id(3),
        placement: Placement {
            id: id(n),
            slug: slug.to_owned(),
            name: format!("mvp.placement.{slug}"),
            note: None,
            prefab: id(prefab),
            transform: transform(x),
            overrides: BTreeMap::new(),
        },
    }
}

fn insert(target: u128, pointer: &str, value: String) -> EditCommand {
    EditCommand::InsertValue {
        target: EditTarget::Object(id(target)),
        pointer: pointer.to_owned(),
        value: value.into_bytes(),
    }
}

#[test]
fn mvp_campaign_shape_is_authorable_headlessly() {
    let mut doc = open();
    let baseline = doc.canonical_files().clone();
    let mut apply = |batch: Vec<EditCommand>| {
        doc_apply(&mut doc, batch);
    };

    // A hostile and an NPC creature, each with its display name.
    apply(vec![
        create(
            "creatures/goblin.json",
            creature(GOBLIN, "goblin", "hostile", stats(6, 7, 5, 6)),
        ),
        text("mvp.goblin", "Goblin"),
    ]);
    apply(vec![
        create(
            "creatures/mayor.json",
            creature(MAYOR, "mayor", "npc", stats(3, 6, 8, 8)),
        ),
        text("mvp.mayor", "Mayor"),
    ]);

    // A weapon item.
    apply(vec![
        create(
            "items/sword.json",
            Document::Item(Item {
                id: id(SWORD),
                slug: "sword".to_owned(),
                name: "mvp.sword".to_owned(),
                note: None,
                stats: [("damage".to_owned(), whole(3))].into_iter().collect(),
                tags: vec!["weapon".to_owned()],
            }),
        ),
        text("mvp.sword", "Sword"),
    ]);

    // Placements for both creatures beside the existing spawn.
    apply(vec![
        place(GOBLIN_PLACEMENT, "goblin", GOBLIN, 4),
        text("mvp.placement.goblin", "Goblin"),
    ]);
    apply(vec![
        place(MAYOR_PLACEMENT, "mayor", MAYOR, 2),
        text("mvp.placement.mayor", "Mayor"),
    ]);

    // A dialogue grown node by node: npc_line -> player_choice -> end, the
    // NPC placement speaking.
    apply(vec![
        create(
            "dialogue/mayor.json",
            Document::Dialogue(Dialogue {
                id: id(DIALOGUE),
                slug: "mayor-greeting".to_owned(),
                name: "mvp.dialogue".to_owned(),
                note: None,
                entry: id(GREET),
                nodes: vec![DialogueNode {
                    id: id(GREET),
                    body: DialogueBody::NpcLine {
                        speaker: id(MAYOR_PLACEMENT),
                        text_key: "mvp.dialogue.greet".to_owned(),
                        conditions: Vec::new(),
                        on_enter: Vec::new(),
                        next: None,
                    },
                }],
            }),
        ),
        text("mvp.dialogue", "Greeting"),
        text("mvp.dialogue.greet", "Goblins took the mill."),
    ]);
    apply(vec![
        insert(
            DIALOGUE,
            "/nodes/-",
            format!(
                r#"{{"body":{{"conditions":[],"kind":"player_choice","next":"{}","on_select":[],"text_key":"mvp.dialogue.reply"}},"id":"{}"}}"#,
                id(END),
                id(REPLY)
            ),
        ),
        text("mvp.dialogue.reply", "I will deal with them."),
    ]);
    apply(vec![insert(
        DIALOGUE,
        "/nodes/-",
        format!(r#"{{"body":{{"kind":"end"}},"id":"{}"}}"#, id(END)),
    )]);
    apply(vec![EditCommand::SetValue {
        target: EditTarget::Object(id(GREET)),
        pointer: "/body/next".to_owned(),
        value: format!("\"{}\"", id(REPLY)).into_bytes(),
    }]);

    // A quest with a non-terminal `started` and a terminal `completed` state.
    apply(vec![
        create(
            "quests/clear.json",
            Document::Quest(Quest {
                id: id(QUEST),
                slug: "clear-the-mill".to_owned(),
                name: "mvp.quest".to_owned(),
                note: None,
                entry: id(STARTED),
                states: vec![QuestState {
                    id: id(STARTED),
                    name: "mvp.quest.started".to_owned(),
                    terminal: false,
                    on_enter: Vec::new(),
                    transitions: vec![QuestTransition {
                        condition: "goblin_dead".to_owned(),
                        target: id(COMPLETED),
                    }],
                }],
            }),
        ),
        text("mvp.quest", "Clear the mill"),
        text("mvp.quest.started", "Started"),
    ]);
    apply(vec![
        insert(
            QUEST,
            "/states/-",
            format!(
                r#"{{"id":"{}","name":"mvp.quest.completed","on_enter":[],"terminal":true,"transitions":[]}}"#,
                id(COMPLETED)
            ),
        ),
        text("mvp.quest.completed", "Completed"),
    ]);

    assert_eq!(doc.validate(), Vec::new());
    let plan = doc.save_plan();
    let written: BTreeSet<SourcePath> = plan.write.keys().cloned().collect();
    let expected: BTreeSet<SourcePath> = [
        "areas/start/placements.json",
        "creatures/goblin.json",
        "creatures/mayor.json",
        "dialogue/mayor.json",
        "items/sword.json",
        "locale/en.json",
        "quests/clear.json",
    ]
    .into_iter()
    .map(sp)
    .collect();
    assert_eq!(written, expected);
    assert!(plan.remove.is_empty());
    assert_eq!(validate_files(&apply_plan(&baseline, &plan)), Vec::new());

    // Every authored object is indexed where the commands put it.
    for n in [
        GOBLIN,
        MAYOR,
        SWORD,
        GOBLIN_PLACEMENT,
        MAYOR_PLACEMENT,
        DIALOGUE,
        GREET,
        REPLY,
        END,
        QUEST,
        STARTED,
        COMPLETED,
    ] {
        assert!(doc.campaign().index.contains_key(&id(n)), "{n}");
    }
}

/// Applies one authoring batch, which must commit a change.
fn doc_apply(doc: &mut crpg_edit::CampaignDocument, batch: Vec<EditCommand>) {
    let receipt = doc.apply_batch(batch).expect("authoring command accepted");
    assert!(!receipt.changes.is_empty());
}
