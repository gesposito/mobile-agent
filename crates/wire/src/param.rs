//! How a request's parameters are read: as the daemon has always read them
//! by name. A parameter given null is one not given, a string given another
//! type is no string, and a flag is set by `true` alone; a number, a point
//! or a list of strings given a value of another type keeps that value
//! ([`Param`]), which the daemon refuses when a command reads the parameter.
//! The deserializers here are the fields' `deserialize_with`, and the
//! defaults and their tests the fields' `default` and `skip_serializing_if`.

use serde::de::{DeserializeOwned, Deserializer, Error as _};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

/// A number, a point or a list of strings as a request gives it: a value of
/// the parameter's type, or the JSON value given in its place.
///
/// The daemon reads a parameter only where a command uses it and refuses a
/// value of another type only then, naming the parameter and the value:
/// `wait` reads `quiet` only to settle, and `swipe` its `distance` only
/// without `from` and `to`. So a request keeps such a value as it came
/// instead of refusing the line, and serializes it back unchanged.
#[derive(Clone, Debug, PartialEq)]
pub enum Param<T> {
    /// A value of the parameter's type.
    Valid(T),
    /// The value the request gave in its place.
    Invalid(Value),
}

impl<T> Param<T> {
    /// The value, or the JSON value given in its place.
    pub fn get(&self) -> Result<&T, &Value> {
        match self {
            Param::Valid(value) => Ok(value),
            Param::Invalid(given) => Err(given),
        }
    }
}

impl<T> Param<Option<T>> {
    /// Whether the parameter was not given.
    pub fn is_none(&self) -> bool {
        matches!(self, Param::Valid(None))
    }
}

impl<T> Param<Vec<T>> {
    /// Whether the parameter holds an empty list, as one not given does.
    pub fn is_empty(&self) -> bool {
        matches!(self, Param::Valid(list) if list.is_empty())
    }
}

impl<T: Default> Default for Param<T> {
    fn default() -> Self {
        Param::Valid(T::default())
    }
}

impl<T> From<T> for Param<T> {
    fn from(value: T) -> Self {
        Param::Valid(value)
    }
}

impl<T: Serialize> Serialize for Param<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Param::Valid(value) => value.serialize(serializer),
            Param::Invalid(given) => given.serialize(serializer),
        }
    }
}

/// A value of `T`, null included where `T` takes it (`Option`), or the
/// value as it came.
impl<'de, T: DeserializeOwned> Deserialize<'de> for Param<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(kept(Value::deserialize(deserializer)?))
    }
}

fn kept<T: DeserializeOwned>(given: Value) -> Param<T> {
    match T::deserialize(&given) {
        Ok(value) => Param::Valid(value),
        Err(_) => Param::Invalid(given),
    }
}

/// A parameter's schema is its type's: a request is written with values of
/// the parameters' types.
#[cfg(feature = "schema")]
impl<T: schemars::JsonSchema> schemars::JsonSchema for Param<T> {
    fn inline_schema() -> bool {
        T::inline_schema()
    }

    fn schema_name() -> std::borrow::Cow<'static, str> {
        T::schema_name()
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        T::schema_id()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        T::json_schema(generator)
    }
}

/// A flag: set by `true`, any other value leaving it unset.
pub(crate) fn flag<'de, D: Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    Ok(Value::deserialize(deserializer)? == Value::Bool(true))
}

/// A string; none for a value of another type.
pub(crate) fn text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::String(text) => Some(text),
        _ => None,
    })
}

/// A string a command needs: empty for a value of another type, which the
/// daemon refuses as one not given.
pub(crate) fn needed<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(text(deserializer)?.unwrap_or_default())
}

/// The session: a string that is not empty, the default session for null.
pub(crate) fn session<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    match Value::deserialize(deserializer)? {
        Value::String(session) if !session.is_empty() => Ok(session),
        Value::Null => Ok(default_session()),
        _ => Err(D::Error::custom("\"session\" must be a non-empty string")),
    }
}

/// The session a request names none for.
pub(crate) fn default_session() -> String {
    "default".to_owned()
}

/// A list of strings; empty for null.
pub(crate) fn list<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Param<Vec<String>>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Null => Param::Valid(Vec::new()),
        given => kept(given),
    })
}

/// A whole number whose default is `N`, which null gives.
pub(crate) fn whole_or<'de, D: Deserializer<'de>, const N: u64>(
    deserializer: D,
) -> Result<Param<u64>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Null => whole::<N>(),
        given => kept(given),
    })
}

/// `N`, a whole number parameter's default.
pub(crate) fn whole<const N: u64>() -> Param<u64> {
    Param::Valid(N)
}

/// Whether a whole number parameter holds its default `N`.
pub(crate) fn is_whole<const N: u64>(param: &Param<u64>) -> bool {
    *param == whole::<N>()
}

/// A distance in points whose default is `N` points, which null gives.
pub(crate) fn points_or<'de, D: Deserializer<'de>, const N: u16>(
    deserializer: D,
) -> Result<Param<f64>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Null => points::<N>(),
        given => kept(given),
    })
}

/// `N` points, a distance's default.
pub(crate) fn points<const N: u16>() -> Param<f64> {
    Param::Valid(f64::from(N))
}

/// Whether a distance holds its default of `N` points.
pub(crate) fn is_points<const N: u16>(param: &Param<f64>) -> bool {
    *param == points::<N>()
}

/// A direction or state whose default is `default`, which a value of
/// another type gives.
fn text_or<'de, D: Deserializer<'de>>(
    deserializer: D,
    default: fn() -> String,
) -> Result<String, D::Error> {
    Ok(text(deserializer)?.unwrap_or_else(default))
}

/// `up`, a swipe's default direction.
pub(crate) fn up() -> String {
    "up".to_owned()
}

pub(crate) fn up_or<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    text_or(deserializer, up)
}

pub(crate) fn is_up(direction: &str) -> bool {
    direction == "up"
}

/// `down`, a scroll's default direction.
pub(crate) fn down() -> String {
    "down".to_owned()
}

pub(crate) fn down_or<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    text_or(deserializer, down)
}

pub(crate) fn is_down(direction: &str) -> bool {
    direction == "down"
}

/// `visible`, the state an element wait waits for by default.
pub(crate) fn visible() -> String {
    "visible".to_owned()
}

pub(crate) fn visible_or<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    text_or(deserializer, visible)
}

pub(crate) fn is_visible(state: &str) -> bool {
    state == "visible"
}

/// Joins the lines a doc comment wraps, as Markdown reads them, in a
/// schema's description and those of its properties: schemars keeps every
/// line break of the comments it takes them from.
#[cfg(feature = "schema")]
pub(crate) fn soft_wraps(schema: &mut schemars::Schema) {
    fn join(described: &mut serde_json::Map<String, Value>) {
        if let Some(Value::String(text)) = described.get_mut("description") {
            *text = text
                .split("\n\n")
                .map(|paragraph| {
                    paragraph
                        .lines()
                        .map(str::trim)
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
                .join("\n\n");
        }
    }
    let Some(root) = schema.as_object_mut() else {
        return;
    };
    join(root);
    if let Some(Value::Object(properties)) = root.get_mut("properties") {
        for property in properties.values_mut() {
            if let Value::Object(property) = property {
                join(property);
            }
        }
    }
}
