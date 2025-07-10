#[macro_export]
macro_rules! tools {
    ($state:tt, $(($capitalized:tt, $lowercase:tt, $string:literal)),+) => {
        $(mod $lowercase;)+
        $(pub use $lowercase::$capitalized;)+

        #[derive(Debug, $crate::clap::Subcommand)]
        pub enum Tools {
            $(
                $capitalized(#[clap(flatten)] $capitalized),
            )+
        }

        // Simple Deserialize implementation using serde_json::Value
        impl<'de> $crate::serde::Deserialize<'de> for Tools {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
            D: $crate::serde::Deserializer<'de>,
            {
                use $crate::serde::de;

                let value: $crate::serde_json::Value = $crate::serde::Deserialize::deserialize(deserializer)?;

                let obj = value.as_object()
                .ok_or_else(|| de::Error::custom("expected object"))?;

                let name = obj.get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| de::Error::missing_field("name"))?;

                let arguments = obj.get("arguments")
                .ok_or_else(|| de::Error::missing_field("arguments"))?;

                match name {
                    $(
                        $string => $crate::serde_json::from_value(arguments.clone())
                                       .map_err(de::Error::custom)
                                       .map(Tools::$capitalized),
                    )+
                    _ => Err(de::Error::unknown_variant(name, &[$($string),+])),
                }
            }
        }

        // Manual Serialize implementation to maintain the same format
        impl $crate::serde::Serialize for Tools {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
            S: $crate::serde::Serializer,
            {
                use $crate::serde::ser::SerializeStruct;

                let mut state = serializer.serialize_struct("Tools", 2)?;
                match self {
                    $(
                        Tools::$capitalized(args) => {
                            state.serialize_field("name", $string)?;
                            state.serialize_field("arguments", args)?;
                        }
                    )+
                }
                state.end()
            }
        }


        impl $crate::traits::Tool<$state> for Tools {
            fn execute(self, state: &mut $state) -> $crate::anyhow::Result<String> {
                match self {
                    $(Tools::$capitalized(tool) => tool.execute(state),)+
                }
            }

        }

        impl $crate::traits::AsToolsList for Tools {
            fn tools_list() -> Vec<$crate::types::ToolSchema> {
                use $crate::traits::AsToolSchema;
                vec![$($capitalized::schema(),)+]
            }
        }

        impl Tools {
            #[allow(dead_code)]
            pub fn name(&self) -> &str {
                match self {
                    $(Tools::$capitalized(_) => $string,)+
                }
            }
        }
    };
}

#[macro_export]
macro_rules! server_info {
    () => {
        $crate::types::Info {
            name: env!("CARGO_PKG_NAME").into(),
            version: env!("CARGO_PKG_VERSION").into(),
        }
    };
}
