use std::convert::TryFrom;
use std::fmt;

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use rocket::form::{FromFormField, ValueField};

#[derive(Clone, Debug, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RepositoryName {
    pub name: String,
}

use rocket::request::FromParam;

impl<'r> FromParam<'r> for RepositoryName {
    type Error = &'r str;

    fn from_param(param: &'r str) -> Result<Self, Self::Error> {
        Ok(Self {
            name: param.to_string(),
        })
    }
}

impl<'v> FromFormField<'v> for RepositoryName {
    fn from_value(field: ValueField<'v>) -> rocket::form::Result<'v, Self> {
        match field.value.parse() {
            Ok(value) => Ok(value),
            _ => std::result::Result::Err(
                rocket::form::Error::validation("Invalid repository name").into(),
            ),
        }
    }
}

impl FromStr for RepositoryName {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(RepositoryName {
            name: s.to_string(),
        })
    }
}

// We implement this so that serde_json can parse a RepositoryName from a straight string
impl TryFrom<String> for RepositoryName {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Ok(RepositoryName { name: value })
    }
}

// We implement this so that serde_json can serialize a RepositoryName struct into a string
impl From<RepositoryName> for String {
    fn from(name: RepositoryName) -> Self {
        name.name
    }
}

impl fmt::Display for RepositoryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl PartialEq for RepositoryName {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for RepositoryName {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str() {
        let name: RepositoryName = "a/b/c".parse().unwrap();
        assert_eq!(name.name, "a/b/c");
    }

    #[test]
    fn to_str() {
        let name: RepositoryName = "a/b/c".parse().unwrap();
        assert_eq!(name.to_string(), "a/b/c");
    }

    #[test]
    fn from_json() {
        let data = r#"
        "a/b/c"
        "#;
        let parsed: RepositoryName = serde_json::from_str(data).unwrap();
        let name: RepositoryName = "a/b/c".parse().unwrap();
        assert_eq!(parsed, name);
    }

    #[test]
    fn to_json() {
        let data = r#""a/b/c""#;
        let name: RepositoryName = "a/b/c".parse().unwrap();
        let serialized = serde_json::to_string(&name).unwrap();

        assert_eq!(data, serialized);
    }

    #[test]
    fn equality() {
        let name1: RepositoryName = "a/b/c".parse().unwrap();
        let name2: RepositoryName = "c/b/a".parse().unwrap();

        assert_eq!(name1, name1.clone());
        assert_ne!(name1, name2);
    }
}
