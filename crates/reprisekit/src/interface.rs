pub const ENV: &str = "REPRISE_INTERFACE";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interface {
    #[default]
    Classic,
    Blue,
    Glass,
}

impl Interface {
    pub fn parse(s: &str) -> Option<Interface> {
        match s {
            "classic" => Some(Interface::Classic),
            "blue" => Some(Interface::Blue),
            "glass" => Some(Interface::Glass),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Interface::Classic => "classic",
            Interface::Blue => "blue",
            Interface::Glass => "glass",
        }
    }

    pub fn from_env_value(v: Option<&str>) -> Interface {
        v.and_then(Interface::parse).unwrap_or_default()
    }

    pub fn from_process_env() -> Interface {
        Interface::from_env_value(std::env::var(ENV).ok().as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_interface_parses_and_round_trips() {
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            assert_eq!(Interface::parse(i.as_str()), Some(i));
        }
        assert_eq!(Interface::parse("blue"), Some(Interface::Blue));
        assert_eq!(Interface::default(), Interface::Classic);
    }

    #[test]
    fn unknown_or_missing_env_value_is_classic() {
        assert_eq!(Interface::parse("frosted"), None);
        assert_eq!(
            Interface::from_env_value(Some("frosted")),
            Interface::Classic
        );
        assert_eq!(Interface::from_env_value(None), Interface::Classic);
        assert_eq!(
            Interface::from_env_value(Some("classic")),
            Interface::Classic
        );
    }
}
