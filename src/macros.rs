#[macro_export]
macro_rules! tools {
    ($state:tt, $(($capitalized:tt, $lowercase:tt, $string:literal)),+) => {
        $(mod $lowercase;)+
        $(pub use $lowercase::$capitalized;)+

        #[derive(Debug, $crate::serde::Serialize, $crate::serde::Deserialize, $crate::clap::Subcommand)]
        #[serde(tag = "name")]
        pub enum Tools {
            $(
              #[serde(rename = $string)] $capitalized { #[clap(flatten)]  arguments: $capitalized },
            )+
        }

        impl $crate::traits::Tool<$state> for Tools {
            fn execute(self, state: &mut $state) -> $crate::anyhow::Result<String> {
                match self {
                    $(Tools::$capitalized { arguments} => arguments.execute(state),)+
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
                    $(Tools::$capitalized { .. } => $string,)+
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
