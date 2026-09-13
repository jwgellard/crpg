//! Serializable authored event IR; execution and scheduling are external.

use crate::{DataValue, ValueType, VarDecl};
use crpg_core::Ulid;
use schemars::JsonSchema;
use serde::{de, Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// An independently stored or aggregate-owned authored event graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventGraph {
    /// Stable authored-object identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub id: Ulid,
    /// Renameable human-facing slug.
    pub slug: String,
    /// Locale key for the display name.
    pub name: String,
    /// Preserved author annotation.
    #[serde(rename = "_note", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Initiating trigger.
    pub entry: Trigger,
    /// First node identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub start: Ulid,
    /// Nodes in authored order.
    pub nodes: Vec<Node>,
    /// Connections in authored order.
    pub edges: Vec<Edge>,
    /// Graph-local declarations.
    pub locals: Vec<VarDecl>,
}

/// Declarative initiating event; symbolic custom ids are not object ids.
///
/// Deserialization rejects unknown fields for every variant, including unit
/// variants where serde's derived `deny_unknown_fields` would otherwise ignore
/// trailing fields on internally tagged enums.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Trigger {
    /// Entering an area.
    AreaEnter,
    /// Leaving an area.
    AreaExit,
    /// Interacting with an object.
    Interact,
    /// Entering a dialogue node.
    DialogueNode,
    /// Changing quest state.
    QuestStateChange,
    /// Relative tick timer.
    Timer {
        /// Relative ticks, including zero and the full unsigned range.
        ticks: u64,
    },
    /// Starting combat.
    CombatStart,
    /// Ending combat.
    CombatEnd,
    /// Death event.
    Death,
    /// Acquiring an item.
    ItemAcquired,
    /// Extension vocabulary trigger.
    Custom {
        /// Symbolic vocabulary key.
        id: String,
    },
}

/// Identified graph node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Node {
    /// Stable node identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub id: Ulid,
    /// Declarative operation.
    pub body: NodeBody,
}

/// Stored operation; expressions remain opaque until later compilation.
///
/// Deserialization rejects unknown fields for every variant, including unit
/// shapes, for the same internally tagged reason documented on [`Trigger`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NodeBody {
    /// Evaluate a condition with true/false ports.
    Condition {
        /// Opaque expression.
        expr: String,
    },
    /// Invoke an action with a next port.
    Action {
        /// Symbolic action and arguments.
        call: ActionCall,
    },
    /// Branch through zero-based case ports.
    Branch {
        /// Opaque expression.
        expr: String,
        /// Cases in authored order.
        cases: Vec<DataValue>,
    },
    /// Ordered references to nodes in this graph.
    Sequence {
        /// Child node identities, in execution order.
        #[schemars(with = "Vec<crate::schema::UlidText>")]
        nodes: Vec<Ulid>,
    },
    /// Wait a relative tick count; no absolute scheduling occurs here.
    Wait {
        /// Relative ticks, including zero and the full unsigned range.
        ticks: u64,
    },
    /// Call a script vocabulary entry.
    CallScript {
        /// Symbolic script key.
        script_id: String,
        /// Named tagged arguments.
        args: BTreeMap<String, DataValue>,
    },
    /// Call another authored graph.
    CallGraph {
        /// Authored graph identity.
        #[schemars(with = "crate::schema::UlidText")]
        graph_id: Ulid,
        /// Named tagged arguments.
        args: BTreeMap<String, DataValue>,
    },
}

/// Output port of a graph node.
///
/// Deserialization rejects unknown fields for every variant, including unit
/// shapes, for the same internally tagged reason documented on [`Trigger`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Port {
    /// Sequential continuation.
    Next,
    /// Condition succeeds.
    True,
    /// Condition fails.
    False,
    /// Branch case.
    Case {
        /// Zero-based authored case position.
        index: u32,
    },
}

/// Ordered graph connection; endpoint validation is deferred to T011.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    /// Source node identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub from: Ulid,
    /// Source output port.
    pub port: Port,
    /// Destination node identity.
    #[schemars(with = "crate::schema::UlidText")]
    pub to: Ulid,
}

/// Declarative action invocation, without function pointers or a registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionCall {
    /// Symbolic action vocabulary key.
    pub action_id: String,
    /// Named tagged arguments.
    pub args: BTreeMap<String, DataValue>,
}

/// Schema-able declaration of an action's parameter surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionSignature {
    /// Symbolic action vocabulary key.
    pub action_id: String,
    /// Parameters in declaration order.
    pub parameters: Vec<ActionParameter>,
}

/// Declarative action parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionParameter {
    /// Argument name.
    pub name: String,
    /// Accepted category.
    pub value_type: ValueType,
    /// Whether the caller must supply this argument.
    pub required: bool,
}

fn exact_keys<'de, D: serde::Deserializer<'de>>(
    map: &BTreeMap<String, Value>,
    allowed: &'static [&'static str],
) -> Result<(), D::Error> {
    let expected: BTreeSet<&str> = allowed.iter().copied().collect();
    let actual: BTreeSet<&str> = map.keys().map(String::as_str).collect();
    if actual == expected {
        return Ok(());
    }
    for key in &actual {
        if !expected.contains(key) {
            return Err(de::Error::unknown_field(key, allowed));
        }
    }
    for key in &expected {
        if !actual.contains(key) {
            return Err(de::Error::missing_field(key));
        }
    }
    Ok(())
}

fn field<'de, D: serde::Deserializer<'de>, T: serde::de::DeserializeOwned>(
    map: &BTreeMap<String, Value>,
    name: &'static str,
) -> Result<T, D::Error> {
    map.get(name)
        .cloned()
        .ok_or_else(|| de::Error::missing_field(name))
        .and_then(|value| serde_json::from_value(value).map_err(de::Error::custom))
}

impl<'de> Deserialize<'de> for Trigger {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        let kind = map
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| de::Error::missing_field("kind"))?;
        match kind {
            "area_enter" => {
                exact_keys::<D>(&map, &["kind"])?;
                Ok(Self::AreaEnter)
            }
            "area_exit" => {
                exact_keys::<D>(&map, &["kind"])?;
                Ok(Self::AreaExit)
            }
            "interact" => {
                exact_keys::<D>(&map, &["kind"])?;
                Ok(Self::Interact)
            }
            "dialogue_node" => {
                exact_keys::<D>(&map, &["kind"])?;
                Ok(Self::DialogueNode)
            }
            "quest_state_change" => {
                exact_keys::<D>(&map, &["kind"])?;
                Ok(Self::QuestStateChange)
            }
            "timer" => {
                exact_keys::<D>(&map, &["kind", "ticks"])?;
                Ok(Self::Timer {
                    ticks: field::<D, u64>(&map, "ticks")?,
                })
            }
            "combat_start" => {
                exact_keys::<D>(&map, &["kind"])?;
                Ok(Self::CombatStart)
            }
            "combat_end" => {
                exact_keys::<D>(&map, &["kind"])?;
                Ok(Self::CombatEnd)
            }
            "death" => {
                exact_keys::<D>(&map, &["kind"])?;
                Ok(Self::Death)
            }
            "item_acquired" => {
                exact_keys::<D>(&map, &["kind"])?;
                Ok(Self::ItemAcquired)
            }
            "custom" => {
                exact_keys::<D>(&map, &["kind", "id"])?;
                Ok(Self::Custom {
                    id: field::<D, String>(&map, "id")?,
                })
            }
            other => Err(de::Error::unknown_variant(
                other,
                &[
                    "area_enter",
                    "area_exit",
                    "interact",
                    "dialogue_node",
                    "quest_state_change",
                    "timer",
                    "combat_start",
                    "combat_end",
                    "death",
                    "item_acquired",
                    "custom",
                ],
            )),
        }
    }
}

impl<'de> Deserialize<'de> for NodeBody {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        let kind = map
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| de::Error::missing_field("kind"))?;
        match kind {
            "condition" => {
                exact_keys::<D>(&map, &["kind", "expr"])?;
                Ok(Self::Condition {
                    expr: field::<D, String>(&map, "expr")?,
                })
            }
            "action" => {
                exact_keys::<D>(&map, &["kind", "call"])?;
                Ok(Self::Action {
                    call: field::<D, ActionCall>(&map, "call")?,
                })
            }
            "branch" => {
                exact_keys::<D>(&map, &["kind", "expr", "cases"])?;
                let cases = field::<D, Vec<DataValue>>(&map, "cases")?;
                let expr = field::<D, String>(&map, "expr")?;
                Ok(Self::Branch { expr, cases })
            }
            "sequence" => {
                exact_keys::<D>(&map, &["kind", "nodes"])?;
                Ok(Self::Sequence {
                    nodes: field::<D, Vec<Ulid>>(&map, "nodes")?,
                })
            }
            "wait" => {
                exact_keys::<D>(&map, &["kind", "ticks"])?;
                Ok(Self::Wait {
                    ticks: field::<D, u64>(&map, "ticks")?,
                })
            }
            "call_script" => {
                exact_keys::<D>(&map, &["kind", "script_id", "args"])?;
                let args = field::<D, BTreeMap<String, DataValue>>(&map, "args")?;
                let script_id = field::<D, String>(&map, "script_id")?;
                Ok(Self::CallScript { script_id, args })
            }
            "call_graph" => {
                exact_keys::<D>(&map, &["kind", "graph_id", "args"])?;
                let args = field::<D, BTreeMap<String, DataValue>>(&map, "args")?;
                let graph_id = field::<D, Ulid>(&map, "graph_id")?;
                Ok(Self::CallGraph { graph_id, args })
            }
            other => Err(de::Error::unknown_variant(
                other,
                &[
                    "condition",
                    "action",
                    "branch",
                    "sequence",
                    "wait",
                    "call_script",
                    "call_graph",
                ],
            )),
        }
    }
}

impl<'de> Deserialize<'de> for Port {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map = BTreeMap::<String, Value>::deserialize(deserializer)?;
        let kind = map
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| de::Error::missing_field("kind"))?;
        match kind {
            "next" => {
                exact_keys::<D>(&map, &["kind"])?;
                Ok(Self::Next)
            }
            "true" => {
                exact_keys::<D>(&map, &["kind"])?;
                Ok(Self::True)
            }
            "false" => {
                exact_keys::<D>(&map, &["kind"])?;
                Ok(Self::False)
            }
            "case" => {
                exact_keys::<D>(&map, &["kind", "index"])?;
                Ok(Self::Case {
                    index: field::<D, u32>(&map, "index")?,
                })
            }
            other => Err(de::Error::unknown_variant(
                other,
                &["next", "true", "false", "case"],
            )),
        }
    }
}
