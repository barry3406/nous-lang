use std::collections::{HashMap, HashSet};

use nous_ast::types::TypeExpr;
use serde::{Deserialize, Serialize};

/// Built-in primitive type names understood by the Nous type system.
pub const BUILTIN_TYPES: &[&str] = &[
    "Nat", "Int", "Dec", "Bool", "Text", "Bytes", "Time", "Duration", "Void",
    // Generic containers — the bare name is registered; type arguments are
    // checked separately when a `TypeExpr::Generic` is resolved.
    "List", "Map", "Set", "Option", "Result",
];

// ---------------------------------------------------------------------------
// Entity definition stored in the environment
// ---------------------------------------------------------------------------

/// A field entry inside a stored entity definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    pub ty: TypeExpr,
}

/// The information the type checker retains about an `entity` declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
}

// ---------------------------------------------------------------------------
// State machine definition stored in the environment
// ---------------------------------------------------------------------------

/// A single transition edge in a state machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionDef {
    pub from: String,
    pub action: String,
    pub to: String,
}

/// The information the type checker retains about a `state` declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDef {
    pub name: String,
    /// All states that appear as either `from` or `to` in any transition.
    pub states: HashSet<String>,
    pub transitions: Vec<TransitionDef>,
}

// ---------------------------------------------------------------------------
// Function signature stored in the environment
// ---------------------------------------------------------------------------

/// A parameter entry inside a stored function signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamDef {
    pub name: String,
    pub ty: TypeExpr,
}

/// The information the type checker retains about a `fn` or `flow` declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FnSig {
    pub name: String,
    pub params: Vec<ParamDef>,
    pub return_type: TypeExpr,
    /// Effects declared in the contract.
    pub effects: Vec<String>,
}

// ---------------------------------------------------------------------------
// TypeEnv
// ---------------------------------------------------------------------------

/// The type environment threaded through the type-checking pass.
///
/// It records:
/// - Which type *names* are in scope (builtins + user-defined).
/// - Entity definitions (fields, invariants).
/// - State machine definitions (states, transitions).
/// - Function signatures (params, return type, effects).
#[derive(Debug, Clone)]
pub struct TypeEnv {
    /// All type names that are currently valid (builtins + declared types).
    known_types: HashSet<String>,

    /// Entity definitions keyed by entity name.
    entities: HashMap<String, EntityDef>,

    /// State machine definitions keyed by machine name.
    state_machines: HashMap<String, StateDef>,

    /// Function (and flow) signatures keyed by function name.
    functions: HashMap<String, FnSig>,
}

impl Default for TypeEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeEnv {
    /// Construct a fresh environment pre-populated with built-in types.
    pub fn new() -> Self {
        let mut known_types = HashSet::new();
        for &name in BUILTIN_TYPES {
            known_types.insert(name.to_owned());
        }
        Self {
            known_types,
            entities: HashMap::new(),
            state_machines: HashMap::new(),
            functions: HashMap::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Type name registration & lookup
    // -----------------------------------------------------------------------

    /// Register a new user-defined type name (entity, enum, type alias, …).
    pub fn register_type_name(&mut self, name: impl Into<String>) {
        self.known_types.insert(name.into());
    }

    /// Return `true` if `name` refers to a known type (builtin or declared).
    pub fn lookup_type(&self, name: &str) -> bool {
        self.known_types.contains(name)
    }

    // -----------------------------------------------------------------------
    // Entity definitions
    // -----------------------------------------------------------------------

    /// Record an entity definition.  Also registers the entity name as a
    /// known type so that fields can reference it.
    pub fn define_entity(&mut self, def: EntityDef) {
        self.known_types.insert(def.name.clone());
        self.entities.insert(def.name.clone(), def);
    }

    /// Retrieve an entity definition by name.
    pub fn get_entity(&self, name: &str) -> Option<&EntityDef> {
        self.entities.get(name)
    }

    /// Iterate over all registered entity definitions.
    pub fn entities(&self) -> impl Iterator<Item = &EntityDef> {
        self.entities.values()
    }

    // -----------------------------------------------------------------------
    // State machine definitions
    // -----------------------------------------------------------------------

    /// Record a state machine definition.
    pub fn define_state(&mut self, def: StateDef) {
        self.state_machines.insert(def.name.clone(), def);
    }

    /// Retrieve a state machine definition by name.
    pub fn get_state_machine(&self, name: &str) -> Option<&StateDef> {
        self.state_machines.get(name)
    }

    /// Iterate over all registered state machine definitions.
    pub fn state_machines(&self) -> impl Iterator<Item = &StateDef> {
        self.state_machines.values()
    }

    // -----------------------------------------------------------------------
    // Function signatures
    // -----------------------------------------------------------------------

    /// Record a function (or flow) signature.
    pub fn define_fn(&mut self, sig: FnSig) {
        self.functions.insert(sig.name.clone(), sig);
    }

    /// Retrieve a function signature by name.
    pub fn lookup_fn(&self, name: &str) -> Option<&FnSig> {
        self.functions.get(name)
    }

    /// Iterate over all registered function signatures.
    pub fn functions(&self) -> impl Iterator<Item = &FnSig> {
        self.functions.values()
    }
}
